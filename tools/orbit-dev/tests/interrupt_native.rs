//! Optional real-runtime contract test; the loopback provider never uses real auth.
use std::process::Command;
#[test]
fn real_streams_tools_and_urgent_interrupts() {
    let Some(binary) = std::env::var_os("ORBIT_TEST_NEXODE_BINARY") else {
        eprintln!("Set ORBIT_TEST_NEXODE_BINARY to run real interrupt acceptance");
        return;
    };
    let out = Command::new(env!("CARGO_BIN_EXE_orbit-dev"))
        .args(["test-soft-interrupt", "--binary"])
        .arg(binary)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let cases = report["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    assert!(cases.iter().all(|r| r["passed"] == true));
    let urgent = cases.iter().find(|r| r["case"] == "urgent-tools").unwrap();
    assert_eq!(urgent["tool_calls"], 3);
}
