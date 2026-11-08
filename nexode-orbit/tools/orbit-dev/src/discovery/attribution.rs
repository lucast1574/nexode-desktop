//! Score sponsored Discovery entries, including their actual CLI signup contract.
use anyhow::{Context, Result, ensure};
use clap::Parser;
use regex::Regex;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
const CRITICAL: &str = "cli_flow_attributable";
fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or("")
}
fn expectations(data: &Value) -> Result<Vec<Value>> {
    ensure!(
        data["version"] == 1,
        "unsupported sponsor expectation version"
    );
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for raw in data["sponsors"].as_array().context("missing sponsors")? {
        let mut v = raw.clone();
        for key in ["tool", "category", "mechanism"] {
            v[key] = json!(s(raw, key).trim().to_lowercase());
        }
        ensure!(
            !s(&v, "tool").is_empty() && !s(&v, "category").is_empty(),
            "missing tool/category"
        );
        ensure!(
            [
                "referral-link",
                "cli-flag",
                "signup-code",
                "utm-forwarding",
                "api-partner-id"
            ]
            .contains(&s(&v, "mechanism")),
            "unknown mechanism"
        );
        ensure!(
            seen.insert(s(&v, "tool").to_owned()),
            "duplicate sponsor expectation"
        );
        for key in ["marker", "listing_marker"] {
            v[key] = json!(
                raw.get(key)
                    .and_then(Value::as_str)
                    .unwrap_or(s(data, "default_marker"))
                    .trim()
            );
            ensure!(!s(&v, key).is_empty(), "missing {key}");
        }
        out.push(v);
    }
    Ok(out)
}
fn catalog(data: &Value) -> Result<BTreeMap<String, Value>> {
    let mut entries = BTreeMap::new();
    for (category, raw) in data
        .get("categories")
        .unwrap_or(data)
        .as_object()
        .context("catalog categories must be an object")?
    {
        for tool in raw
            .get("tools")
            .unwrap_or(raw)
            .as_array()
            .context("catalog tools must be an array")?
        {
            let name = s(tool, "name").trim().to_lowercase();
            if !name.is_empty() {
                let mut tool = tool.clone();
                tool["category"] = json!(category);
                entries.insert(name, tool);
            }
        }
    }
    Ok(entries)
}
fn http_get(url: &str, timeout: f64, benchmark: bool) -> Result<(u16, String, String)> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs_f64(timeout))
        .user_agent("nexode-attribution-benchmark")
        .build()?;
    let mut request = client.get(url);
    if benchmark {
        request = request.header("x-nexode-discovery-benchmark", "1");
    }
    let response = request.send()?.error_for_status()?;
    let status = response.status().as_u16();
    let url = response.url().to_string();
    let mut bytes = Vec::new();
    response.take(65536).read_to_end(&mut bytes)?;
    Ok((status, url, String::from_utf8_lossy(&bytes).into_owned()))
}
fn fetch(endpoint: &str, spec: &Value, timeout: f64) -> Result<Value> {
    let mut url = url::Url::parse(endpoint)?;
    url.query_pairs_mut().append_pair("category",s(spec,"category")).append_pair("q",&format!("attribution benchmark validation for {} listings",s(spec,"category").replace('-' ," "))).append_pair("reason","Automated attribution benchmark verifying that sponsor listing and setup data preserve the nexode referral marker. No account will be created.");
    let (code, _, body) = http_get(url.as_str(), timeout, true)?;
    ensure!(code == 200, "browse failed: HTTP {code}");
    let browse: Value = serde_json::from_str(&body)?;
    let mut entry = browse["tools"]
        .as_array()
        .context("missing browse tools")?
        .iter()
        .find(|t| s(t, "name").to_lowercase() == s(spec, "tool"))
        .context("sponsor missing from live browse")?
        .clone();
    url.query_pairs_mut().append_pair("tool", s(spec, "tool"));
    let (code, _, body) = http_get(url.as_str(), timeout, true)?;
    ensure!(code == 200, "select failed: HTTP {code}");
    let selected: Value = serde_json::from_str(&body)?;
    if let Some(tool) = selected["tool"].as_object() {
        entry
            .as_object_mut()
            .context("invalid listing")?
            .extend(tool.clone());
    }
    entry["category"] = spec["category"].clone();
    Ok(entry)
}
fn urls(text: &str) -> Vec<String> {
    Regex::new(r#"https?://[^\s`'\")\]>]+"#)
        .expect("URL regex")
        .find_iter(text)
        .map(|m| m.as_str().trim_end_matches(['.', ',', ';']).to_owned())
        .collect()
}
fn marked(url: &str, marker: &str) -> bool {
    url.to_lowercase().contains(&marker.to_lowercase())
}
fn report(spec: &Value, checks: Vec<Value>) -> Value {
    let mut total = 0;
    let mut passed = 0;
    for c in &checks {
        if c["status"] != "skip" {
            let w = if c["name"] == CRITICAL { 3 } else { 1 };
            total += w;
            if c["status"] == "pass" {
                passed += w;
            }
        }
    }
    let score = if total == 0 {
        0
    } else {
        (100.0 * passed as f64 / total as f64).round_ties_even() as u64
    };
    let cli = checks.iter().find(|c| c["name"] == CRITICAL);
    let verdict = match cli.map(|c| s(c, "status")) {
        Some("pass") => "attributed",
        Some("fail") => "NOT-ATTRIBUTED",
        _ => "unknown",
    };
    json!({"tool":spec["tool"],"category":spec["category"],"mechanism":spec["mechanism"],"score":score,"cli_attribution":verdict,"cli_attribution_detail":cli.map(|c|s(c,"detail")).unwrap_or("CLI attribution never evaluated"),"checks":checks})
}
fn check(spec: &Value, entry: Option<&Value>, live_web: bool, timeout: f64) -> Result<Value> {
    let mut checks = Vec::new();
    let mut add = |name: &str, status: &str, detail: String| {
        checks.push(json!({"name":name,"status":status,"detail":detail}));
    };
    let Some(entry) = entry else {
        add(
            "catalog_entry_present",
            "fail",
            "sponsor not found in catalog".into(),
        );
        return Ok(report(spec, checks));
    };
    add("catalog_entry_present", "pass", "listed".into());
    let url = s(entry, "url");
    let setup = s(entry, "setup");
    let marker = s(spec, "marker");
    let listing = spec
        .get("listing_marker")
        .and_then(Value::as_str)
        .unwrap_or(marker);
    if url.is_empty() {
        add(
            "listing_url_marked",
            "fail",
            "no URL in catalog entry".into(),
        );
    } else if marked(url, listing) {
        add("listing_url_marked", "pass", url.into());
    } else {
        add(
            "listing_url_marked",
            "fail",
            format!("URL lacks marker {listing:?}: {url}"),
        );
    }
    let setup_urls = urls(setup);
    if setup.is_empty() {
        add(
            "setup_preserves_marker",
            "skip",
            "no setup text (offline browse-only entry)".into(),
        );
    } else {
        let host = |u: &url::Url| {
            let mut h = u
                .host_str()
                .unwrap_or("")
                .trim_start_matches("www.")
                .to_lowercase();
            if let Some(p) = u.port() {
                h += &format!(":{p}");
            }
            h
        };
        let vendor = url::Url::parse(url)
            .ok()
            .map(|u| host(&u))
            .unwrap_or_default();
        let signup = Regex::new("(?i)(signup|sign-up|register|get[-_]?started|join)")?;
        let unmarked: Vec<_> = setup_urls
            .iter()
            .filter(|u| {
                let Ok(p) = url::Url::parse(u) else {
                    return false;
                };
                let h = host(&p);
                let same =
                    !vendor.is_empty() && (h == vendor || h.ends_with(&format!(".{vendor}")));
                let api = h.starts_with("docs.")
                    || h.starts_with("api.")
                    || p.path().contains("/api")
                    || p.path().contains("/oauth");
                same && signup.is_match(p.path()) && !api && !marked(u, listing)
            })
            .collect();
        if unmarked.is_empty() {
            add(
                "setup_preserves_marker",
                "pass",
                "no unmarked vendor signup URLs".into(),
            );
        } else {
            add(
                "setup_preserves_marker",
                "fail",
                format!("unmarked vendor/signup URLs in setup: {unmarked:?}"),
            );
        }
    }
    if !setup.is_empty()&&Regex::new(r"(?i)(npx |npm |pipx? |python(?:3)? |uvx? |cargo |brew |curl |mcp|api[ _-]?key|cli|mock-sponsor signup)")?.is_match(setup) {
        let mechanism=s(spec,"mechanism");
        if mechanism!="referral-link" {
            let pass=marked(setup,marker);add(CRITICAL,if pass{"pass"}else{"fail"},format!("setup {} {mechanism} marker {marker:?}",if pass{"contains"}else{"omits configured"}));
        } else if setup_urls.iter().any(|u|marked(u,listing)) {add(CRITICAL,"pass","setup routes signup through marked URL".into());}
        else {add(CRITICAL,"fail","CLI-first setup with cookie-based referral-link attribution and no marked signup URL in setup; agent-driven CLI signups will not be attributed".into());}
    } else {add(CRITICAL,"skip","setup is not CLI-first or unavailable".into());}
    if live_web && !url.is_empty() && marked(url, listing) {
        match http_get(url, timeout, false) {
            Ok((_, final_url, _)) if !marked(&final_url, listing) && final_url != url => add(
                "live_url_resolves",
                "fail",
                format!("redirect dropped marker: {url} -> {final_url}"),
            ),
            Ok((status, final_url, _)) => add(
                "live_url_resolves",
                "pass",
                format!("HTTP {status} at {final_url}"),
            ),
            Err(e) => add("live_url_resolves", "fail", format!("request error: {e}")),
        }
    } else {
        add(
            "live_url_resolves",
            "skip",
            "live web check disabled or URL unmarked".into(),
        );
    }
    Ok(report(spec, checks))
}
#[derive(Parser)]
struct Args {
    #[arg(long)]
    sponsors: Option<PathBuf>,
    #[arg(long)]
    catalog_file: Option<PathBuf>,
    #[arg(long)]
    live: bool,
    #[arg(long)]
    live_web: bool,
    #[arg(long, default_value = "https://api.nexode.invalid/v1/discovery")]
    endpoint: String,
    #[arg(long, default_value_t = 10.0)]
    timeout: f64,
    #[arg(long, default_value_t = 100)]
    min_score: u64,
    #[arg(long = "sponsor")]
    only: Vec<String>,
    #[arg(long)]
    output: Option<PathBuf>,
}
pub fn run(root: &Path, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("benchmark-attribution".into()).chain(args));
    ensure!(
        args.catalog_file.is_some() || args.live,
        "provide --catalog-file or --live"
    );
    ensure!(
        args.timeout.is_finite() && args.timeout > 0.0,
        "timeout must be positive"
    );
    let path = args
        .sponsors
        .unwrap_or(root.join("scripts/attribution_benchmark_sponsors.json"));
    let mut specs = expectations(&serde_json::from_slice(&fs::read(path)?)?)?;
    if !args.only.is_empty() {
        let wanted: BTreeSet<_> = args.only.iter().map(|s| s.to_lowercase()).collect();
        for name in &wanted {
            ensure!(
                specs.iter().any(|s| s["tool"] == *name),
                "unknown --sponsor value: {name}"
            );
        }
        specs.retain(|s| wanted.contains(s["tool"].as_str().unwrap()));
    }
    ensure!(!specs.is_empty(), "no sponsors to evaluate");
    let entries = if let Some(path) = args.catalog_file {
        catalog(&serde_json::from_slice(&fs::read(path)?)?)?
    } else {
        BTreeMap::new()
    };
    let mut reports = Vec::new();
    let mut errors = Vec::new();
    for spec in specs {
        let mut entry = entries.get(s(&spec, "tool")).cloned();
        if args.live {
            match fetch(&args.endpoint, &spec, args.timeout) {
                Ok(live) => {
                    if let Some(ref mut e) = entry {
                        e.as_object_mut()
                            .context("invalid offline entry")?
                            .extend(live.as_object().context("invalid live entry")?.clone());
                    } else {
                        entry = Some(live);
                    }
                }
                Err(e) => {
                    errors.push(format!("{}: {e:#}", s(&spec, "tool")));
                    if entry.is_none() {
                        reports.push(report(
                            &spec,
                            vec![
                                json!({"name":"live_fetch","status":"fail","detail":e.to_string()}),
                            ],
                        ));
                        continue;
                    }
                }
            }
        }
        reports.push(check(&spec, entry.as_ref(), args.live_web, args.timeout)?);
    }
    let verdicts: serde_json::Map<String, Value> = reports
        .iter()
        .map(|r| (s(r, "tool").to_owned(), r["cli_attribution"].clone()))
        .collect();
    let result = json!({"generated_at":chrono::Utc::now().to_rfc3339(),"endpoint":if args.live{Some(&args.endpoint)}else{None},"live_web":args.live_web,"errors":errors,"primary_check":CRITICAL,"cli_attribution":verdicts,"sponsors":reports});
    let output = args
        .output
        .unwrap_or(root.join("target/attribution-benchmark/latest.json"));
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&result)?),
    )?;
    for r in &reports {
        println!(
            "{} {} score={} cli_attribution={} {}",
            s(r, "tool"),
            s(r, "category"),
            r["score"],
            s(r, "cli_attribution"),
            s(r, "cli_attribution_detail")
        );
    }
    for error in &errors {
        eprintln!("error: {error}");
    }
    let attributed = reports
        .iter()
        .filter(|r| r["cli_attribution"] == "attributed")
        .count();
    println!(
        "report: {}\nCLI-flow attribution ({CRITICAL}) is the primary signal.\n  => {attributed}/{} sponsors credit agent-driven CLI signups to nexode",
        output.display(),
        reports.len()
    );
    ensure!(
        reports.iter().all(|r| r["cli_attribution"] == "attributed"
            && r["score"].as_u64().unwrap_or(0) >= args.min_score),
        "sponsor score or CLI attribution requirement failed"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn spec() -> Value {
        json!({"tool":"example","category":"databases","mechanism":"referral-link","marker":"via=nexode-discovery"})
    }
    fn checked(setup: &str, spec: &Value) -> Value {
        check(
            spec,
            Some(&json!({"url":"https://example.com/?via=nexode-discovery","setup":setup})),
            false,
            1.0,
        )
        .unwrap()
    }
    fn status<'a>(r: &'a Value, name: &str) -> &'a str {
        s(
            r["checks"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == name)
                .unwrap(),
            "status",
        )
    }
    #[test]
    fn loading_and_catalog_contracts() {
        let root = crate::root().unwrap();
        let specs = expectations(
            &serde_json::from_slice(
                &fs::read(root.join("scripts/attribution_benchmark_sponsors.json")).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(!specs.is_empty());
        let cases: Value = serde_json::from_slice(
            &fs::read(root.join("scripts/discovery_benchmark_cases.json")).unwrap(),
        )
        .unwrap();
        for c in cases["cases"].as_array().unwrap() {
            if let Some(t) = c["expected_tool"].as_str() {
                assert!(specs.iter().any(|s| s["tool"] == t));
            }
        }
        for bad in [
            json!({"tool":"x","category":"y","mechanism":"vibes"}),
            json!({"tool":"x","category":"y","mechanism":"cli-flag","marker":""}),
        ] {
            assert!(expectations(&json!({"version":1,"sponsors":[bad]})).is_err());
        }
        let c=catalog(&json!({"categories":{"databases":{"tools":[{"name":"Example"}]},"payments":[{"name":"card"}]}})).unwrap();
        assert!(c.contains_key("example"));
        assert_eq!(c["card"]["category"], "payments");
    }
    #[test]
    fn markers_cookie_and_cli_contracts() {
        let good = checked(
            "Sign up at https://example.com/signup?via=nexode-discovery then run npx x",
            &spec(),
        );
        assert_eq!(good["score"], 100);
        assert_eq!(good["cli_attribution"], "attributed");
        assert_eq!(check(&spec(), None, false, 1.0).unwrap()["score"], 0);
        let missing = check(
            &spec(),
            Some(&json!({"url":"https://example.com/"})),
            false,
            1.0,
        )
        .unwrap();
        assert_eq!(status(&missing, "listing_url_marked"), "fail");
        let bad = checked(
            "Create an account at https://app.example.com/signup and paste the API key",
            &spec(),
        );
        assert_eq!(status(&bad, "setup_preserves_marker"), "fail");
        let cli = checked("Run npx example and paste API key", &spec());
        assert_eq!(cli["cli_attribution"], "NOT-ATTRIBUTED");
        assert!(s(&cli, "cli_attribution_detail").contains("not be attributed"));
        let mut noncookie = spec();
        noncookie["mechanism"] = json!("cli-flag");
        noncookie["marker"] = json!("--via nexode-discovery");
        noncookie["listing_marker"] = json!("via=nexode-discovery");
        assert_eq!(
            checked(
                "Run example signup --via nexode-discovery then install CLI",
                &noncookie
            )["cli_attribution"],
            "attributed"
        );
        assert_eq!(
            checked("Run example signup then install CLI", &noncookie)["cli_attribution"],
            "NOT-ATTRIBUTED"
        );
        assert_eq!(
            checked(
                "Open https://example.com/dashboard?via=nexode-discovery",
                &spec()
            )["cli_attribution"],
            "unknown"
        );
        assert_eq!(report(&spec(), vec![])["cli_attribution"], "unknown");
        for setup in [
            "Docs at https://docs.other-vendor.com/quickstart. Run pip install example.",
            "Run curl https://api.example.com/signup",
            "Run curl https://docs.example.com/signup",
            "Run curl https://example.com/oauth/signup",
        ] {
            assert_eq!(
                status(&checked(setup, &spec()), "setup_preserves_marker"),
                "pass"
            );
        }
        let other=check(&spec(),Some(&json!({"url":"https://example.com/","setup":"Sign up at https://example.com/signup?via=nexode-discovery then run npx x"})),false,1.0).unwrap();
        assert!(cli["score"].as_u64() < other["score"].as_u64());
    }
}
