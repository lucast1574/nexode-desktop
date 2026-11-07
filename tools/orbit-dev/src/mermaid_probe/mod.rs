//! Standalone reproduction of Mermaid fit geometry and visible-alpha placement.
mod png_info;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use serde_json::{Value, json};
use std::path::PathBuf;
fn utilization(used: f64, total: f64) -> f64 {
    if total == 0.0 {
        0.0
    } else {
        used * 100.0 / total
    }
}
fn axis(c: u64, px: u64, font: u64) -> u64 {
    if c == 0 || px == 0 || font == 0 {
        100
    } else {
        (c * font * 100 / px).clamp(1, 1000)
    }
}
fn scroll(image: u64, area: u64, zoom: u64, cell: u64) -> u64 {
    if image == 0 || area == 0 || zoom == 0 || cell == 0 {
        return 0;
    }
    let visible = area * cell * 100 / zoom;
    let excess = image.saturating_sub(visible);
    (excess / 2) / ((cell * 100).div_ceil(zoom)).max(1)
}
fn plan(w: u64, h: u64, iw: u64, ih: u64, fw: u64, fh: u64) -> Value {
    let empty = w == 0 || h == 0 || iw == 0 || ih == 0;
    let scale = if empty {
        1.0
    } else {
        (w as f64 * fw.max(1) as f64 / iw as f64).min(h as f64 * fh.max(1) as f64 / ih as f64)
    };
    let cw = if empty {
        w
    } else {
        ((iw as f64 * scale / fw.max(1) as f64).ceil() as u64).min(w)
    };
    let ch = if empty {
        h
    } else {
        ((ih as f64 * scale / fh.max(1) as f64).ceil() as u64).min(h)
    };
    let rect = json!({"x":(w-cw)/2,"y":(h-ch)/2,"width":cw,"height":ch});
    let fit = if empty {
        100
    } else {
        (w * fw.max(1) * 100 / iw)
            .min(h * fh.max(1) * 100 / ih)
            .clamp(1, 1000)
    };
    let zw = axis(w, iw, fw);
    let zh = axis(h, ih, fh);
    let preferred = zw.max(zh).clamp(70, 1000);
    let uw = utilization(cw as f64, w as f64);
    let uh = utilization(ch as f64, h as f64);
    let ua = utilization((cw * ch) as f64, (w * h) as f64);
    let under = uw < 85.0 || uh < 85.0 || ua < 85.0;
    let larger = preferred > fit + 5;
    let old = (fit < 70 || under) && larger;
    let fixed = under && larger;
    let fill = json!({"mode":format!("fit-fill@{preferred}%"),"zoom_percent":preferred,"scroll_x":scroll(iw,w,preferred,fw),"scroll_y":scroll(ih,h,preferred,fh)});
    let contain = json!({"mode":"fit","rect":rect});
    json!({"input":{"inner_width_cells":w,"inner_height_cells":h,"image_width_px":iw,"image_height_px":ih,"font_width_px":fw,"font_height_px":fh},"contain_rect_cells":rect,"utilization_percent":{"width":uw,"height":uh,"area":ua},"fit_zoom_percent":fit,"axis_fill_zoom_percent":{"width":zw,"height":zh},"preferred_fill_zoom_percent":preferred,"underutilized":under,"meaningfully_larger":larger,"old_buggy_plan":if old{&fill}else{&contain},"fixed_plan":if fixed{&fill}else{&contain},"repro_was_clipping_bug":old&&!fixed})
}
fn metrics(plan: &Value, bbox: [u64; 4]) -> Option<Value> {
    let i = &plan["input"];
    let w = i["inner_width_cells"].as_u64()? as f64;
    let h = i["inner_height_cells"].as_u64()? as f64;
    let iw = i["image_width_px"].as_u64()?;
    let ih = i["image_height_px"].as_u64()?;
    if iw == 0 || ih == 0 || i["font_width_px"] == 0 || i["font_height_px"] == 0 {
        return None;
    }
    let rect = &plan["contain_rect_cells"];
    let x = rect["x"].as_u64()? as f64;
    let y = rect["y"].as_u64()? as f64;
    let sx = rect["width"].as_u64()? as f64 / iw as f64;
    let sy = rect["height"].as_u64()? as f64 / ih as f64;
    let left = x + bbox[0] as f64 * sx;
    let top = y + bbox[1] as f64 * sy;
    let right = x + bbox[2] as f64 * sx;
    let bottom = y + bbox[3] as f64 * sy;
    let width = (right - left).max(0.0);
    let height = (bottom - top).max(0.0);
    let ox = (left + right) / 2.0 - w / 2.0;
    let oy = (top + bottom) / 2.0 - h / 2.0;
    Some(
        json!({"content_bbox_cells":{"left":left,"top":top,"right":right,"bottom":bottom,"width":width,"height":height},"blank_cells":{"left":left,"right":w-right,"top":top,"bottom":h-bottom},"blank_imbalance_cells":{"horizontal_right_minus_left":w-right-left,"vertical_bottom_minus_top":h-bottom-top},"content_center_offset_cells":{"x":ox,"y":oy},"content_area_utilization_percent":utilization((width*height*1000.0).round_ties_even(),w*h*1000.0),"camera_centered":ox.abs()<=1.0&&oy.abs()<=1.0,"note":"Low content_area_utilization can be normal for a wide/short or tall/narrow diagram. A large content_center_offset or top/bottom blank imbalance means the camera/placement is wrong."}),
    )
}
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "73x46")]
    inner: String,
    #[arg(long, default_value = "1180x1470")]
    image: String,
    #[arg(long, default_value = "8x16")]
    font: String,
    #[arg(long)]
    png: Option<PathBuf>,
    #[arg(long)]
    alpha_bbox: Option<String>,
    #[arg(long)]
    assert_camera_centered: bool,
    #[arg(long)]
    assert_old_clipping_repro: bool,
}
fn pair(s: &str) -> Result<(u64, u64)> {
    let lower = s.to_lowercase();
    let (a, b) = lower.split_once('x').context("expected WIDTHxHEIGHT")?;
    let a = a.parse()?;
    let b = b.parse()?;
    ensure!(
        a <= 1_000_000 && b <= 1_000_000,
        "dimensions exceed diagnostic limit"
    );
    Ok((a, b))
}
pub fn run(args: impl Iterator<Item = String>) -> Result<i32> {
    let a = Args::parse_from(std::iter::once("mermaid-fit-probe".into()).chain(args));
    let (w, h) = pair(&a.inner)?;
    let (iw, ih) = pair(&a.image)?;
    let (fw, fh) = pair(&a.font)?;
    let mut result = plan(w, h, iw, ih, fw, fh);
    if let Some(path) = a.png {
        let path = if let Ok(rest) = path.strip_prefix("~") {
            dirs::home_dir().context("home unavailable")?.join(rest)
        } else {
            path
        };
        result["png"] = png_info::inspect(&path)?;
    }
    let bbox = if let Some(raw) = a.alpha_bbox {
        let parts = raw
            .split(',')
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(parts.len() == 4, "bbox requires LEFT,TOP,RIGHT,BOTTOM");
        Some([parts[0], parts[1], parts[2], parts[3]])
    } else {
        result["png"]["alpha_bbox"].as_array().and_then(|b| {
            Some([
                b.first()?.as_u64()?,
                b.get(1)?.as_u64()?,
                b.get(2)?.as_u64()?,
                b.get(3)?.as_u64()?,
            ])
        })
    };
    if let Some(b) = bbox {
        ensure!(
            b[0] <= b[2] && b[1] <= b[3] && b[2] <= iw && b[3] <= ih,
            "alpha bbox outside image dimensions"
        );
        if let Some(m) = metrics(&result, b) {
            result["visual_fit_metrics"] = m;
        }
    }
    println!("{}", serde_json::to_string_pretty(&result)?);
    if a.assert_camera_centered {
        ensure!(
            !result["visual_fit_metrics"].is_null(),
            "camera assertion needs visible alpha bounds and positive dimensions"
        );
        if result["visual_fit_metrics"]["camera_centered"] != true {
            return Ok(2);
        }
    }
    if a.assert_old_clipping_repro && result["repro_was_clipping_bug"] != true {
        return Ok(3);
    }
    Ok(0)
}
#[cfg(test)]
mod tests;
