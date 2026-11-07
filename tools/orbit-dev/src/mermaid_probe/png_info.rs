use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{fs::File, path::Path};
pub(super) fn inspect(path: &Path) -> Result<Value> {
    let mut result = json!({"path":path,"exists":path.exists()});
    if !path.exists() {
        return Ok(result);
    }
    let mut decoder = png::Decoder::new(File::open(path)?);
    decoder.set_limits(png::Limits {
        bytes: 256 * 1024 * 1024,
    });
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let size = reader.output_buffer_size();
    ensure!(
        size <= 256 * 1024 * 1024,
        "PNG exceeds diagnostic memory limit"
    );
    let mut bytes = vec![0; size];
    let info = reader.next_frame(&mut bytes)?;
    ensure!(
        info.bit_depth == png::BitDepth::Eight,
        "unexpected expanded PNG depth"
    );
    let samples = info.color_type.samples();
    let alpha = matches!(
        info.color_type,
        png::ColorType::Rgba | png::ColorType::GrayscaleAlpha
    );
    let mut left = info.width;
    let mut top = info.height;
    let mut right = 0;
    let mut bottom = 0;
    for (y, row) in bytes[..info.buffer_size()]
        .chunks(info.line_size)
        .enumerate()
    {
        for (x, pixel) in row.chunks_exact(samples).enumerate() {
            if !alpha || pixel[samples - 1] != 0 {
                left = left.min(x as u32);
                right = right.max(x as u32 + 1);
                top = top.min(y as u32);
                bottom = bottom.max(y as u32 + 1);
            }
        }
    }
    result["size"] = json!([info.width, info.height]);
    result["alpha_bbox"] = Value::Null;
    if right > left && bottom > top {
        result["alpha_bbox"] = json!([left, top, right, bottom]);
        result["content_size"] = json!([right - left, bottom - top]);
        result["transparent_margins"] =
            json!({"left":left,"top":top,"right":info.width-right,"bottom":info.height-bottom});
    }
    Ok(result)
}
