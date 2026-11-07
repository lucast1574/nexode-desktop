//! Mapping from parsed CLI arguments to an initial process title.
//!
//! This logic depends on the clap `Args`/`Command` types defined in `cli`, so
//! it lives in the CLI layer. The low-level title-setting primitives it uses
//! (`compact_process_title`, `session_name`, `set_title`) live in the
//! `process_title` core module.

use crate::cli::args::{AmbientCommand, Args, Command};
use crate::process_title::{compact_process_title, session_name, set_title};

pub(crate) fn initial_title(args: &Args) -> String {
    match &args.command {
        Some(Command::Serve { .. }) => "nexode:server".to_string(),
        Some(Command::Acp) => "nexode acp".to_string(),
        Some(Command::Server { .. }) => "nexode server".to_string(),
        Some(Command::Connect) => "nexode:client".to_string(),
        #[cfg(unix)]
        Some(Command::ApiBridge { .. }) => "nexode api-bridge".to_string(),
        Some(Command::Run { .. }) => "nexode run".to_string(),
        Some(Command::Login { .. }) => "nexode login".to_string(),
        Some(Command::Account { .. }) => "nexode account".to_string(),
        Some(Command::Mobile { .. }) => "nexode mobile".to_string(),
        Some(Command::Mcp { .. }) => "nexode mcp".to_string(),
        Some(Command::Repl) => "nexode repl".to_string(),
        Some(Command::Update { .. }) => "nexode update".to_string(),
        Some(Command::Version { .. }) => "nexode version".to_string(),
        Some(Command::Usage { .. }) => "nexode usage".to_string(),
        Some(Command::Telemetry(_)) => "nexode telemetry".to_string(),
        Some(Command::SelfDev { .. }) => "nexode:selfdev".to_string(),
        Some(Command::Debug { .. }) => "nexode debug".to_string(),
        Some(Command::Auth(_)) => "nexode auth".to_string(),
        Some(Command::Provider(_)) => "nexode provider".to_string(),
        Some(Command::Memory(_)) => "nexode memory".to_string(),
        Some(Command::Session(_)) => "nexode session".to_string(),
        Some(Command::Ambient(subcommand)) => match subcommand {
            AmbientCommand::RunVisible => "nexode ambient visible".to_string(),
            _ => "nexode ambient".to_string(),
        },
        Some(Command::Cloud(_)) => "nexode cloud".to_string(),
        Some(Command::Pair { .. }) => "nexode pair".to_string(),
        Some(Command::Permissions) => "nexode permissions".to_string(),
        Some(Command::Transcript { .. }) => "nexode transcript".to_string(),
        Some(Command::Companion { .. }) => "nexode companion".to_string(),
        Some(Command::Dictate { .. }) => "nexode dictate".to_string(),
        Some(Command::SetupHotkey {
            listen_macos_hotkey,
            notify_cli_launch,
            listen_windows_hotkey,
            uninstall,
        }) => {
            if *listen_macos_hotkey || *listen_windows_hotkey {
                "nexode hotkey listener".to_string()
            } else if notify_cli_launch.is_some() {
                "nexode shortcut reminder".to_string()
            } else if *uninstall {
                "nexode hotkey uninstall".to_string()
            } else {
                "nexode hotkey setup".to_string()
            }
        }
        Some(Command::Browser { .. }) => "nexode browser".to_string(),
        Some(Command::Replay { .. }) => "nexode replay".to_string(),
        Some(Command::Model(_)) => "nexode model".to_string(),
        Some(Command::ProviderTestCoverage { .. }) => "nexode provider-test-coverage".to_string(),
        Some(Command::ProviderDoctor { .. }) => "nexode provider-doctor".to_string(),
        Some(Command::AuthTest { .. }) => "nexode auth-test".to_string(),
        Some(Command::Restart { .. }) => "nexode restart".to_string(),
        Some(Command::Menubar { .. }) => "nexode menubar".to_string(),
        Some(Command::SetupLauncher) => "nexode setup-launcher".to_string(),
        None => {
            if let Some(resume) = args.resume.as_deref().filter(|resume| !resume.is_empty()) {
                let prefix = if crate::cli::selfdev::client_selfdev_requested() {
                    "nexode:d:"
                } else {
                    "nexode:c:"
                };
                compact_process_title(prefix, Some(&session_name(resume)))
            } else if crate::cli::selfdev::client_selfdev_requested() {
                "nexode:selfdev".to_string()
            } else {
                "nexode:client".to_string()
            }
        }
    }
}

pub(crate) fn set_initial_title(args: &Args) {
    set_title(initial_title(args));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::lock_test_env;
    use clap::Parser;

    const SELFDEV_ENV: &str = nexode_selfdev_types::CLIENT_SELFDEV_ENV;

    fn with_selfdev_env_removed<T>(f: impl FnOnce() -> T) -> T {
        let _guard = lock_test_env();
        let previous = std::env::var_os(SELFDEV_ENV);
        crate::env::remove_var(SELFDEV_ENV);
        let result = f();
        if let Some(value) = previous {
            crate::env::set_var(SELFDEV_ENV, value);
        }
        result
    }

    #[test]
    fn initial_title_labels_server() {
        with_selfdev_env_removed(|| {
            let args = Args::parse_from(["nexode", "serve"]);
            assert_eq!(initial_title(&args), "nexode:server");
        });
    }

    #[test]
    fn initial_title_labels_resume_client_with_short_name() {
        with_selfdev_env_removed(|| {
            let args = Args::parse_from(["nexode", "--resume", "session_fox_123"]);
            assert_eq!(initial_title(&args), "nexode:c:fox");
        });
    }

    #[test]
    fn initial_title_labels_selfdev_command() {
        with_selfdev_env_removed(|| {
            let args = Args::parse_from(["nexode", "self-dev"]);
            assert_eq!(initial_title(&args), "nexode:selfdev");
        });
    }

    #[test]
    fn initial_title_labels_windows_hotkey_listener() {
        let args = Args::parse_from(["nexode", "setup-hotkey", "--listen-windows-hotkey"]);
        assert_eq!(initial_title(&args), "nexode hotkey listener");
    }

    #[test]
    fn initial_title_labels_hotkey_uninstall() {
        let args = Args::parse_from(["nexode", "setup-hotkey", "--uninstall"]);
        assert_eq!(initial_title(&args), "nexode hotkey uninstall");
    }
}
