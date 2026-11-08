#![cfg(target_os = "linux")]
mod support;
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn isolated_maki_fixture_measures_application_echo_and_reaps_every_client() {
    let d = tempfile::tempdir().unwrap();
    let binary = d.path().join("maki");
    support::executable(
        &binary,
        r#"#!/bin/bash
if [[ $1 == --version ]]; then printf 'maki-fixture\n'; exit; fi
[[ -z ${REAL_PROVIDER_SECRET:-} ]] || exit 2
[[ $ANTHROPIC_API_KEY == sk-ant-benchmark-placeholder-not-valid ]] || exit 3
[[ -f "$XDG_CONFIG_HOME/maki/init.lua" ]] || exit 4
[[ $* != *--no-plugins* && $* == *--no-commands* ]] || exit 5
printf 'fixture prompt ❯ '
while IFS= read -r -n1 c; do printf '%s' "$c"; done
"#,
    );
    let out = d.path().join("result.json");
    let result = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["bench-nexode-maki", "--tools", "maki", "--maki"])
        .arg(&binary)
        .args([
            "--maki-no-splash",
            "--runs",
            "1",
            "--sessions",
            "1",
            "--idle-seconds",
            "0.01",
            "--output",
        ])
        .arg(&out)
        .env("REAL_PROVIDER_SECRET", "must-not-be-inherited")
        .output()
        .unwrap();
    support::success(result);
    let v: Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();
    let row = &v["runs"][0];
    assert_eq!(row["processes"], 1);
    assert_eq!(row["input_echo_samples_ms"].as_array().unwrap().len(), 20);
    assert!(row["startup"][0]["first_visible_ms"].is_number());
    assert!(row["pss_mib"].as_f64().unwrap() > 0.0);
    assert!(row["settled_screen"].as_str().unwrap().contains("q19z"));
    for pid in row["executables"].as_object().unwrap().keys() {
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "owned benchmark process survived cleanup"
        );
    }
}
