//! Native single-container Terminal-Bench runner and resumable campaigns.
mod campaign;
mod dataset;
mod docker;
mod engine;
mod runtime;
mod task;
use anyhow::{Context, Result, ensure};
pub use campaign::run as campaign;
use clap::Parser;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Parser, Clone)]
pub(super) struct Args {
    #[arg(short = 'p', long, conflicts_with = "dataset")]
    pub path: Option<PathBuf>,
    /// Installed dataset directory name (or terminal-bench@2.0 / @2.1).
    #[arg(short = 'd', long)]
    pub dataset: Option<String>,
    #[arg(short = 't', long = "include-task-name", alias = "task")]
    pub tasks: Vec<String>,
    #[arg(short = 'm', long)]
    pub model: Option<String>,
    #[arg(long)]
    pub claude: bool,
    #[arg(short = 'k', long, default_value_t = 1)]
    pub n_attempts: usize,
    #[arg(long, default_value_t = 1)]
    pub n_concurrent: usize,
    #[arg(long)]
    pub n_tasks: Option<usize>,
    #[arg(long, default_value = "jobs")]
    pub jobs_dir: PathBuf,
    #[arg(long)]
    pub job_name: Option<String>,
    #[arg(long, default_value_t = 1.)]
    pub timeout_multiplier: f64,
    #[arg(long)]
    pub binary: Option<PathBuf>,
    #[arg(long, default_value = "docker")]
    pub engine: String,
    #[arg(long)]
    pub yes: bool,
    /// Validate tasks and print a plan without starting containers or inference.
    #[arg(long)]
    pub dry_run: bool,
}
pub(super) fn expand(p: impl AsRef<Path>) -> PathBuf {
    let p = p.as_ref();
    if let Ok(rest) = p.strip_prefix("~") {
        dirs::home_dir().unwrap_or_default().join(rest)
    } else {
        p.to_owned()
    }
}
pub(super) fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.into())
}
pub(super) fn safe_name(s: &str) -> Result<()> {
    ensure!(
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "invalid task/job name"
    );
    Ok(())
}
pub(super) fn write_json(p: &Path, v: &Value) -> Result<()> {
    let mut f = tempfile::NamedTempFile::new_in(p.parent().context("output parent")?)?;
    use std::io::Write;
    writeln!(f, "{}", serde_json::to_string_pretty(v)?)?;
    f.persist(p).map_err(|e| e.error)?;
    Ok(())
}
impl Args {
    pub fn model(&self) -> String {
        self.model.clone().unwrap_or_else(|| {
            env(
                "NEXODE_TB_MODEL",
                if self.claude {
                    "anthropic-api/claude-opus-4-8"
                } else {
                    "openai/gpt-5.4"
                },
            )
        })
    }
    pub fn binary(&self) -> PathBuf {
        if let Some(p) = &self.binary {
            return expand(p);
        }
        if let Some(p) = std::env::var_os("NEXODE_HARBOR_BINARY") {
            return expand(PathBuf::from(p));
        }
        let dir = expand(env("NEXODE_HARBOR_BINARY_DIR", "/tmp/nexode-compat-dist"));
        let legacy = dir.join("nexode-linux-x86_64.bin");
        if self.claude && legacy.is_file() {
            legacy
        } else {
            dir.join("nexode-linux-x86_64")
        }
    }
    pub fn ensure_binary(&self) -> Result<()> {
        if self.binary().is_file() {
            return Ok(());
        }
        ensure!(
            self.binary.is_none() && std::env::var_os("NEXODE_HARBOR_BINARY").is_none(),
            "explicit Nexode binary does not exist"
        );
        let root = crate::root()?;
        let status = std::process::Command::new("bash")
            .arg(root.join("scripts/build_linux_compat.sh"))
            .arg(expand(env(
                "NEXODE_HARBOR_BINARY_DIR",
                "/tmp/nexode-compat-dist",
            )))
            .current_dir(root)
            .status()?;
        ensure!(
            status.success() && self.binary().is_file(),
            "compatibility build did not produce the benchmark binary"
        );
        Ok(())
    }
    pub fn source(&self) -> Result<PathBuf> {
        let p = if let Some(p) = &self.path {
            p.clone()
        } else if let Some(d) = &self.dataset {
            let root = PathBuf::from(env("NEXODE_TB_DATASETS_DIR", "/tmp"));
            let name = match d.as_str() {
                "terminal-bench@2.0" => "terminal-bench-2",
                "terminal-bench@2.1" => "terminal-bench-2.1",
                _ => return dataset::resolve(d),
            };
            root.join(name)
        } else {
            PathBuf::from(env(
                "NEXODE_TB_PATH",
                if self.claude {
                    "/tmp/terminal-bench-2.1"
                } else {
                    "/tmp/terminal-bench-2"
                },
            ))
        };
        let p = expand(p);
        if !p.exists()
            && let Some(d) = &self.dataset
        {
            return dataset::resolve(d);
        }
        p.canonicalize().context(
            "dataset must be installed locally; pass --path to a Harbor-format task/dataset",
        )
    }
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    execute(Args::try_parse_from(
        std::iter::once("terminal-bench".into()).chain(args),
    )?)
}
pub(super) fn execute(a: Args) -> Result<i32> {
    ensure!(
        a.n_attempts > 0 && a.n_concurrent > 0 && a.n_concurrent <= 128,
        "attempts and concurrency must be positive (concurrency <=128)"
    );
    ensure!(
        a.timeout_multiplier.is_finite() && a.timeout_multiplier > 0.,
        "invalid timeout multiplier"
    );
    let source = a.source()?;
    let mut paths = if source.join("task.toml").is_file() {
        vec![source]
    } else {
        fs::read_dir(&source)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.join("task.toml").is_file())
            .collect::<Vec<_>>()
    };
    paths.sort();
    for name in &a.tasks {
        safe_name(name)?;
        ensure!(
            paths
                .iter()
                .any(|p| p.file_name().is_some_and(|n| n == name.as_str())),
            "selected task not found: {name}"
        );
    }
    if !a.tasks.is_empty() {
        paths.retain(|p| {
            a.tasks
                .iter()
                .any(|n| p.file_name().is_some_and(|s| s == n.as_str()))
        });
    }
    if let Some(n) = a.n_tasks {
        paths.truncate(n);
    }
    ensure!(!paths.is_empty(), "no matching tasks");
    let tasks = paths
        .iter()
        .map(|p| task::Task::load(p))
        .collect::<Result<Vec<_>>>()?;
    if a.dry_run {
        println!(
            "{}",
            json!({"tasks":paths,"attempts":a.n_attempts,"model":a.model(),"runner":"native-docker","inference":false})
        );
        return Ok(0);
    }
    a.ensure_binary()?;
    let job_name = a
        .job_name
        .clone()
        .unwrap_or_else(|| chrono::Utc::now().format("%Y%m%d-%H%M%S-%f").to_string());
    safe_name(&job_name)?;
    fs::create_dir_all(&a.jobs_dir)?;
    let job = a.jobs_dir.canonicalize()?.join(job_name);
    fs::create_dir(&job).context("job already exists; choose a new --job-name")?;
    let started = chrono::Utc::now().to_rfc3339();
    let mut jobs = vec![];
    for task in tasks {
        for attempt in 1..=a.n_attempts {
            jobs.push((task.clone(), attempt));
        }
    }
    let mut results = vec![];
    for batch in jobs.chunks(a.n_concurrent) {
        let values = std::thread::scope(|s| {
            let handles = batch
                .iter()
                .map(|(t, n)| {
                    let a = &a;
                    let job = &job;
                    s.spawn(move || docker::trial(a, t, *n, job))
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|h| {
                    h.join()
                        .map_err(|_| anyhow::anyhow!("trial worker panicked"))?
                })
                .collect::<Result<Vec<_>>>()
        })?;
        results.extend(values);
    }
    let failed = results.iter().any(|v| !v["exception_info"].is_null());
    write_json(
        &job.join("result.json"),
        &json!({"runner":"orbit-dev-native-docker","n_total_trials":results.len(),"started_at":started,"finished_at":chrono::Utc::now().to_rfc3339(),"trials":results}),
    )?;
    Ok(i32::from(failed))
}
