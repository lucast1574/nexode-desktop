use std::{fs, process::Command};
#[test]
fn native_dashboard_exports_archive_and_both_image_formats() {
    let dir = tempfile::tempdir().unwrap();
    for extension in ["png", "svg"] {
        let path = dir.path().join(format!("report.{extension}"));
        let json = dir.path().join("report.json");
        let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .arg("reddit-dashboard")
            .arg("--output")
            .arg(&path)
            .arg("--json")
            .arg(&json)
            .env("PATH", "")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let bytes = fs::read(&path).unwrap();
        if extension == "png" {
            let decoder = png::Decoder::new(bytes.as_slice());
            let reader = decoder.read_info().unwrap();
            assert_eq!((reader.info().width, reader.info().height), (2400, 3600));
        } else {
            let svg = String::from_utf8(bytes).unwrap();
            let document = roxmltree::Document::parse(&svg).unwrap();
            assert_eq!(document.root_element().tag_name().name(), "svg");
            for panel in [
                "OVERALL RANKING",
                "SUBSCRIBERS vs ENGAGEMENT",
                "DISCUSSION DEPTH",
                "POSTING ACTIVITY",
                "DAY OF WEEK",
                "VIRAL POTENTIAL",
                "HISTORICAL POSTING",
                "ARCHIVED STRATEGY",
            ] {
                assert!(svg.contains(panel));
            }
        }
        let report: serde_json::Value = serde_json::from_slice(&fs::read(json).unwrap()).unwrap();
        let original: serde_json::Value =
            serde_json::from_str(include_str!("../src/reddit_data.json")).unwrap();
        for (key, value) in original.as_object().unwrap() {
            assert_eq!(&report[key], value);
        }
        assert_eq!(report["computed"]["r/ClaudeAI"]["peak_hour"], 6);
        assert_eq!(report["computed"]["r/ClaudeAI"]["composite"], 184.);
        assert!(report["computed"]["r/ClaudeAI"]["day_percent"].is_null());
    }
    let path = dir.path().join("invalid.txt");
    assert!(
        !Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
            .args(["reddit-dashboard", "--output"])
            .arg(&path)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!path.exists());
}
