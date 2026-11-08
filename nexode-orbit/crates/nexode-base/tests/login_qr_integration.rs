//! Exercise the real OpenAI login prompt in a child process, without completing
//! OAuth, opening a browser, or touching saved credentials.
use std::process::{Command, Stdio};

/// Same wiring as the CLI's terminal presenter.
struct TerminalPresenter;

impl nexode_base::auth::login_ui::LoginPresenter for TerminalPresenter {
    fn qr_section(
        &self,
        data: &str,
        heading: &str,
        indent: &str,
        browser_suppressed: bool,
    ) -> Option<String> {
        nexode_terminal_prompt::qr_indented_section(data, heading, indent, browser_suppressed)
    }

    fn read_secret_line(&self) -> anyhow::Result<String> {
        nexode_terminal_prompt::read_secret_line()
    }
}

#[test]
fn openai_browserless_login_prints_qr() {
    if std::env::var_os("NEXODE_QR_LOGIN_TEST_CHILD").is_some() {
        nexode_base::auth::login_ui::install(Box::new(TerminalPresenter));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(nexode_base::auth::oauth::login_openai(true))
            .unwrap_err();
        assert!(error.to_string().contains("No callback URL entered"));
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "openai_browserless_login_prints_qr",
            "--nocapture",
        ])
        .env("NEXODE_QR_LOGIN_TEST_CHILD", "1")
        .env("NEXODE_HOME", home.path())
        .env_remove("NEXODE_SHOW_LOGIN_QR")
        .env_remove("NEXODE_LOGIN_QR")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(stderr.contains("https://auth.openai.com/"), "{stderr}");
    assert!(stderr.contains("Scan this QR"), "{stderr}");
    assert!(stderr.contains('█') || stderr.contains('▀') || stderr.contains('▄'));
    assert!(stderr.contains("Paste the full callback URL"), "{stderr}");
    assert!(!stderr.contains("Waiting up to"), "{stderr}");
}
