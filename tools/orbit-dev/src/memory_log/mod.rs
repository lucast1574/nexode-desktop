//! Offline runtime-memory JSONL analysis, preserving per-process lifetime selection.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
mod hints;
mod incident;
mod report;
mod stats;
const MB: i64 = 1024 * 1024;
fn int(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|n| n as i64))
}
fn n(v: &Value) -> i64 {
    int(v).unwrap_or(0)
}
fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}
fn first(v: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| int(&v[*key]))
}
fn nonempty(v: &Value) -> bool {
    v.as_object().is_some_and(|m| !m.is_empty())
}
fn rounded(v: f64) -> f64 {
    (v * 10000.0).round_ties_even() / 10000.0
}
fn mb(v: Option<i64>) -> String {
    v.map(|n| format!("{:.1} MB", n as f64 / MB as f64))
        .unwrap_or_else(|| "n/a".into())
}
#[derive(Clone)]
struct Sample {
    path: PathBuf,
    line: usize,
    raw: Value,
    time: i64,
    kind: String,
    target: String,
    instance: String,
    source: String,
    category: String,
    reason: String,
}
impl Sample {
    fn new(path: PathBuf, line: usize, raw: Value) -> Self {
        let source = s(&raw["source"]).to_owned();
        let kind = if !s(&raw["kind"]).is_empty() {
            s(&raw["kind"])
        } else if raw["sessions"].is_object() {
            "attribution"
        } else if source.starts_with("process:") {
            "process"
        } else if source.starts_with("attribution:") {
            "attribution"
        } else {
            "legacy"
        }
        .to_owned();
        let target = if raw["client"].is_object() {
            "client"
        } else if raw["server"].is_object() {
            "server"
        } else if path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("client-runtime-memory-"))
        {
            "client"
        } else {
            "server"
        }
        .to_owned();
        let id = if target == "server" {
            &raw["server"]["id"]
        } else {
            &raw["client"]["client_instance_id"]
        };
        let instance = if s(id).is_empty() { "legacy" } else { s(id) }.to_owned();
        let mut category = s(&raw["trigger"]["category"]).to_owned();
        let mut reason = s(&raw["trigger"]["reason"]).to_owned();
        let (default_category, default_reason) = if source == "startup"
            || source.ends_with(":startup")
        {
            ("startup", "server_start")
        } else if source == "interval" || source.starts_with("process:heartbeat") {
            ("process_heartbeat", "periodic")
        } else if source.starts_with("attribution:heartbeat") {
            ("attribution_heartbeat", "periodic")
        } else if source.starts_with("attribution:event:") || source.starts_with("process:event:") {
            (source.split_once(":event:").unwrap().1, "event")
        } else if source.starts_with("server:runtime-log:") {
            let suffix = source.rsplit(':').next().unwrap();
            (
                suffix,
                if suffix == "interval" {
                    "periodic"
                } else {
                    &kind
                },
            )
        } else {
            (kind.as_str(), "legacy")
        };
        if category.is_empty() {
            category = default_category.into();
        }
        if reason.is_empty() {
            reason = default_reason.into();
        }
        Self {
            path,
            line,
            time: n(&raw["timestamp_ms"]),
            raw,
            kind,
            target,
            instance,
            source,
            category,
            reason,
        }
    }
    fn pss(&self) -> Option<i64> {
        int(&self.raw["process"]["os"]["pss_bytes"])
    }
    fn allocator(&self, key: &str) -> Option<i64> {
        int(&self.raw["process"]["allocator"]["stats"][key])
    }
    fn sessions(&self) -> &Value {
        &self.raw["sessions"]
    }
    fn totals(&self) -> &Value {
        &self.raw["totals"]
    }
    fn attributed(&self) -> Option<i64> {
        if nonempty(self.sessions()) {
            int(&self.sessions()["total_json_bytes"])
        } else if nonempty(self.totals()) {
            int(&self.totals()["total_attributed_bytes"])
        } else {
            None
        }
    }
}
fn latest(samples: &[Sample]) -> Vec<Sample> {
    let mut selected = BTreeMap::<&str, (&str, i64)>::new();
    for sample in samples {
        if selected
            .get(sample.target.as_str())
            .is_none_or(|(_, time)| sample.time > *time)
        {
            selected.insert(&sample.target, (&sample.instance, sample.time));
        }
    }
    samples
        .iter()
        .filter(|sample| selected[sample.target.as_str()].0 == sample.instance)
        .cloned()
        .collect()
}
fn counts<'a>(items: impl Iterator<Item = &'a str>) -> Value {
    let mut values = serde_json::Map::new();
    for key in items {
        let count = n(values.get(key).unwrap_or(&Value::Null));
        values.insert(key.into(), json!(count + 1));
    }
    Value::Object(values)
}
fn inventory(samples: &[Sample]) -> Vec<Value> {
    let mut groups = BTreeMap::<(&str, &str), Vec<&Sample>>::new();
    for sample in samples {
        groups
            .entry((&sample.target, &sample.instance))
            .or_default()
            .push(sample);
    }
    let mut result=groups.into_iter().map(|((target,instance),group)|json!({"target":target,"instance_id":instance,"sample_count":group.len(),"first_timestamp_ms":group[0].time,"last_timestamp_ms":group.last().unwrap().time,"duration_ms":(group.last().unwrap().time-group[0].time).max(0),"kinds":counts(group.iter().map(|s|s.kind.as_str()))})).collect::<Vec<_>>();
    result.sort_by(|a, b| {
        n(&b["last_timestamp_ms"])
            .cmp(&n(&a["last_timestamp_ms"]))
            .then_with(|| s(&b["target"]).cmp(s(&a["target"])))
    });
    result
}
fn last_attr(samples: &[Sample]) -> Option<&Sample> {
    samples
        .iter()
        .rev()
        .find(|s| nonempty(s.sessions()) || nonempty(s.totals()))
}
#[derive(Parser)]
struct Args {
    paths: Vec<PathBuf>,
    #[arg(long)]
    log_dir: Option<PathBuf>,
    #[arg(long)]
    days: Option<usize>,
    #[arg(long, default_value_t = 8)]
    top: usize,
    #[arg(long)]
    json: bool,
    #[arg(long,conflicts_with_all=["instance","list_instances"])]
    all_instances: bool,
    #[arg(long, conflicts_with = "list_instances")]
    instance: Option<String>,
    #[arg(long)]
    list_instances: bool,
    #[arg(long, default_value_t = 8.0)]
    min_spike_mb: f64,
}
fn expand(path: PathBuf) -> Result<PathBuf> {
    if path == Path::new("~") {
        return dirs::home_dir().context("home unavailable");
    }
    if let Ok(rest) = path.strip_prefix("~/") {
        return Ok(dirs::home_dir().context("home unavailable")?.join(rest));
    }
    Ok(path)
}
fn date(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?.strip_suffix(".jsonl")?;
    let parts = name.rsplit('-').take(3).collect::<Vec<_>>();
    if parts.len() != 3 {
        return None;
    }
    Some(parts.into_iter().rev().collect::<Vec<_>>().join("-"))
}
fn paths(args: &Args) -> Result<Vec<PathBuf>> {
    let mut inputs = args.paths.clone();
    if let Some(path) = &args.log_dir {
        inputs.push(path.clone());
    }
    if inputs.is_empty() {
        inputs.push(
            std::env::var_os("NEXODE_HOME")
                .map(PathBuf::from)
                .or_else(|| dirs::home_dir().map(|p| p.join(".nexode")))
                .context("home unavailable")?
                .join("logs/memory"),
        );
    }
    let mut files = BTreeSet::new();
    for path in inputs {
        let path = expand(path)?;
        if path.is_file() {
            files.insert(path.canonicalize()?);
        } else if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let path = entry?.path();
                if path.file_name().is_some_and(|s| {
                    let s = s.to_string_lossy();
                    s.contains("runtime-memory-") && s.ends_with(".jsonl")
                }) {
                    files.insert(path.canonicalize()?);
                }
            }
        }
    }
    if let Some(days) = args.days.filter(|n| *n > 0) {
        let mut dates = BTreeSet::new();
        for path in files.iter().rev() {
            if let Some(day) = date(path) {
                dates.insert(day);
                if dates.len() >= days {
                    break;
                }
            }
        }
        files.retain(|p| date(p).is_some_and(|d| dates.contains(&d)));
    }
    Ok(files.into_iter().collect())
}
fn load(paths: &[PathBuf]) -> Result<Vec<Sample>> {
    let mut samples = vec![];
    for path in paths {
        let text = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for (i, line) in text.lines().enumerate() {
            if let Ok(value) = serde_json::from_str::<Value>(line)
                && value.is_object()
            {
                samples.push(Sample::new(path.clone(), i + 1, value));
            }
        }
    }
    samples.sort_by(|a, b| {
        a.time
            .cmp(&b.time)
            .then(a.path.cmp(&b.path))
            .then(a.line.cmp(&b.line))
    });
    Ok(samples)
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let args =
        Args::try_parse_from(std::iter::once("analyze-runtime-memory-log".into()).chain(args))?;
    ensure!(
        args.min_spike_mb.is_finite() && args.min_spike_mb >= 0.0,
        "minimum spike must be finite and nonnegative"
    );
    let paths = paths(&args)?;
    ensure!(!paths.is_empty(), "No runtime memory log files found.");
    let mut samples = load(&paths)?;
    ensure!(
        !samples.is_empty(),
        "No runtime memory samples found in selected files."
    );
    if args.list_instances {
        let result = json!({"files":paths,"instances":inventory(&samples)});
        if args.json {
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!(
                "Runtime Memory Process Instances\n================================\nfiles: {}",
                paths.len()
            );
            for item in result["instances"].as_array().unwrap() {
                let timestamp = |key| {
                    chrono::DateTime::from_timestamp_millis(n(&item[key]))
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| "unknown".into())
                };
                println!(
                    "{} | target={} | samples={} | {} -> {} | duration={:.1}s",
                    s(&item["instance_id"]),
                    s(&item["target"]),
                    n(&item["sample_count"]),
                    timestamp("first_timestamp_ms"),
                    timestamp("last_timestamp_ms"),
                    n(&item["duration_ms"]) as f64 / 1000.0
                );
            }
        }
        return Ok(());
    }
    if let Some(id) = &args.instance {
        samples.retain(|s| s.instance == *id);
        ensure!(!samples.is_empty(), "Process instance was not found");
    } else if !args.all_instances {
        samples = latest(&samples);
    }
    let mut summary = report::summarize(&samples, args.top, (args.min_spike_mb * MB as f64) as i64);
    summary["files"] = json!(paths);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        report::human(&summary)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_python_reference_for_processes_incidents_clients_and_reload_boundaries() {
        let cases: Vec<Value> = serde_json::from_str(include_str!(
            "../../tests/fixtures/runtime_memory_reference.json"
        ))
        .unwrap();
        for case in cases {
            let mut samples = case["rows"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(i, raw)| {
                    Sample::new(
                        PathBuf::from(if raw["client"].is_object() {
                            "client-runtime-memory-2026-11-07.jsonl"
                        } else {
                            "server-runtime-memory-2026-11-07.jsonl"
                        }),
                        i + 1,
                        raw.clone(),
                    )
                })
                .collect::<Vec<_>>();
            samples.sort_by(|a, b| {
                a.time
                    .cmp(&b.time)
                    .then(a.path.cmp(&b.path))
                    .then(a.line.cmp(&b.line))
            });
            assert_eq!(
                json!(inventory(&samples)),
                case["inventory"],
                "inventory {}",
                case["name"]
            );
            match s(&case["mode"]) {
                "latest" => samples = latest(&samples),
                "all" => {}
                instance => samples.retain(|s| s.instance == instance),
            }
            let actual = report::summarize(&samples, n(&case["top"]) as usize, 8 * MB);
            assert_eq!(actual, case["expected"], "scenario {}", case["name"]);
        }
    }
}
