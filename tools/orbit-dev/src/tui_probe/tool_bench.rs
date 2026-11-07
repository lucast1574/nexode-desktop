//! Tool timings through native debug JSON, scoped to an owned session.
use super::{
    debug_fixture::{Fixture, OwnedSession},
    io,
};
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Parser)]
struct Args {
    #[arg(default_value_t = 5)]
    iterations: usize,
    #[arg(long)]
    fresh: bool,
    #[arg(long, default_value = "./target/release/nexode")]
    binary: PathBuf,
    #[arg(long, required_unless_present = "fresh", conflicts_with = "fresh")]
    socket: Option<PathBuf>,
    #[arg(long, default_value = ".")]
    repo: PathBuf,
    #[arg(long)]
    csv: PathBuf,
    /// Include Jev recall against the supplied configured daemon.
    #[arg(long, conflicts_with = "fresh")]
    live: bool,
}
struct Case {
    label: String,
    name: &'static str,
    input: Value,
    reset: bool,
    rejected: bool,
}
fn cases(repo: &std::path::Path, work: &std::path::Path, live: bool) -> Vec<Case> {
    let mut cases = vec![];
    let mut add = |label: &str, name, input| {
        cases.push(Case {
            label: label.into(),
            name,
            input,
            reset: false,
            rejected: false,
        })
    };
    for (label, file, input) in [
        ("read (tiny file)", work.join("test.txt"), json!({})),
        ("read (1000 lines)", work.join("large.txt"), json!({})),
        (
            "read (10 lines @ offset)",
            work.join("large.txt"),
            json!({"offset":500,"limit":10}),
        ),
        ("read (main.rs)", repo.join("src/main.rs"), json!({})),
    ] {
        let mut input = input;
        input["file_path"] = json!(file);
        add(label, "read", input);
    }
    add(
        "write (small)",
        "write",
        json!({"file_path":work.join("write_test.txt"),"content":"hello world"}),
    );
    add(
        "write (10KB)",
        "write",
        json!({"file_path":work.join("write_test.txt"),"content":"x".repeat(10000)}),
    );
    add(
        "edit (simple replace)",
        "edit",
        json!({"file_path":work.join("edit_test.txt"),"old_string":"quick brown","new_string":"slow red"}),
    );
    for (label, query, path) in [
        ("agentgrep (fn main in src/)", "fn main", "src"),
        (
            "agentgrep (async fn in tools)",
            "async fn",
            "crates/nexode-app-core/src/tool",
        ),
        ("agentgrep (tokio::spawn)", "tokio::spawn", "src"),
    ] {
        add(
            label,
            "agentgrep",
            json!({"mode":"grep","query":query,"path":repo.join(path),"type":"rs"}),
        );
    }
    for (label, path) in [
        ("agentgrep find (**/*.rs)", "."),
        (
            "agentgrep find (tools/*.rs)",
            "crates/nexode-app-core/src/tool",
        ),
    ] {
        add(
            label,
            "agentgrep",
            json!({"mode":"find","glob":"**/*.rs","path":repo.join(path)}),
        );
    }
    for (label, command) in [
        ("bash ls (repo root)", "ls"),
        ("bash ls (src/)", "ls src"),
        ("bash ls (tools/)", "ls crates/nexode-app-core/src/tool"),
        ("bash (echo)", "echo hello"),
        ("bash (true)", "true"),
        ("bash (ls -la)", "ls -la crates/nexode-app-core/src/tool"),
        ("bash (wc -l)", "wc -l src/main.rs"),
        ("bash (cat /dev/null)", "cat /dev/null"),
        ("bash (git log -5)", "git log --oneline -5"),
        ("bash (cargo --version)", "cargo --version"),
    ] {
        add(label, "bash", json!({"command":command}));
    }
    add("todo (read)", "todo", json!({}));
    add(
        "todo (write)",
        "todo",
        json!({"todos":[{"name":"benchmark test","status":"pending","priority":"low","confidence":"verified"}]}),
    );
    add(
        "session_search",
        "session_search",
        json!({"query":"benchmark test","source":"nexode","include_external":false,"working_dir":repo}),
    );
    if live {
        add(
            "memory (semantic recall)",
            "memory",
            json!({"action":"recall","query":"benchmark test","limit":3}),
        );
    }
    add(
        "memory (list)",
        "memory",
        json!({"action":"list","limit":5}),
    );
    for c in &mut cases {
        c.reset = c.name == "edit";
    }
    cases.push(Case {
        label: "invalid (dispatch rejection)".into(),
        name: "orbit_nonexistent_tool",
        input: json!({}),
        reset: false,
        rejected: true,
    });
    cases
}
fn csv_cell(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
pub(super) fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::try_parse_from(std::iter::once("benchmark-tools".into()).chain(args))?;
    ensure!(
        (1..=1000).contains(&a.iterations),
        "iterations must be 1..1000"
    );
    let repo = a.repo.canonicalize()?;
    ensure!(
        repo.join("src/main.rs").is_file(),
        "repo must be Nexode checkout"
    );
    let fixture = if a.fresh {
        Some(Fixture::new(&a.binary.canonicalize()?)?)
    } else {
        None
    };
    let work = tempfile::tempdir()?;
    fs::write(work.path().join("test.txt"), "hello world\n")?;
    fs::write(work.path().join("large.txt"),(0..1000).map(|i|format!("Line {i}: This is a test line with some content for benchmarking purposes. The quick brown fox jumps over the lazy dog.\n")).collect::<String>())?;
    let socket = fixture
        .as_ref()
        .map(|f| &f.socket)
        .or(a.socket.as_ref())
        .context("socket required")?;
    let session = OwnedSession::new(socket, &repo)?;
    let mut csv = String::from("tool,iteration,time_ms,input_size_bytes,output_size_bytes\n");
    let mut summaries = vec![];
    for c in cases(&repo, work.path(), a.live) {
        let input = c.input.to_string();
        let mut times = vec![];
        for i in 1..=a.iterations {
            if c.reset {
                fs::write(
                    work.path().join("edit_test.txt"),
                    "The quick brown fox jumps over the lazy dog\n",
                )?;
            }
            let start = Instant::now();
            let response = io::debug_response(
                socket,
                &format!("tool:{} {input}", c.name),
                Some(&session.id),
                Duration::from_secs(120),
            )?;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            let rejected = response["ok"] == false || response["type"] == "error";
            ensure!(
                rejected == c.rejected,
                "{} returned unexpected rejection state",
                c.label
            );
            let output = response["output"].as_str().unwrap_or("");
            if !c.rejected {
                let value: Value = serde_json::from_str(output).context("invalid tool output")?;
                ensure!(
                    value["metadata"]["exit_code"].as_i64().unwrap_or(0) == 0,
                    "{} shell exited unsuccessfully",
                    c.label
                );
            }
            csv.push_str(&format!(
                "{},{i},{elapsed:.3},{},{}\n",
                csv_cell(&c.label),
                input.len(),
                output.len()
            ));
            times.push(elapsed);
        }
        times.sort_by(f64::total_cmp);
        let avg = times.iter().sum::<f64>() / times.len() as f64;
        summaries.push(json!({"tool":c.label,"avg_ms":avg,"p50_ms":times[times.len()/2],"min_ms":times[0],"max_ms":times[times.len()-1]}));
    }
    summaries.sort_by(|a, b| {
        b["avg_ms"]
            .as_f64()
            .unwrap()
            .total_cmp(&a["avg_ms"].as_f64().unwrap())
    });
    fs::write(&a.csv, csv)?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"csv":a.csv,"iterations":a.iterations,"summary":summaries,"semantic_recall":if a.live{"included"}else{"skipped (requires --live and Jev credentials)"}})
        )?
    );
    Ok(0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cases_reset_edits_and_use_current_tool_names() {
        let cases = cases(
            std::path::Path::new("/repo"),
            std::path::Path::new("/tmp/work"),
            false,
        );
        assert_eq!(cases.iter().filter(|c| c.reset).count(), 1);
        assert!(cases.iter().any(|c| c.name == "todo"));
        assert!(!cases.iter().any(|c| c.name == "todowrite"));
        assert_eq!(csv_cell("a,\"b"), "\"a,\"\"b\"");
    }
}
