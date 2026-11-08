use super::*;
#[test]
fn clipped_screenshot_reproduction_and_content_centering() {
    let p = plan(73, 46, 1180, 1470, 8, 16);
    assert_eq!(p["fit_zoom_percent"], 49);
    assert_eq!(p["old_buggy_plan"]["mode"], "fit-fill@70%");
    assert_eq!(p["fixed_plan"]["mode"], "fit");
    assert_eq!(p["repro_was_clipping_bug"], true);
    let p = plan(20, 20, 10, 10, 1, 1);
    assert_eq!(metrics(&p, [4, 4, 6, 6]).unwrap()["camera_centered"], true);
    assert_eq!(metrics(&p, [0, 0, 2, 2]).unwrap()["camera_centered"], false);
    assert_eq!(plan(0, 0, 0, 0, 0, 0)["fit_zoom_percent"], 100);
    assert_eq!(scroll(1000, 10, 100, 10), 45);
    assert!(pair("1x-2").is_err());
}
#[test]
fn native_png_alpha_bounds_ignore_invisible_rgb_and_handle_missing_file() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("sample.png");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = png::Encoder::new(file, 4, 4);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    let mut bytes = vec![0; 4 * 4 * 4];
    bytes[0] = 255;
    for y in 1..3 {
        for x in 1..3 {
            bytes[(y * 4 + x) * 4 + 3] = 255;
        }
    }
    writer.write_image_data(&bytes).unwrap();
    drop(writer);
    let v = png_info::inspect(&path).unwrap();
    assert_eq!(v["alpha_bbox"], json!([1, 1, 3, 3]));
    assert_eq!(v["content_size"], json!([2, 2]));
    assert_eq!(v["transparent_margins"]["right"], 1);
    assert_eq!(
        png_info::inspect(&tmp.path().join("missing.png")).unwrap()["exists"],
        false
    );
}
