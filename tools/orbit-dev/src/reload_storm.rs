//! Concurrent target-session subscriptions using the actual Unix socket protocol.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    socket: PathBuf,
    #[arg(short = 'n', default_value_t = 40)]
    clients: usize,
    #[arg(long)]
    home: Option<PathBuf>,
    #[arg(long, default_value_t = 0)]
    chunk: usize,
}
fn sessions(home: &Path, n: usize) -> Result<Vec<String>> {
    let mut files = fs::read_dir(home.join("sessions"))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".json")?;
            if !id.starts_with("session_")
                || id.chars().filter(|c| *c == '_').count() < 3
                || id.ends_with(".agent-models")
                || id.ends_with(".meta")
            {
                return None;
            }
            Some((
                e.metadata()
                    .ok()?
                    .modified()
                    .unwrap_or(SystemTime::UNIX_EPOCH),
                id.to_owned(),
            ))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(files.into_iter().take(n).map(|(_, s)| s).collect())
}
#[cfg(unix)]
mod unix {
    use super::*;
    use serde_json::{Value, json};
    use std::time::{Duration, Instant};
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
    };
    async fn one(
        socket: &Path,
        session: &str,
        cwd: &Path,
        chunk: usize,
        limit: Duration,
    ) -> Result<(f64, usize)> {
        let start = Instant::now();
        tokio::time::timeout(limit,async {
            let mut stream=UnixStream::connect(socket).await?;
            stream.write_all(format!("{}\n",json!({"type":"subscribe","id":1,"working_dir":cwd,"target_session_id":session,"client_instance_id":format!("bench_{session}")})).as_bytes()).await?;
            let mut reader=BufReader::with_capacity(if chunk==0{8192}else{chunk},stream);
            let mut line=Vec::new();
            loop {
                line.clear();
                loop {
                    let bytes=reader.fill_buf().await?;ensure!(!bytes.is_empty(),"eof");
                    let newline=bytes.iter().position(|b|*b==b'\n');let count=newline.map(|n|n+1).unwrap_or(bytes.len());line.extend_from_slice(&bytes[..count]);reader.consume(count);
                    ensure!(line.len()<=1<<30,"protocol line exceeded 1 GiB");
                    if newline.is_some(){break;}
                    if chunk>0 {tokio::task::yield_now().await;}
                }
                let event:Value=serde_json::from_slice(&line).context("invalid protocol event")?;
                match event["type"].as_str() {Some("history")=>return Ok((start.elapsed().as_secs_f64(),line.len())),Some("error")=>anyhow::bail!("{}",String::from_utf8_lossy(&line[..line.len().min(200)])),_=>{}}
            }
        }).await.context("timeout")?
    }
    pub(super) fn run(args: Args, ids: Vec<String>) -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let start = Instant::now();
            let cwd = std::env::current_dir()?;
            let mut tasks = tokio::task::JoinSet::new();
            for id in ids {
                let socket = args.socket.clone();
                let cwd = cwd.clone();
                tasks.spawn(async move {
                    let result = one(&socket, &id, &cwd, args.chunk, Duration::from_secs(60)).await;
                    (id, result)
                });
            }
            let mut ok = Vec::new();
            let mut bad = Vec::new();
            while let Some(result) = tasks.join_next().await {
                let (id, result) = result?;
                match result {
                    Ok((time, _)) => ok.push(time),
                    Err(e) => bad.push((id, e)),
                }
            }
            ok.sort_by(f64::total_cmp);
            if !ok.is_empty() {
                let q = |p: f64| ok[((ok.len() as f64 * p) as usize).min(ok.len() - 1)] * 1000.0;
                println!(
                    "history n={} p50={:.0}ms p90={:.0}ms max={:.0}ms wall={:.0}ms",
                    ok.len(),
                    q(0.5),
                    q(0.9),
                    ok[ok.len() - 1] * 1000.0,
                    start.elapsed().as_secs_f64() * 1000.0
                );
            }
            for (id, e) in bad.iter().take(5) {
                println!("FAIL {id} {e:#}");
            }
            ensure!(bad.is_empty(), "{} subscription(s) failed", bad.len());
            Ok(())
        })
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn chunked_history_errors_and_timeout() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                for (reply, success) in [
                    (
                        "{\"type\":\"done\"}\n{\"type\": \"history\",\"messages\":[]}\n",
                        true,
                    ),
                    ("{\"type\":\"error\",\"message\":\"bad session\"}\n", false),
                    ("", false),
                ] {
                    let dir = tempfile::tempdir().unwrap();
                    let socket = dir.path().join("sock");
                    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
                    let task = tokio::spawn(async move {
                        let (stream, _) = listener.accept().await.unwrap();
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        reader.read_line(&mut line).await.unwrap();
                        let v: Value = serde_json::from_str(&line).unwrap();
                        assert_eq!(v["target_session_id"], "session_a_b_c");
                        assert_eq!(v["client_instance_id"], "bench_session_a_b_c");
                        if reply.is_empty() {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        } else {
                            reader.get_mut().write_all(reply.as_bytes()).await.unwrap();
                        }
                    });
                    assert_eq!(
                        one(
                            &socket,
                            "session_a_b_c",
                            Path::new("/work"),
                            3,
                            Duration::from_millis(50)
                        )
                        .await
                        .is_ok(),
                        success
                    );
                    task.abort();
                    let _ = task.await;
                }
            });
        }
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("bench-reload-storm".into()).chain(args));
    ensure!(
        args.clients > 0 && args.chunk <= 1 << 30,
        "invalid client count or chunk size"
    );
    let home = args
        .home
        .clone()
        .or_else(|| dirs::home_dir().map(|p| p.join(".nexode")))
        .context("home unavailable")?;
    let ids = sessions(&home, args.clients)?;
    ensure!(!ids.is_empty(), "no matching sessions");
    #[cfg(unix)]
    {
        unix::run(args, ids)
    }
    #[cfg(not(unix))]
    {
        let _ = (args, ids);
        anyhow::bail!("reload benchmark requires Unix sockets")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_excludes_sidecars_and_invalid_session_ids() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("sessions")).unwrap();
        for n in [
            "session_a_b_c.json",
            "session_a_b_c.meta.json",
            "session_a_b_c.agent-models.json",
            "session_bad.json",
            "other_a_b_c.json",
        ] {
            fs::write(d.path().join("sessions").join(n), "{}").unwrap();
        }
        assert_eq!(sessions(d.path(), 40).unwrap(), vec!["session_a_b_c"]);
    }
}
