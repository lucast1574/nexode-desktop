use anyhow::{Context, Result, ensure};
use chrono::{Datelike, Duration, Local, NaiveDate};
use clap::Parser;
use serde_json::{Value, json};
use std::{fs, path::PathBuf, process::Command};
#[derive(Parser)]
#[command(about = "Generate repository badges from GitHub API data through gh")]
struct Args {
    #[arg(long)]
    repo: Option<String>,
    #[arg(long)]
    output: Option<PathBuf>,
    /// Read captured GitHub API pages instead of contacting GitHub.
    #[arg(long)]
    input: Option<PathBuf>,
    #[arg(long)]
    today: Option<NaiveDate>,
}
fn human(n: u64) -> String {
    for (div, suffix, precision) in [(1_000_000, "M", 2), (1_000, "k", 1)] {
        if n >= div {
            return format!("{:.*}", precision, n as f64 / div as f64)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_owned()
                + suffix;
        }
    }
    n.to_string()
}
fn comma(n: u64) -> String {
    let text = n.to_string();
    text.chars()
        .enumerate()
        .fold(String::new(), |mut s, (i, c)| {
            if i > 0 && (text.len() - i).is_multiple_of(3) {
                s.push(',');
            }
            s.push(c);
            s
        })
}
fn escaped(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
fn render(repo: &str, dates: &[NaiveDate], today: NaiveDate) -> Result<String> {
    ensure!(!dates.is_empty(), "GitHub returned no stargazers");
    let current = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let weeks: Vec<_> = (0..26)
        .rev()
        .map(|i| current - Duration::weeks(i))
        .collect();
    let values: Vec<_> = weeks
        .iter()
        .map(|w| {
            dates
                .iter()
                .filter(|d| **d <= (*w + Duration::days(6)).min(today))
                .count() as u64
        })
        .collect();
    let weekly: Vec<_> = weeks
        .iter()
        .map(|w| {
            dates
                .iter()
                .filter(|d| **d >= *w && **d <= (*w + Duration::days(6)).min(today))
                .count() as u64
        })
        .collect();
    let latest = *values.last().unwrap();
    let magnitude = 10_u64.pow(latest.to_string().len().saturating_sub(2) as u32);
    let step = magnitude.max(latest.div_ceil(4 * magnitude) * magnitude);
    let max = (step * 4).max(4);
    let x = |i: usize| 68.0 + i as f64 * 708.0 / 25.0;
    let y = |v: u64| 78.0 + (1.0 - v as f64 / max as f64) * 278.0;
    let mut ticks = String::new();
    for i in 0..5 {
        let value = step * i;
        let yy = y(value);
        let label = if value >= 1000 {
            format!("{}k", value as f64 / 1000.0)
        } else {
            value.to_string()
        };
        ticks += &format!(
            r#"<line x1="68" y1="{yy:.1}" x2="776" y2="{yy:.1}" class="grid"/><text x="56" y="{:.1}" text-anchor="end">{label}</text>"#,
            yy + 5.0
        );
    }
    let points = values
        .iter()
        .enumerate()
        .map(|(i, v)| format!("{:.1},{:.1}", x(i), y(*v)))
        .collect::<Vec<_>>()
        .join(" ");
    let area = format!("68,356.0 {points} 776,356.0");
    let mut dots = String::new();
    let mut labels = String::new();
    for (i, w) in weeks.iter().enumerate() {
        let xx = x(i);
        let yy = y(values[i]);
        let label = w.format("%b %-d").to_string();
        if i % 4 == 0 || i == 25 {
            labels += &format!(r#"<text x="{xx:.1}" y="390" text-anchor="middle">{label}</text>"#);
        }
        let class = if i == 25 { " current" } else { "" };
        let radius = if i == 25 { 5 } else { 3 };
        dots += &format!(
            r#"<circle class="point{class}" cx="{xx:.1}" cy="{yy:.1}" r="{radius}"><title>{label}: {} total stars (+{} that week)</title></circle>"#,
            comma(values[i]),
            comma(weekly[i])
        );
    }
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 420" role="img" aria-labelledby="title desc">
<title id="title">{} cumulative GitHub stars over time</title>
<desc id="desc">Cumulative GitHub stars sampled weekly over the last 26 weeks, ending at {} stars.</desc>
<style>
:root {{color-scheme:light dark}} .bg {{fill:#fff}} text {{fill:#57606a;font:13px -apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif}}
.grid {{stroke:#d8dee4;stroke-width:1}} .area {{fill:#2f81f7;fill-opacity:.14}} .line {{fill:none;stroke:#2f81f7;stroke-width:3;stroke-linejoin:round;stroke-linecap:round}}
.point {{fill:#fff;stroke:#2f81f7;stroke-width:2}} .point.current {{fill:#2f81f7;stroke:#fff;stroke-width:2}}
.heading {{fill:#24292f;font-size:17px;font-weight:600}} .metric {{fill:#24292f;font-size:18px;font-weight:600}}
@media (prefers-color-scheme:dark) {{.bg {{fill:#0d1117}} text {{fill:#8b949e}} .grid {{stroke:#30363d}} .area {{fill:#58a6ff;fill-opacity:.16}} .line {{stroke:#58a6ff}} .point {{fill:#0d1117;stroke:#58a6ff}} .point.current {{fill:#58a6ff;stroke:#0d1117}} .heading,.metric {{fill:#f0f6fc}}}}
</style>
<rect class="bg" width="100%" height="100%" rx="6"/>
<text class="heading" x="68" y="28">GitHub stars over time</text>
<text x="68" y="51">Cumulative stars · weekly sampling · last 26 weeks</text>
<text class="metric" x="776" y="28" text-anchor="end">{} stars</text>
<text x="776" y="51" text-anchor="end">+{} this week so far</text>
{ticks}<polygon class="area" points="{area}"/><polyline class="line" points="{points}"/>{dots}{labels}
</svg>
"##,
        escaped(repo),
        comma(latest),
        comma(latest),
        comma(weekly[25])
    ))
}
fn download_total(pages: &Value) -> Result<u64> {
    let mut total = 0_u64;
    for page in pages.as_array().context("expected API pages")? {
        for release in page.as_array().context("expected release page")? {
            for asset in release["assets"].as_array().context("missing assets")? {
                total = total
                    .checked_add(
                        asset["download_count"]
                            .as_u64()
                            .context("invalid download count")?,
                    )
                    .context("download count overflow")?;
            }
        }
    }
    Ok(total)
}
pub fn run(stars: bool, args: impl Iterator<Item = String>) -> Result<()> {
    let args = Args::parse_from(std::iter::once("repository-badge".into()).chain(args));
    let repo = args
        .repo
        .or_else(|| std::env::var("GITHUB_REPOSITORY").ok())
        .context("provide --repo OWNER/REPO or GITHUB_REPOSITORY")?;
    ensure!(
        regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*$")?
            .is_match(&repo),
        "invalid repository"
    );
    let pages: Value = if let Some(input) = args.input {
        serde_json::from_slice(&fs::read(input)?)?
    } else {
        let endpoint = format!(
            "repos/{repo}/{}?per_page=100",
            if stars { "stargazers" } else { "releases" }
        );
        let out = Command::new("gh")
            .args([
                "api",
                "--paginate",
                "--slurp",
                "-H",
                if stars {
                    "Accept: application/vnd.github.star+json"
                } else {
                    "Accept: application/vnd.github+json"
                },
                &endpoint,
            ])
            .output()
            .context("run gh API")?;
        ensure!(
            out.status.success(),
            "GitHub API request failed (exit {})",
            out.status
        );
        serde_json::from_slice(&out.stdout)?
    };
    let output = args.output.unwrap_or_else(|| {
        PathBuf::from(if stars {
            "docs/images/star-history.svg"
        } else {
            "docs/badges/downloads.json"
        })
    });
    let content = if stars {
        let mut dates = Vec::new();
        for page in pages.as_array().context("expected pages")? {
            for star in page.as_array().context("expected star page")? {
                dates.push(
                    chrono::DateTime::parse_from_rfc3339(
                        star["starred_at"].as_str().context("missing star date")?,
                    )?
                    .date_naive(),
                );
            }
        }
        render(
            &repo,
            &dates,
            args.today.unwrap_or_else(|| Local::now().date_naive()),
        )?
    } else {
        let total = download_total(&pages)?;
        println!("{total}");
        format!(
            "{}\n",
            json!({"schemaVersion":1,"label":"downloads","message":human(total),"color":"brightgreen"})
        )
    };
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(output, content)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn badge_counts_all_pages_and_formats_thresholds() {
        assert_eq!(download_total(&json!([[{"assets":[{"download_count":1000},{"download_count":5}]}],[{"assets":[{"download_count":17}]}],[]])).unwrap(),1022);
        for (n, s) in [
            (0, "0"),
            (999, "999"),
            (1000, "1k"),
            (1234, "1.2k"),
            (1000000, "1M"),
            (1250000, "1.25M"),
        ] {
            assert_eq!(human(n), s);
        }
        assert!(download_total(&json!([[{"assets":[{"download_count":-1}]}]])).is_err());
    }
    #[test]
    fn chart_has_26_weeks_including_partial_week_and_escapes_xml() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let dates = vec![
            today - Duration::days(365),
            today - Duration::days(3),
            today,
        ];
        let svg = render("owner/<repo>", &dates, today).unwrap();
        let document = roxmltree::Document::parse(&svg).unwrap();
        assert_eq!(
            document
                .descendants()
                .filter(|n| n.has_tag_name("circle"))
                .count(),
            26
        );
        assert!(svg.contains("3 stars"));
        assert!(svg.contains("+2 this week so far"));
        assert!(svg.contains("owner/&lt;repo&gt;"));
        assert!(render("o/r", &[], today).is_err());
    }
}
