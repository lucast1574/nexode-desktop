//! Render the archived Reddit dataset, without fetching or presenting it as current.
use anyhow::{Result, ensure};
use clap::Parser;
use plotters::{coord::Shift, prelude::*};
use serde_json::Value;
use std::path::PathBuf;

const DATA: &str = include_str!("reddit_data.json");
const BG: RGBColor = RGBColor(13, 17, 23);
const FG: RGBColor = RGBColor(230, 237, 243);
const COLORS: [RGBColor; 5] = [
    RGBColor(88, 166, 255),
    RGBColor(63, 185, 80),
    RGBColor(210, 168, 255),
    RGBColor(240, 136, 62),
    RGBColor(248, 81, 73),
];
#[derive(Parser)]
struct Args {
    /// OpenType/TrueType font; defaults to DejaVu Sans, Arial or Liberation Sans.
    #[arg(long)]
    font: Option<PathBuf>,
    #[arg(long, default_value = "/tmp/nexode_reddit_dashboard.png")]
    output: PathBuf,
    /// Also export the archived inputs and derived metrics as JSON.
    #[arg(long)]
    json: Option<PathBuf>,
}
fn number(v: &Value, key: &str) -> f64 {
    v[key].as_f64().expect("embedded numeric dataset")
}
fn composite(v: &Value) -> f64 {
    number(v, "relevance") * (number(v, "avg_up") + 2. * number(v, "avg_com"))
}
fn normalized(row: &Value) -> Vec<f64> {
    let values: Vec<f64> = row
        .as_array()
        .expect("embedded array")
        .iter()
        .map(|v| v.as_f64().unwrap())
        .collect();
    let total: f64 = values.iter().sum();
    values
        .into_iter()
        .map(|n| if total == 0. { 0. } else { 100. * n / total })
        .collect()
}
fn peak(row: &[f64]) -> Option<usize> {
    row.iter()
        .enumerate()
        .fold(None, |best, (i, n)| match best {
            Some(j) if row[j] >= *n => Some(j),
            _ => Some(i),
        })
        .filter(|&i| row[i] > 0.)
}
fn color(v: &Value) -> RGBColor {
    COLORS[v["tier"].as_u64().unwrap() as usize - 1]
}
fn text<B: DrawingBackend>(
    a: &DrawingArea<B, Shift>,
    s: impl Into<String>,
    x: i32,
    y: i32,
    size: u32,
    c: RGBColor,
) -> Result<()>
where
    B::ErrorType: 'static,
{
    a.draw(&Text::new(
        s.into(),
        (x, y),
        ("sans-serif", size).into_font().color(&c),
    ))?;
    Ok(())
}
fn title<B: DrawingBackend>(a: &DrawingArea<B, Shift>, s: &str) -> Result<()>
where
    B::ErrorType: 'static,
{
    text(a, s, 24, 25, 24, COLORS[0])
}
fn bars<B: DrawingBackend>(
    a: &DrawingArea<B, Shift>,
    data: &Value,
    label: &str,
    metric: fn(&Value) -> f64,
) -> Result<()>
where
    B::ErrorType: 'static,
{
    title(a, label)?;
    let mut rows: Vec<_> = data.as_object().unwrap().iter().collect();
    rows.sort_by(|a, b| metric(b.1).total_cmp(&metric(a.1)));
    let max = metric(rows[0].1);
    let w = a.dim_in_pixel().0 as f64;
    for (i, (name, v)) in rows.iter().enumerate() {
        let y = 65 + i as i32 * 27;
        text(a, *name, 24, y, 17, FG)?;
        let end = 260. + (w - 370.) * metric(v) / max;
        a.draw(&Rectangle::new(
            [(260, y - 9), (end as i32, y + 9)],
            color(v).filled(),
        ))?;
        text(a, format!("{:.1}", metric(v)), end as i32 + 8, y, 16, FG)?;
    }
    Ok(())
}
fn render<B: DrawingBackend>(root: DrawingArea<B, Shift>, d: &Value) -> Result<()>
where
    B::ErrorType: 'static,
{
    root.fill(&BG)?;
    text(
        &root,
        "Nexode Reddit Strategy Dashboard",
        40,
        45,
        38,
        COLORS[0],
    )?;
    text(
        &root,
        "Archived snapshot: 15 communities. Historical planning data, not current recommendations. Times: Pacific.",
        40,
        95,
        23,
        FG,
    )?;
    for (i, label) in [
        "Perfect fit",
        "Strong fit",
        "Good fit",
        "Broad reach",
        "Skip",
    ]
    .iter()
    .enumerate()
    {
        text(
            &root,
            format!("Tier {}: {label}", i + 1),
            40 + i as i32 * 460,
            140,
            22,
            COLORS[i],
        )?;
    }
    let (_, body) = root.split_vertically(185);
    let rows = body.split_evenly((6, 1));
    let data = &d["data"];
    bars(
        &rows[0],
        data,
        "OVERALL RANKING: relevance x (average upvotes + 2 x average comments)",
        composite,
    )?;
    let pair = rows[1].split_evenly((1, 2));
    title(&pair[0], "SUBSCRIBERS vs ENGAGEMENT")?;
    let mut chart = ChartBuilder::on(&pair[0])
        .margin_top(60)
        .margin_right(150)
        .margin_left(20)
        .margin_bottom(15)
        .x_label_area_size(45)
        .y_label_area_size(65)
        .build_cartesian_2d((80f64..10000f64).log_scale(), 0f64..190f64)?;
    chart
        .configure_mesh()
        .x_desc("Subscribers (thousands, log scale)")
        .y_desc("Average upvotes")
        .axis_style(FG)
        .label_style(("sans-serif", 17).into_font().color(&FG))
        .axis_desc_style(("sans-serif", 18).into_font().color(&FG))
        .light_line_style(RGBColor(33, 38, 45))
        .draw()?;
    for (name, v) in data.as_object().unwrap() {
        let p = (number(v, "subs"), number(v, "avg_up"));
        chart.draw_series(std::iter::once(Circle::new(
            p,
            (number(v, "relevance") * 5.).sqrt() as i32 + 3,
            color(v).filled(),
        )))?;
        chart.draw_series(std::iter::once(
            EmptyElement::at(p)
                + Text::new(
                    name.clone(),
                    (10, -8),
                    ("sans-serif", 13).into_font().color(&FG),
                ),
        ))?;
    }
    bars(&pair[1], data, "DISCUSSION DEPTH: average comments", |v| {
        number(v, "avg_com")
    })?;
    title(
        &rows[2],
        "POSTING ACTIVITY BY HOUR: percentage within each community; * = first peak",
    )?;
    for h in 0..24 {
        text(&rows[2], h.to_string(), 280 + h * 83, 65, 18, FG)?;
    }
    let max = d["hour_data"]
        .as_object()
        .unwrap()
        .values()
        .flat_map(normalized)
        .fold(0f64, f64::max);
    for (i, (name, row)) in d["hour_data"].as_object().unwrap().iter().enumerate() {
        let y = 95 + i as i32 * 27;
        text(&rows[2], name, 24, y, 17, FG)?;
        let values = normalized(row);
        let peak = peak(&values);
        for (h, n) in values.iter().enumerate() {
            let t = n / max;
            let c = RGBColor(255, (240. - 210. * t) as u8, (180. - 160. * t) as u8);
            let x = 265 + h as i32 * 83;
            rows[2].draw(&Rectangle::new([(x, y - 12), (x + 80, y + 12)], c.filled()))?;
            text(
                &rows[2],
                format!("{n:.1}{}", if peak == Some(h) { "*" } else { "" }),
                x + 5,
                y,
                14,
                BLACK,
            )?;
        }
    }
    let pair = rows[3].split_evenly((1, 2));
    title(&pair[0], "DAY OF WEEK: within-community percentage")?;
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut chart = ChartBuilder::on(&pair[0])
        .margin_top(65)
        .margin_right(20)
        .margin_left(15)
        .margin_bottom(125)
        .x_label_area_size(35)
        .y_label_area_size(45)
        .build_cartesian_2d(0f64..7f64, 0f64..45f64)?;
    chart
        .configure_mesh()
        .x_labels(8)
        .x_label_formatter(&|x| days[(*x as usize).min(6)].into())
        .y_desc("% of posts")
        .axis_style(FG)
        .label_style(("sans-serif", 17).into_font().color(&FG))
        .axis_desc_style(("sans-serif", 17).into_font().color(&FG))
        .light_line_style(RGBColor(33, 38, 45))
        .draw()?;
    for (i, (name, row)) in d["day_data"].as_object().unwrap().iter().enumerate() {
        let c = color(&data[name]);
        chart.draw_series(normalized(row).iter().enumerate().map(|(day, n)| {
            Rectangle::new(
                [
                    (day as f64 + 0.08 + i as f64 * 0.084, 0.),
                    (day as f64 + 0.15 + i as f64 * 0.084, *n),
                ],
                c.filled(),
            )
        }))?;
        text(
            &pair[0],
            format!("{} {}", i + 1, name),
            30 + (i % 3) as i32 * 390,
            455 + (i / 3) as i32 * 24,
            16,
            c,
        )?;
    }
    bars(
        &pair[1],
        data,
        "VIRAL POTENTIAL: maximum observed upvotes",
        |v| number(v, "max_up"),
    )?;
    title(&rows[4], "HISTORICAL POSTING WINDOWS (Pacific)")?;
    for h in (0..=24).step_by(2) {
        text(&rows[4], format!("{h}:00"), 260 + h * 70, 60, 17, FG)?;
    }
    for (i, (name, row)) in d["best_times"].as_object().unwrap().iter().enumerate() {
        let y = 90 + i as i32 * 27;
        text(&rows[4], name, 24, y, 17, FG)?;
        let start = row[0].as_i64().unwrap() as i32;
        let end = row[1].as_i64().unwrap() as i32;
        rows[4].draw(&Rectangle::new(
            [(260 + 70 * start, y - 9), (260 + 70 * end, y + 9)],
            color(&data[name]).filled(),
        ))?;
        text(
            &rows[4],
            format!("{start}:00-{end}:00 ({})", row[2].as_str().unwrap()),
            280 + 70 * end,
            y,
            17,
            FG,
        )?;
    }
    title(
        &rows[5],
        "ARCHIVED STRATEGY: published planning score is independent of the computed ranking",
    )?;
    let columns = [24, 315, 460, 675, 920, 1740, 2030];
    for (i, row) in d["schedule"].as_array().unwrap().iter().enumerate() {
        let y = 70 + i as i32 * 28;
        let name = row[0].as_str().unwrap();
        for (j, v) in row.as_array().unwrap().iter().enumerate() {
            let s = if i == 0 && j == 5 {
                "Published score"
            } else {
                v.as_str().unwrap()
            };
            text(
                &rows[5],
                s,
                columns[j],
                y,
                18,
                if i == 0 {
                    COLORS[0]
                } else if j == 0 {
                    color(&data[name])
                } else {
                    FG
                },
            )?;
        }
        text(
            &rows[5],
            if i == 0 {
                "Computed score".into()
            } else {
                format!("{:.1}", composite(&data[name]))
            },
            columns[6],
            y,
            18,
            FG,
        )?;
    }
    root.present()?;
    Ok(())
}
pub fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let a = Args::try_parse_from(std::iter::once("reddit-dashboard".into()).chain(args))?;
    let font = a
        .font
        .or_else(|| {
            [
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
                "C:/Windows/Fonts/arial.ttf",
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "/usr/share/fonts/truetype/liberation2/LiberationSans-Regular.ttf",
            ]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
        })
        .ok_or_else(|| {
            anyhow::anyhow!("No font found; pass --font PATH to an OpenType or TrueType font")
        })?;
    // Plotters' process-global font registry requires static bytes; this CLI exits
    // after one render, so retaining this single font matches its registry lifetime.
    plotters::style::register_font(
        "sans-serif",
        FontStyle::Normal,
        Box::leak(std::fs::read(font)?.into_boxed_slice()),
    )
    .map_err(|_| anyhow::anyhow!("Invalid font file"))?;
    let mut data: Value = serde_json::from_str(DATA)?;
    ensure!(
        a.json.as_ref() != Some(&a.output),
        "image and JSON output paths must differ"
    );
    match a.output.extension().and_then(|s| s.to_str()) {
        Some("png") => render(
            BitMapBackend::new(&a.output, (2400, 3600)).into_drawing_area(),
            &data,
        )?,
        Some("svg") => render(
            SVGBackend::new(&a.output, (2400, 3600)).into_drawing_area(),
            &data,
        )?,
        _ => anyhow::bail!("output must have .png or .svg extension"),
    }
    if let Some(path) = a.json {
        let metrics:serde_json::Map<String,Value>=data["data"].as_object().unwrap().iter().map(|(name,v)| {
            let hours=normalized(&data["hour_data"][name]);
            (name.clone(),serde_json::json!({"composite":composite(v),"hour_percent":hours,"peak_hour":peak(&hours),"day_percent":data["day_data"].get(name).map(normalized)}))
        }).collect();
        data["computed"] = metrics.into();
        std::fs::write(path, serde_json::to_vec_pretty(&data)?)?;
    }
    println!("Saved archived dashboard to {}", a.output.display());
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn archived_metrics_and_first_peak() {
        let d: Value = serde_json::from_str(DATA).unwrap();
        assert_eq!(d["data"].as_object().unwrap().len(), 15);
        assert_eq!(composite(&d["data"]["r/ClaudeAI"]), 184.);
        assert_eq!(composite(&d["data"]["r/ChatGPTCoding"]), 864.);
        assert_eq!(peak(&[1., 3., 3.]), Some(1));
        assert_eq!(peak(&[0., 0.]), None);
        for row in d["hour_data"].as_object().unwrap().values() {
            assert_eq!(row.as_array().unwrap().len(), 24);
            assert!((normalized(row).iter().sum::<f64>() - 100.).abs() < 1e-9);
        }
        assert_eq!(normalized(&serde_json::json!([0, 0])), vec![0., 0.]);
    }
}
