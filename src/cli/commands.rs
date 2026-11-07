#![cfg_attr(test, allow(clippy::await_holding_lock))]

use anyhow::Result;
use serde::Serialize;
use std::io::{Read, Write};
use std::net::ToSocketAddrs;

use crate::{browser, gateway, memory, session, storage, tui};

use super::{output::terminal_title, terminal::init_tui_runtime};

mod menubar;
mod provider_setup;
mod report_info;
mod restart;

pub(crate) use super::auth_test::run_post_login_validation;
#[cfg(test)]
pub(crate) use super::auth_test::{
    AuthTestChoicePlan, AuthTestTarget, ResolvedAuthTestTarget, auth_test_choice_plan,
    auth_test_error_is_retryable, configured_auth_test_targets, resolve_auth_test_targets,
};
pub use super::auth_test::{
    run_auth_test_command, run_auth_test_context_audit_command, run_auth_test_coverage_command,
};
pub use menubar::{ensure_menubar_helper_running, run_menubar_command};
pub(crate) use provider_setup::{ProviderAddOptions, run_provider_add_command};
pub use restart::{
    maybe_run_pending_restart_restore_on_startup, run_restart_clear_command,
    run_restart_restore_command, run_restart_save_command, run_restart_status_command,
};

pub enum AmbientSubcommand {
    Status,
    Log,
    Trigger,
    Stop,
    RunVisible,
}

pub async fn run_ambient_command(cmd: AmbientSubcommand) -> Result<()> {
    if let AmbientSubcommand::RunVisible = cmd {
        return run_ambient_visible().await;
    }

    let debug_cmd = match cmd {
        AmbientSubcommand::Status => "ambient:status",
        AmbientSubcommand::Log => "ambient:log",
        AmbientSubcommand::Trigger => "ambient:trigger",
        AmbientSubcommand::Stop => "ambient:stop",
        AmbientSubcommand::RunVisible => unreachable!(),
    };

    super::debug::run_debug_command(debug_cmd, "", None, None, false).await
}

pub async fn run_transcript_command(
    text: Option<String>,
    mode: crate::protocol::TranscriptMode,
    session: Option<String>,
) -> Result<()> {
    let text = if let Some(text) = text {
        text
    } else {
        let mut stdin = String::new();
        std::io::stdin().read_to_string(&mut stdin)?;
        let trimmed = stdin.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            anyhow::bail!("Provide transcript text as an argument or pipe it via stdin")
        }
        trimmed.to_string()
    };

    // Focused-window lookup needs the user's compositor, so it happens here in
    // the client rather than in the (possibly remote) server.
    let session = session.or_else(|| {
        nexode_voice::dictation::focused_nexode_session()
            .ok()
            .flatten()
    });
    let mut client = crate::server::Client::connect_debug().await?;
    let request_id = client.send_transcript(&text, mode, session).await?;

    loop {
        match unwrap_turn_event(client.read_event().await?) {
            crate::protocol::ServerEvent::Ack { id } if id == request_id => {}
            crate::protocol::ServerEvent::Done { id } if id == request_id => return Ok(()),
            crate::protocol::ServerEvent::Error { id, message, .. } if id == request_id => {
                anyhow::bail!(message)
            }
            _ => {}
        }
    }
}

pub async fn run_companion_command(status: bool, no_voice: bool) -> Result<()> {
    use crate::companion_client::{CompanionRequest, request_async};
    if status {
        match request_async(CompanionRequest::Status, std::time::Duration::from_secs(2)).await {
            Ok(response) => {
                println!(
                    "{} at {} (capabilities: {})",
                    response.message,
                    crate::companion_client::socket_path().display(),
                    response.capabilities.join(", ")
                );
                return Ok(());
            }
            Err(error) => anyhow::bail!("{error}"),
        }
    }
    nexode_companion::run(nexode_companion::CompanionOptions {
        voice: !no_voice,
        ..Default::default()
    })
    .await
}

pub async fn run_dictate_command(type_output: bool) -> Result<()> {
    let run = nexode_voice::dictation::run_configured().await?;

    if type_output {
        nexode_voice::dictation::type_text(&run.text)
    } else {
        run_transcript_command(Some(run.text), run.mode, None).await
    }
}

#[derive(Serialize)]
struct SessionRenameOutput {
    session_id: String,
    display_name: String,
    title: Option<String>,
    cleared: bool,
}

pub fn run_session_rename_command(
    session_ref: &str,
    name: Option<&str>,
    clear: bool,
    json: bool,
) -> Result<()> {
    let resolved_id = session::find_session_by_name_or_id(session_ref)?;
    let mut session = session::Session::load(&resolved_id)?;

    if clear {
        session.rename_title(None);
    } else {
        let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) else {
            anyhow::bail!("Provide a session name or use --clear");
        };
        session.rename_title(Some(name.to_string()));
    }

    session.save()?;
    crate::tui::session_picker::invalidate_session_list_cache();

    let output = SessionRenameOutput {
        session_id: session.id.clone(),
        display_name: session.display_name().to_string(),
        title: session.display_title().map(ToOwned::to_owned),
        cleared: clear,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if clear {
        println!(
            "Cleared custom name for session {} ({}).",
            output.display_name, output.session_id
        );
    } else if let Some(title) = output.title.as_deref() {
        println!(
            "Renamed session {} ({}) to \"{}\".",
            output.display_name, output.session_id, title
        );
    }

    Ok(())
}

async fn run_ambient_visible() -> Result<()> {
    use crate::ambient::VisibleCycleContext;

    let context = VisibleCycleContext::load().map_err(|e| {
        anyhow::anyhow!(
            "Failed to load visible cycle context: {}\nIs the ambient runner running?",
            e
        )
    })?;

    let (provider, registry) = super::provider_init::init_provider_and_registry(
        &super::provider_init::ProviderChoice::Auto,
        None,
    )
    .await?;

    registry.register_ambient_tools().await;

    let safety = std::sync::Arc::new(crate::safety::SafetySystem::new());
    crate::tool::ambient::init_safety_system(safety);

    let (terminal, tui_runtime) = init_tui_runtime()?;

    let mut app = tui::App::new(provider, registry);
    app.set_ambient_mode(context.system_prompt, context.initial_message);

    let _ = crossterm::execute!(
        std::io::stdout(),
        crossterm::terminal::SetTitle(terminal_title("🤖 nexode ambient cycle"))
    );

    let result = app.run(terminal).await;

    tui_runtime.finish(true);

    if let Some(cycle_result) = crate::tool::ambient::take_cycle_result() {
        let result_path = VisibleCycleContext::result_path()?;
        crate::storage::write_json(&result_path, &cycle_result)?;
        eprintln!("Ambient cycle result saved.");
    }

    result?;
    Ok(())
}

pub enum MemorySubcommand {
    List {
        scope: String,
        tag: Option<String>,
    },
    Search {
        query: String,
        semantic: bool,
    },
    Export {
        output: String,
        scope: String,
    },
    Import {
        input: String,
        scope: String,
        overwrite: bool,
    },
    Stats,
    ClearTest,
}

pub async fn run_memory_command(cmd: MemorySubcommand) -> Result<()> {
    run_memory_command_for_dir(cmd, std::env::current_dir().ok()).await
}

async fn run_memory_command_for_dir(
    cmd: MemorySubcommand,
    project_dir: Option<std::path::PathBuf>,
) -> Result<()> {
    use memory::{MemoryEntry, MemoryManager};

    let project_dir = project_dir.filter(|dir| !dir.as_os_str().is_empty());
    // Match agent-side memory resolution: project memory is available only when
    // the caller supplied a concrete working directory.
    let manager = match project_dir.as_ref() {
        Some(dir) => MemoryManager::new().with_project_dir(dir),
        _ => MemoryManager::new(),
    };

    match cmd {
        MemorySubcommand::List { scope, tag } => {
            let mut all_memories: Vec<MemoryEntry> = Vec::new();

            if (scope == "all" || scope == "project")
                && let Ok(graph) = manager.load_project_graph()
            {
                all_memories.extend(graph.all_memories().cloned());
            }
            if (scope == "all" || scope == "global")
                && let Ok(graph) = manager.load_global_graph()
            {
                all_memories.extend(graph.all_memories().cloned());
            }

            if let Some(tag_filter) = tag {
                all_memories.retain(|m| m.tags.contains(&tag_filter));
            }

            all_memories.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

            if all_memories.is_empty() {
                println!("No memories found.");
            } else {
                println!("Found {} memories:\n", all_memories.len());
                for entry in &all_memories {
                    let tags_str = if entry.tags.is_empty() {
                        String::new()
                    } else {
                        format!(" [{}]", entry.tags.join(", "))
                    };
                    let conf = entry.effective_confidence();
                    println!(
                        "- [{}] {}{}\n  id: {} (conf: {:.0}%, accessed: {}x)",
                        entry.category,
                        entry.content,
                        tags_str,
                        entry.id,
                        conf * 100.0,
                        entry.access_count
                    );
                    println!();
                }
            }
        }

        MemorySubcommand::Search { query, semantic } => {
            if semantic {
                match crate::memory_jev::recall(&manager, &query, 20, memory::MemoryScope::All)
                    .await
                {
                    Ok(results) => {
                        if results.is_empty() {
                            println!("No memories found matching '{}'", query);
                        } else {
                            println!(
                                "Found {} memories matching '{}' (Jev relevance):\n",
                                results.len(),
                                query
                            );
                            for (entry, score) in results {
                                let tags_str = if entry.tags.is_empty() {
                                    String::new()
                                } else {
                                    format!(" [{}]", entry.tags.join(", "))
                                };
                                println!(
                                    "- [{}] {}{}\n  id: {} (relevance: {:.0}%)",
                                    entry.category,
                                    entry.content,
                                    tags_str,
                                    entry.id,
                                    score * 100.0
                                );
                                println!();
                            }
                        }
                    }
                    Err(e) => {
                        // A credential or transport failure is not an empty
                        // result and must give scripts a nonzero exit status.
                        return Err(e.context("Jev memory search failed"));
                    }
                }
            } else {
                match manager.search(&query) {
                    Ok(results) => {
                        if results.is_empty() {
                            println!("No memories found matching '{}'", query);
                        } else {
                            println!(
                                "Found {} memories matching '{}' (keyword):\n",
                                results.len(),
                                query
                            );
                            for entry in results {
                                let tags_str = if entry.tags.is_empty() {
                                    String::new()
                                } else {
                                    format!(" [{}]", entry.tags.join(", "))
                                };
                                println!(
                                    "- [{}] {}{}\n  id: {}",
                                    entry.category, entry.content, tags_str, entry.id
                                );
                                println!();
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Search failed: {}", e);
                    }
                }
            }
        }

        MemorySubcommand::Export { output, scope } => {
            let mut all_memories: Vec<memory::MemoryEntry> = Vec::new();

            if (scope == "all" || scope == "project")
                && let Ok(graph) = manager.load_project_graph()
            {
                all_memories.extend(graph.all_memories().cloned());
            }
            if (scope == "all" || scope == "global")
                && let Ok(graph) = manager.load_global_graph()
            {
                all_memories.extend(graph.all_memories().cloned());
            }

            let json = serde_json::to_string_pretty(&all_memories)?;
            std::fs::write(&output, json)?;
            println!("Exported {} memories to {}", all_memories.len(), output);
        }

        MemorySubcommand::Import {
            input,
            scope,
            overwrite,
        } => {
            if scope != "global" && project_dir.is_none() {
                anyhow::bail!("cannot import project memories without a project directory");
            }
            let content = std::fs::read_to_string(&input)?;
            let memories: Vec<memory::MemoryEntry> = serde_json::from_str(&content)?;

            let mut imported = 0;
            let mut skipped = 0;

            for entry in memories {
                let entry_id = entry.id.clone();
                let result = if scope == "global" {
                    if !overwrite
                        && let Ok(graph) = manager.load_global_graph()
                        && graph.get_memory(&entry.id).is_some()
                    {
                        skipped += 1;
                        continue;
                    }
                    manager.remember_global(entry)
                } else {
                    if !overwrite
                        && let Ok(graph) = manager.load_project_graph()
                        && graph.get_memory(&entry.id).is_some()
                    {
                        skipped += 1;
                        continue;
                    }
                    manager.remember_project(entry)
                };

                result.map_err(|error| {
                    anyhow::anyhow!(
                        "failed to durably import memory {entry_id} into {scope} scope: {error}"
                    )
                })?;
                imported += 1;
            }

            println!("Imported {} memories ({} skipped)", imported, skipped);
        }

        MemorySubcommand::Stats => {
            let mut project_count = 0;
            let mut global_count = 0;
            let mut total_tags = std::collections::HashSet::new();
            let mut categories: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();

            if let Ok(graph) = manager.load_project_graph() {
                project_count = graph.memory_count();
                for entry in graph.all_memories() {
                    for tag in &entry.tags {
                        total_tags.insert(tag.clone());
                    }
                    *categories.entry(entry.category.to_string()).or_default() += 1;
                }
            }

            if let Ok(graph) = manager.load_global_graph() {
                global_count = graph.memory_count();
                for entry in graph.all_memories() {
                    for tag in &entry.tags {
                        total_tags.insert(tag.clone());
                    }
                    *categories.entry(entry.category.to_string()).or_default() += 1;
                }
            }

            println!("Memory Statistics:");
            println!("  Project memories: {}", project_count);
            println!("  Global memories:  {}", global_count);
            println!("  Total:            {}", project_count + global_count);
            println!("  Unique tags:      {}", total_tags.len());
            println!("\nBy category:");
            for (cat, count) in &categories {
                println!("  {}: {}", cat, count);
            }
        }

        MemorySubcommand::ClearTest => {
            let test_dir = storage::nexode_dir()?.join("memory").join("test");
            if test_dir.exists() {
                let count = std::fs::read_dir(&test_dir)?.count();
                std::fs::remove_dir_all(&test_dir)?;
                println!("Cleared test memory storage ({} files)", count);
            } else {
                println!("Test memory storage is already empty");
            }
        }
    }

    Ok(())
}

pub fn run_pair_command(list: bool, revoke: Option<String>) -> Result<()> {
    let mut registry = gateway::DeviceRegistry::load();

    if list {
        if registry.devices.is_empty() {
            eprintln!("No paired devices.");
        } else {
            eprintln!("\x1b[1mPaired devices:\x1b[0m\n");
            for device in &registry.devices {
                let last_seen = &device.last_seen;
                eprintln!("  \x1b[36m{}\x1b[0m  ({})", device.name, device.id);
                eprintln!("    Paired: {}  Last seen: {}", device.paired_at, last_seen);
                if let Some(ref apns) = device.apns_token {
                    eprintln!("    APNs: {}...", &apns[..apns.len().min(16)]);
                }
                eprintln!();
            }
        }
        return Ok(());
    }

    if let Some(ref target) = revoke {
        let before = registry.devices.len();
        registry
            .devices
            .retain(|d| d.id != *target && d.name != *target);
        if registry.devices.len() < before {
            registry.save()?;
            eprintln!("\x1b[32m✓\x1b[0m Revoked device: {}", target);
        } else {
            eprintln!("\x1b[31m✗\x1b[0m No device found matching: {}", target);
        }
        return Ok(());
    }

    let gw_config = &crate::config::config().gateway;

    if !gw_config.enabled {
        eprintln!("\x1b[33m⚠\x1b[0m  Gateway is disabled. Enable it in ~/.nexode/config.toml:\n");
        eprintln!("    \x1b[2m[gateway]\x1b[0m");
        eprintln!("    \x1b[2menabled = true\x1b[0m");
        eprintln!("    \x1b[2mport = {}\x1b[0m\n", gw_config.port);
        eprintln!("  Then restart the nexode server.\n");
    }

    let code = registry.generate_pairing_code();
    let connect_host = resolve_connect_host(&gw_config.bind_addr);
    let pair_uri = format!(
        "nexode://pair?host={}&port={}&code={}",
        connect_host, gw_config.port, code
    );

    eprintln!();
    eprintln!("  \x1b[1mScan with the nexode iOS app:\x1b[0m\n");
    match nexode_terminal_prompt::qr::render_unicode_qr(&pair_uri) {
        Ok(qr) => {
            for line in qr.lines() {
                eprintln!("  {line}");
            }
        }
        Err(_) => eprintln!("  \x1b[33m(QR code generation failed)\x1b[0m"),
    }
    eprintln!();
    eprintln!(
        "  Pairing code:  \x1b[1;37m{} {}\x1b[0m   \x1b[2m(expires in 5 minutes)\x1b[0m",
        &code[..3],
        &code[3..]
    );
    let resolved_hint = format!("{}:{}", connect_host, gw_config.port);
    let bind_hint = format!("{}:{}", gw_config.bind_addr, gw_config.port);
    eprintln!("  Connect host:  \x1b[36m{}\x1b[0m", resolved_hint);
    if connect_host != gw_config.bind_addr {
        eprintln!("  Bind address:  \x1b[2m{}\x1b[0m", bind_hint);
    }

    if connect_host == gateway::UNKNOWN_CONNECT_HOST {
        eprintln!(
            "\n  \x1b[33mTip:\x1b[0m set NEXODE_GATEWAY_HOST to your reachable Tailscale hostname."
        );
    }

    if (gw_config.bind_addr.as_str(), gw_config.port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut it| it.next())
        .is_none()
    {
        eprintln!(
            "  \x1b[33mWarning:\x1b[0m gateway bind address appears invalid: {}",
            bind_hint
        );
    }
    eprintln!();

    Ok(())
}

pub use gateway::{detect_tailscale_dns_name, parse_tailscale_dns_name, resolve_connect_host};

pub async fn run_browser(action: &str, requested: Option<&str>) -> Result<()> {
    match action {
        "setup" => browser::run_setup_command_for(requested).await?,
        "detect" => {
            let target = browser::resolve_target_browser(requested)?;
            println!("Browser detection");
            println!(
                "  target: {} ({})",
                target.kind.display_name(),
                target.source.describe()
            );
            match crate::browser_detect::system_default_browser_id() {
                Some(id) => println!("  system default: {}", id),
                None => println!("  system default: unknown"),
            }
            let installed: Vec<&str> = crate::browser_detect::ALL_BROWSERS
                .iter()
                .filter(|k| k.is_installed())
                .map(|k| k.id())
                .collect();
            println!(
                "  installed: {}",
                if installed.is_empty() {
                    "none detected".to_string()
                } else {
                    installed.join(", ")
                }
            );
            if let Some(saved) = browser::saved_browser_preference() {
                println!("  configured by setup: {}", saved.id());
            }
            println!(
                "\nOverride with `nexode browser setup <browser>` or NEXODE_BROWSER=<browser>."
            );
        }
        "status" => {
            let target = browser::resolve_target_browser(requested)?;
            let name = target.kind.display_name();
            let status = browser::ensure_browser_ready_noninteractive_for(&target).await?;
            println!("Browser automation");
            println!("  backend: {}", status.backend);
            println!("  browser: {} ({})", status.browser, status.detected_via);
            if let Some(connected) = &status.connected_browser {
                println!("  connected browser: {}", connected);
            }
            println!(
                "  binary: {}",
                if status.binary_installed {
                    "installed"
                } else {
                    "missing"
                }
            );
            println!(
                "  setup: {}",
                if status.setup_complete {
                    "complete"
                } else {
                    "not complete"
                }
            );
            println!(
                "  bridge: {}",
                if status.responding {
                    "responding"
                } else {
                    "not responding"
                }
            );
            println!(
                "  compatibility: {}",
                if status.compatible {
                    "ok"
                } else if !status.responding {
                    // Nothing answered, so there is no extension to compare.
                    "unknown (bridge not responding)"
                } else {
                    "extension/bridge mismatch"
                }
            );
            if !status.missing_actions.is_empty() {
                println!("  missing actions: {}", status.missing_actions.join(", "));
            }

            if status.ready && !browser::status_answered_by(&status, target.kind) {
                let answering = status
                    .connected_browser
                    .as_deref()
                    .unwrap_or("another browser");
                println!(
                    "\n{name} is not connected: the bridge that answered belongs to {answering}. Browser actions with browser={} will fail until {name} runs the Browser Agent Bridge extension. Run `nexode browser setup {}` to set it up.",
                    target.kind.id(),
                    target.kind.id()
                );
                if browser::is_browser_running(target.kind)
                    && let Some(diagnosis) = browser::diagnose_silent_chromium_bridge(target.kind)
                {
                    println!("{diagnosis}");
                }
            } else if status.ready {
                println!("\nBuilt-in browser tool is ready.");
            } else if status.responding && !status.compatible {
                println!(
                    "\nThe browser bridge is connected, but the installed extension is out of date for this nexode build. Run `nexode browser setup` to repair or update it."
                );
            } else if status.binary_installed && !browser::is_browser_running(target.kind) {
                println!(
                    "\n{} is not running, so the bridge cannot respond. Start {} (or run a browser tool action, which launches it automatically), then re-check status. Setup is one-time and does not need to be re-run.",
                    name, name
                );
            } else if status.binary_installed {
                println!(
                    "\n{} is running, but the bridge is not responding. Check that the Browser Agent Bridge extension is enabled ({}). Run `nexode browser setup` only to repair the install.",
                    name,
                    target.kind.extensions_page()
                );
                if let Some(diagnosis) = browser::diagnose_silent_chromium_bridge(target.kind) {
                    println!("{diagnosis}");
                }
            } else {
                println!("\nRun `nexode browser setup` to install or repair it.");
            }
        }
        other => {
            eprintln!("Unknown browser action: {}", other);
            eprintln!("Available: setup [browser], status [browser], detect");
            std::process::exit(1);
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ModelListReport {
    provider: String,
    selected_model: String,
    models: Vec<String>,
    routes: Vec<ModelListRouteReport>,
}

#[derive(Debug, Serialize)]
struct ModelListRouteReport {
    provider: String,
    model: String,
    method: String,
    available: bool,
}

#[derive(Debug, Serialize)]
struct RunCommandReport {
    session_id: String,
    provider: String,
    model: String,
    text: String,
    usage: crate::agent::TokenUsage,
}

#[derive(Debug, Default)]
struct NdjsonRunState {
    text: String,
    session_id: Option<String>,
    upstream_provider: Option<String>,
    connection_type: Option<String>,
    connection_phase: Option<String>,
    status_detail: Option<String>,
    requested_service_tier: Option<String>,
    served_service_tier: Option<String>,
    usage: crate::agent::TokenUsage,
}

impl NdjsonRunState {
    /// Compare wire metadata only. A match is not independent confirmation of
    /// backend routing or billing, especially on subscription-backed endpoints.
    fn service_tier_report_status(&self) -> &'static str {
        let Some(reported) = self.served_service_tier.as_deref() else {
            return "not_reported";
        };
        if reported == "auto" {
            return "unresolved";
        }
        fn canonical(tier: &str) -> &str {
            match tier {
                "off" | "standard" | "default" | "auto" => "default",
                "fast" | "priority" => "priority",
                _ => tier,
            }
        }
        if canonical(self.requested_service_tier.as_deref().unwrap_or("default"))
            == canonical(reported)
        {
            "reported_match"
        } else {
            "reported_mismatch"
        }
    }
}

pub fn run_auth_status_command(emit_json: bool) -> Result<()> {
    report_info::run_auth_status_command(emit_json)
}

pub async fn run_auth_doctor_command(
    provider_arg: Option<&str>,
    validate: bool,
    emit_json: bool,
) -> Result<()> {
    report_info::run_auth_doctor_command(provider_arg, validate, emit_json).await
}

pub fn run_provider_list_command(emit_json: bool) -> Result<()> {
    report_info::run_provider_list_command(emit_json)
}

pub async fn run_provider_current_command(
    choice: &super::provider_init::ProviderChoice,
    model: Option<&str>,
    emit_json: bool,
) -> Result<()> {
    report_info::run_provider_current_command(choice, model, emit_json).await
}

pub fn run_version_command(emit_json: bool) -> Result<()> {
    report_info::run_version_command(emit_json)
}

pub async fn run_usage_command(emit_json: bool) -> Result<()> {
    report_info::run_usage_command(emit_json).await
}

/// Explicitly pin the daemon's shared-server channel to an installed build.
/// Promotion and reload intentionally remain separate operations: updates may
/// advance a shared server that tracks stable, but must not overwrite a build
/// the user deliberately promoted here.
pub fn run_server_promote_command(version: Option<&str>, emit_json: bool) -> Result<()> {
    #[derive(Serialize)]
    struct ServerPromoteReport {
        version: String,
        previous: Option<String>,
        binary: String,
        promoted: bool,
        detail: String,
    }

    let version = match version {
        Some(version) => version.to_string(),
        None => crate::build::read_current_version()?.ok_or_else(|| {
            anyhow::anyhow!("No current version is installed; pass an installed VERSION explicitly")
        })?,
    };
    let previous = crate::build::promote_version_to_shared_server(&version)?;
    let binary = crate::build::version_binary_path(&version)?;
    let promoted = previous.as_deref() != Some(version.as_str());
    let detail = if promoted {
        format!(
            "shared-server channel {} -> {}. Run `nexode server reload` to apply it to the running daemon.",
            previous.as_deref().unwrap_or("<unset>"),
            version
        )
    } else {
        format!(
            "shared-server channel already points to {}. Run `nexode server reload` if the running daemon has not applied it.",
            version
        )
    };
    let report = ServerPromoteReport {
        version,
        previous,
        binary: binary.display().to_string(),
        promoted,
        detail,
    };

    if emit_json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", report.detail);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ServerReloadReport {
    socket: String,
    had_listener: bool,
    forced: bool,
    reloaded: bool,
    already_current: bool,
    handoff_ready: bool,
    detail: String,
}

fn validate_server_reload_report(report: &ServerReloadReport) -> Result<()> {
    // A reload that asked the old server to hand over, and then never saw the
    // new one take the socket, did not succeed. It is the one outcome a caller
    // cannot infer from the exit status alone: until now every path here
    // returned Ok(()), and the distinction lived only inside the JSON body.
    //
    // Scope is deliberately narrow, because the comment below documents that
    // an installer may call `nexode server reload` unconditionally. The two
    // states that are arguably a success keep exit 0: there was nothing
    // running (`had_listener == false`), or the binary was already current
    // (`already_current`). Only the not-ready handoff, where the daemon is
    // genuinely not serving yet, reports failure.
    if report.had_listener && !report.already_current && !report.handoff_ready {
        anyhow::bail!(
            "nexode server reload was requested but the new server never became ready: {}",
            report.detail
        );
    }
    Ok(())
}

/// Gracefully reload the running background server onto the newest binary.
///
/// This is the preferred upgrade path (issue #291): instead of killing the
/// daemon and dropping live headless/swarm sessions, we ask it to hand its
/// sessions off to a freshly exec'd server (the same path `/reload` uses).
///
/// Behavior:
/// - With `force == false` (the default), the server only reloads when it is
///   provably running older code than an available reload candidate. A server
///   already on the newest binary reports "already up to date" and does
///   nothing, which keeps an installer from downgrading a newer/dev daemon or
///   re-entering the reload-loop family (#277).
/// - With `force == true`, the server reloads unconditionally.
/// - If no server is running, this is a successful no-op so installers can call
///   it unconditionally.
pub async fn run_server_reload_command(force: bool, emit_json: bool) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    run_server_reload_command_to(force, emit_json, &mut stdout).await
}

async fn run_server_reload_command_to(
    force: bool,
    emit_json: bool,
    stdout: &mut impl Write,
) -> Result<()> {
    use crate::protocol::ServerEvent;
    use std::time::Duration;

    let socket = crate::server::socket_path();

    let mut emit = |report: ServerReloadReport| -> Result<()> {
        let outcome = validate_server_reload_report(&report);
        if emit_json {
            serde_json::to_writer_pretty(&mut *stdout, &report)?;
            stdout.write_all(b"\n")?;
        } else if !report.detail.is_empty() {
            writeln!(stdout, "{}", report.detail)?;
        }
        // Keep printing the report above the status check so --json output is
        // byte-identical for callers that parse it.
        outcome
    };

    // No server? Nothing to reload. This is a success so an installer can call
    // `nexode server reload` unconditionally after swapping the binary.
    if !crate::server::has_live_listener(&socket).await {
        // Reap a stale socket left by a crashed daemon so the next launch binds
        // cleanly instead of wedging in a connect-retry loop.
        let reaped = crate::server::reap_stale_socket_if_dead(&socket).await;
        let detail = if reaped {
            "No running nexode server found; cleared a stale socket.".to_string()
        } else {
            "No running nexode server found; nothing to reload.".to_string()
        };
        return emit(ServerReloadReport {
            socket: socket.display().to_string(),
            had_listener: false,
            forced: force,
            reloaded: false,
            already_current: false,
            handoff_ready: false,
            detail,
        });
    }

    let mut client = crate::server::Client::connect().await?;

    // The server requires `Subscribe` as the first frame and rejects any other
    // opening request with "Client must Subscribe with a working_dir before
    // sending stateful requests", so `reload` alone always failed (issue #648).
    // `subscribe()` defaults `working_dir` to the current directory.
    client.subscribe().await?;

    // Before asking the (possibly older) daemon to reload, repair a stale
    // `shared-server` channel from the client side. The running server resolves
    // its reload target from that channel; if it still points at the server's
    // own old binary (the "current client, stale server" state, e.g. after a
    // no-op `/update`), a forced reload would just re-exec the same old binary.
    // Repointing shared-server -> stable when stable is strictly newer gives the
    // reload a newer binary to exec into. Never downgrades; preserves a fresher
    // self-dev pin. Best-effort: a failure here must not block the reload.
    match crate::build::repair_stale_shared_server_channel() {
        Ok(crate::build::SharedServerRepair::Repaired {
            repaired_to,
            previous,
        }) => {
            crate::logging::info(&format!(
                "server reload: repaired stale shared-server channel {:?} -> {} before reload",
                previous, repaired_to
            ));
        }
        Ok(crate::build::SharedServerRepair::AlreadyCurrent) => {}
        Err(err) => {
            crate::logging::warn(&format!(
                "server reload: shared-server channel repair failed (continuing): {}",
                err
            ));
        }
    }

    let request_id = client.reload_with_force(force).await?;

    let mut reloading = false;
    let mut skipped = false;

    // Drive the request to a terminal state. On a real reload the old server
    // exec's a new process, which drops this connection after it sends Done;
    // we treat a disconnect after observing Reloading as the expected handoff.
    loop {
        match client.read_event().await.map(unwrap_turn_event) {
            Ok(ServerEvent::Ack { id }) if id == request_id => {}
            Ok(ServerEvent::Reloading { .. }) => {
                reloading = true;
            }
            Ok(ServerEvent::ReloadProgress { step, .. }) if step == "skip" => {
                skipped = true;
            }
            Ok(ServerEvent::ReloadProgress { .. }) => {}
            Ok(ServerEvent::Done { id }) if id == request_id => break,
            Ok(ServerEvent::Error { id, message, .. }) if id == request_id => {
                anyhow::bail!("server reload failed: {message}");
            }
            Ok(_) => {}
            Err(e) => {
                // A disconnect mid-reload is the expected handoff; otherwise it
                // is a genuine failure.
                if reloading {
                    break;
                }
                return Err(e);
            }
        }
    }

    if skipped && !reloading {
        return emit(ServerReloadReport {
            socket: socket.display().to_string(),
            had_listener: true,
            forced: force,
            reloaded: false,
            already_current: true,
            handoff_ready: true,
            detail: "nexode server is already running the newest binary; no reload needed."
                .to_string(),
        });
    }

    // Wait (bounded) for the freshly exec'd server to take over the socket so
    // callers know the upgrade actually landed.
    let handoff_ready = matches!(
        crate::server::await_reload_handoff(&socket, Duration::from_secs(30)).await,
        crate::server::ReloadWaitStatus::Ready
    );

    let detail = if handoff_ready {
        "nexode server reloaded onto the newest binary.".to_string()
    } else {
        "nexode server reload requested; the new server is still coming up.".to_string()
    };

    emit(ServerReloadReport {
        socket: socket.display().to_string(),
        had_listener: true,
        forced: force,
        reloaded: true,
        already_current: false,
        handoff_ready,
        detail,
    })
}

/// Stop the running background server gracefully and clear its socket.
///
/// Intended for use after an upgrade so the next launch starts the freshly
/// installed binary instead of a surviving daemon running old code (issue #291).
///
/// Steps:
/// 1. Look up the daemon owning the active socket in the server registry and
///    send it SIGTERM (the daemon has a graceful SIGTERM handler).
/// 2. Wait for the listener to go away (bounded), escalating to SIGKILL only if
///    the process refuses to exit.
/// 3. Reap any leftover stale socket so a later launch binds cleanly.
pub async fn run_server_stop_command(force: bool, emit_json: bool) -> Result<()> {
    use std::time::{Duration, Instant};

    if !force {
        let msg = "`nexode server stop` terminates the daemon and drops any live headless/swarm sessions. \
Prefer `nexode server reload` to pick up an upgrade gracefully. \
Re-run with `--force` if you really want to stop the server.";
        if emit_json {
            println!(
                "{}",
                serde_json::json!({
                    "stopped": false,
                    "force_required": true,
                    "detail": msg,
                })
            );
        } else {
            eprintln!("{msg}");
        }
        return Ok(());
    }

    let socket = crate::server::socket_path();
    let had_listener = crate::server::has_live_listener(&socket).await;
    let server_info = crate::registry::find_server_by_socket_sync(&socket);

    #[derive(Serialize)]
    struct ServerStopReport {
        socket: String,
        had_listener: bool,
        signaled_pid: Option<u32>,
        stopped: bool,
        reaped_socket: bool,
        detail: String,
    }

    let mut signaled_pid: Option<u32> = None;
    let mut stopped = false;
    let detail: String;

    if let Some(info) = server_info.as_ref() {
        let pid = info.pid;
        if crate::platform::is_process_running(pid) {
            #[cfg(unix)]
            {
                // The daemon spawns detached with setsid(), so it leads its own
                // process group. Signal the group so any helper children exit too.
                match crate::platform::signal_detached_process_group(pid, libc::SIGTERM) {
                    Ok(()) => {
                        signaled_pid = Some(pid);
                        detail = format!("Sent SIGTERM to nexode server (pid {pid}).");
                    }
                    Err(e) => {
                        detail = format!("Failed to signal nexode server (pid {pid}): {e}");
                    }
                }
            }
            #[cfg(not(unix))]
            {
                match crate::platform::signal_detached_process_group(pid, 0) {
                    Ok(()) => {
                        signaled_pid = Some(pid);
                        detail = format!("Terminated nexode server (pid {pid}).");
                    }
                    Err(e) => {
                        detail = format!("Failed to terminate nexode server (pid {pid}): {e}");
                    }
                }
            }
        } else {
            detail = format!("Registered nexode server (pid {pid}) is not running.");
        }
    } else if had_listener {
        // A listener answers but no registry entry maps to it. We deliberately
        // do not guess a pid; just reap the socket below once the listener is
        // gone. (This is rare: a daemon that bound the socket but never wrote a
        // registry entry.)
        detail = "Found a live server socket with no registry entry.".to_string();
    } else {
        detail = "No running nexode server found.".to_string();
    }

    // Wait for the listener to disappear after signalling. Escalate to SIGKILL
    // once if the daemon does not exit within the graceful window.
    if signaled_pid.is_some() || had_listener {
        let deadline = Instant::now() + Duration::from_secs(5);
        #[cfg(unix)]
        let mut escalated = false;
        loop {
            let listener_gone = !crate::server::has_live_listener(&socket).await;
            let process_gone = signaled_pid
                .map(|pid| !crate::platform::is_process_running(pid))
                .unwrap_or(true);
            if listener_gone && process_gone {
                stopped = true;
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            #[cfg(unix)]
            if !escalated
                && Instant::now() + Duration::from_secs(2) >= deadline
                && let Some(pid) = signaled_pid
                && crate::platform::is_process_running(pid)
            {
                let _ = crate::platform::signal_detached_process_group(pid, libc::SIGKILL);
                escalated = true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    } else {
        stopped = true;
    }

    // Reap any stale socket the (now-dead) daemon left behind so the next launch
    // binds cleanly instead of wedging in a connect-retry loop.
    let reaped = crate::server::reap_stale_socket_if_dead(&socket).await;

    if emit_json {
        let report = ServerStopReport {
            socket: socket.display().to_string(),
            had_listener,
            signaled_pid,
            stopped,
            reaped_socket: reaped,
            detail: detail.clone(),
        };
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        if !detail.is_empty() {
            println!("{detail}");
        }
        if stopped && signaled_pid.is_some() {
            println!("nexode server stopped.");
        } else if stopped && !had_listener && signaled_pid.is_none() {
            // Nothing was running; this is still a success for an installer.
        } else if !stopped {
            println!(
                "nexode server did not exit cleanly; it may still be shutting down. Re-run if needed."
            );
        }
        if reaped {
            println!("Cleared a stale nexode socket.");
        }
    }

    Ok(())
}

pub async fn run_single_message_command(
    choice: &super::provider_init::ProviderChoice,
    model: Option<&str>,
    resume_session: Option<&str>,
    message: &str,
    emit_json: bool,
    emit_ndjson: bool,
) -> Result<()> {
    let provider =
        super::provider_init::init_provider_for_run(choice, model, emit_json || emit_ndjson)
            .await?;
    // Older model caches contain names but no account-scoped speed capabilities.
    // Hydrate those before the first headless request so an unsupported saved
    // tier cannot slip through the backward-compatible unknown-catalog path.
    if provider.model_capabilities_need_refresh()
        && let Err(error) = provider.prefetch_models().await
    {
        crate::logging::warn(&format!(
            "Could not refresh model capabilities before headless run: {error}"
        ));
    }
    let registry = crate::tool::Registry::new(provider.clone()).await;
    // Load MCP servers from ~/.nexode/mcp.json so headless `nexode run` has the
    // same `mcp__*` tools as interactive/server sessions. This is non-blocking:
    // `register_mcp_tools` advertises cached tool schemas synchronously (so the
    // first locked tool snapshot already contains MCP tools, for zero
    // prompt-cache miss) and connects in the background (connect-on-first-call).
    // For a short single-message run, startup latency is unchanged.
    // (#390, #206 Phase 2)
    if run_command_mcp_enabled() {
        registry.register_mcp_tools(None, None, None).await;
        // Cold-cache gap: when a configured MCP server has no cached schema yet
        // (first ever use, or reconfigured), advertise-early registers nothing
        // for it, and a single-turn `nexode run` locks its tool snapshot before
        // the background connection finishes, so the model would never see those
        // tools. Long-lived sessions recover on a later turn, but `nexode run`
        // has no later turn. So, only when the cache is cold for some configured
        // server, briefly wait for the first connection to register tools before
        // the agent runs. Warm runs skip this entirely and stay instant. (#390)
        wait_for_cold_cache_mcp_tools(&registry).await;
    }
    let mut agent = crate::agent::Agent::new(provider.clone(), registry);
    if let Err(error) = restore_agent_session_if_requested(&mut agent, resume_session) {
        agent.mark_closed();
        return Err(error);
    }

    run_single_message_with_agent(&mut agent, provider, message, emit_json, emit_ndjson).await
}

/// Provider id for `nexode run --json`/`--ndjson` output.
///
/// Every direct OpenAI-compatible profile (Ollama, LM Studio, DeepSeek, custom
/// endpoints, ...) runs on the OpenRouter transport, whose `name()` is the
/// fixed slot id `openrouter`. Reporting that attributed purely local runs to
/// a paid third-party aggregator (#804). The session already records the
/// resolved route identity, so prefer it for that slot and leave every other
/// provider's established value unchanged.
fn run_report_provider_name(
    provider: &dyn crate::provider::Provider,
    session_provider_key: Option<&str>,
) -> String {
    let name = provider.name();
    if name.eq_ignore_ascii_case("openrouter")
        && let Some(key) = session_provider_key
            .map(str::trim)
            .filter(|key| !key.is_empty())
    {
        return key.to_string();
    }
    name.to_string()
}

async fn run_single_message_with_agent(
    agent: &mut crate::agent::Agent,
    provider: std::sync::Arc<dyn crate::provider::Provider>,
    message: &str,
    emit_json: bool,
    emit_ndjson: bool,
) -> Result<()> {
    // A detached dependency can outlive a model turn without holding an
    // in-process activity guard. Keep this CLI continuation alive across the
    // wait too, rather than letting an idle-only runtime replacement lose it.
    // run_once's nested read reservations are deliberately safe.
    let result: Result<()> = async {
        let _runtime_activity = crate::runtime_activity::global().try_enter()?;
        if emit_json {
            let text = run_single_message_command_capture_with_auto_poke(agent, message).await?;
            let report = RunCommandReport {
                session_id: agent.session_id().to_string(),
                provider: run_report_provider_name(
                    provider.as_ref(),
                    agent.session_provider_key().as_deref(),
                ),
                model: provider.model(),
                text,
                usage: agent.last_usage().clone(),
            };
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else if emit_ndjson {
            run_single_message_command_ndjson(agent, provider, message).await?;
        } else {
            run_single_message_command_plain_with_auto_poke(agent, message).await?;
        }
        Ok(())
    }
    .await;

    // `Agent::new` and session restore both register this process as the active
    // owner. Unlike the interactive lifecycle, `nexode run` has no later quit
    // path to close the session. Finalize after output has been emitted, while
    // returning the original command result unchanged. This prevents a normal
    // one-shot exit from looking like a stale-PID crash on the next startup
    // (issue #988).
    agent.mark_closed();
    result
}

fn run_command_auto_poke_enabled() -> bool {
    std::env::var("NEXODE_RUN_AUTO_POKE")
        .ok()
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or_else(|| crate::config::config().features.auto_poke)
}

/// Whether headless `nexode run` should load MCP servers from `~/.nexode/mcp.json`.
/// Enabled by default; set `NEXODE_RUN_MCP=0` (or `false`/`off`/`no`) to skip MCP
/// registration for latency-sensitive scripting. (#390)
fn run_command_mcp_enabled() -> bool {
    std::env::var("NEXODE_RUN_MCP")
        .ok()
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(true)
}

/// Max time `nexode run` waits for cold-cache MCP servers to register their
/// tools before running the single turn. Override with `NEXODE_RUN_MCP_WAIT_MS`
/// (0 disables the wait).
fn run_command_mcp_cold_wait() -> std::time::Duration {
    let ms = std::env::var("NEXODE_RUN_MCP_WAIT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(5000);
    std::time::Duration::from_millis(ms)
}

/// Returns the set of MCP servers configured for this run that have no usable
/// cached schema yet (cold cache). Advertise-early can only pre-register tools
/// for servers whose schemas are cached, so these are the servers whose tools
/// would otherwise miss the single-turn snapshot.
fn cold_cache_mcp_servers() -> Vec<String> {
    let config = crate::mcp::McpConfig::load();
    let cache = crate::mcp::McpSchemaCache::load();
    uncached_enabled_mcp_servers(&config, &cache)
}

fn uncached_enabled_mcp_servers(
    config: &crate::mcp::McpConfig,
    cache: &crate::mcp::McpSchemaCache,
) -> Vec<String> {
    config
        .servers
        .iter()
        .filter(|(name, cfg)| cfg.is_enabled() && cache.tools_for(name, cfg).is_none())
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
#[path = "mcp_startup_tests.rs"]
mod mcp_startup_tests;

/// Bridge the cold-cache gap for `nexode run`: if any configured MCP server has
/// no cached schema, briefly poll the registry until its `mcp__*` tools appear
/// (or the budget elapses) so the single turn's locked tool snapshot includes
/// them. Warm caches return immediately because `cold_cache_mcp_servers` is
/// empty. (#390)
async fn wait_for_cold_cache_mcp_tools(registry: &crate::tool::Registry) {
    let cold_servers = cold_cache_mcp_servers();
    if cold_servers.is_empty() {
        return;
    }
    let budget = run_command_mcp_cold_wait();
    if budget.is_zero() {
        return;
    }
    crate::logging::info(&format!(
        "nexode run: waiting up to {}ms for cold-cache MCP server(s) to register tools: {}",
        budget.as_millis(),
        cold_servers.join(", ")
    ));
    let deadline = std::time::Instant::now() + budget;
    loop {
        let covered = cold_servers
            .iter()
            .all(|server| registry.has_mcp_tools_for_server(server));
        if covered {
            crate::logging::info(
                "nexode run: cold-cache MCP server(s) registered tools; proceeding",
            );
            return;
        }
        if std::time::Instant::now() >= deadline {
            crate::logging::warn(
                "nexode run: timed out waiting for cold-cache MCP server(s); \
                 their tools may be missing from this run",
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

fn run_command_auto_poke_max_turns() -> Option<usize> {
    std::env::var("NEXODE_RUN_AUTO_POKE_MAX_TURNS")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
}

fn run_command_auto_poke_limit_reached(turns_completed: usize, max_turns: Option<usize>) -> bool {
    max_turns
        .map(|max_turns| turns_completed >= max_turns)
        .unwrap_or(false)
}

mod todo_followup;
use todo_followup::{
    RunAutoPokeFollowUp, build_run_auto_poke_follow_up_from_todos, prepare_run_todo_follow_up,
    run_actionable_poke_message, run_blocked_notice, run_completely_blocked_notice,
    run_external_blocker_notice, run_todos, take_run_gate_digest_if_turn_ended,
};
#[cfg(test)]
use todo_followup::{run_actionable_todos, take_run_gate_digest};

async fn run_single_message_command_plain_with_auto_poke(
    agent: &mut crate::agent::Agent,
    message: &str,
) -> Result<()> {
    let mut next_message = message.to_string();
    let max_turns = run_command_auto_poke_max_turns();
    let mut turns_completed = 0usize;
    let mut confidence_spike_challenged = false;
    let mut gate_digest_delivered = false;
    let mut incomplete_budget = crate::todo::IncompleteTodoBudget::default();
    loop {
        agent.run_once(&next_message).await?;
        turns_completed += 1;
        if !run_command_auto_poke_enabled() {
            break;
        }
        let todos = run_todos(agent.session_id());
        let (actionable_todos, dependency_resume) = prepare_run_todo_follow_up(
            crate::background::global(),
            agent.session_id(),
            &todos,
            !run_command_auto_poke_limit_reached(turns_completed, max_turns),
        )
        .await;
        if let Some(message) = dependency_resume {
            incomplete_budget = crate::todo::IncompleteTodoBudget::default();
            gate_digest_delivered = false;
            next_message = message;
            continue;
        }
        if todos.iter().any(crate::todo::is_incomplete_todo)
            && !actionable_todos.iter().any(crate::todo::is_incomplete_todo)
        {
            eprintln!("{}", run_blocked_notice(&todos).unwrap_or_else(||
                "Stopped at the turn limit with unfinished todos waiting on background dependencies.".into()
            ));
            break;
        }
        if !incomplete_budget.allow_follow_up(&actionable_todos) {
            eprintln!(
                "We stopped poking after {} attempts without todo progress; unfinished todos remain.",
                crate::todo::INCOMPLETE_TODO_MAX_ATTEMPTS
            );
            break;
        }
        let gate_digest = take_run_gate_digest_if_turn_ended(
            agent.session_id(),
            gate_digest_delivered,
            &todos,
            crate::config::config().features.todo_gate_digest,
        );
        match build_run_auto_poke_follow_up_from_todos(
            crate::config::config().features.todo_gate_digest,
            &actionable_todos,
            confidence_spike_challenged,
            gate_digest,
        ) {
            Some(RunAutoPokeFollowUp::GateDigest { message }) => {
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        eprintln!(
                            "We stopped poking after {max_turns} turn(s); some quality-review points are still open."
                        );
                    }
                    break;
                }
                gate_digest_delivered = true;
                next_message = message;
                eprintln!(
                    "We asked the agent to double-check this turn's weak points. Set NEXODE_RUN_AUTO_POKE=0 to disable."
                );
                continue;
            }
            Some(RunAutoPokeFollowUp::ConfidenceSummary {
                message,
                confidence_spike_challenge,
                ..
            }) => {
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        eprintln!(
                            "We stopped poking after {max_turns} turn(s); the agent's completion confidence still needs validation."
                        );
                    }
                    break;
                }
                confidence_spike_challenged |= confidence_spike_challenge;
                next_message = message;
                eprintln!(
                    "Todos are done. Asking the agent for a final confidence check. Set NEXODE_RUN_AUTO_POKE=0 to disable."
                );
                continue;
            }
            Some(RunAutoPokeFollowUp::Incomplete { count, message }) => {
                let message = run_actionable_poke_message(message, &todos, &actionable_todos);
                gate_digest_delivered = false;
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        eprintln!(
                            "We stopped poking after {max_turns} turn(s); {} todo(s) are still unfinished.",
                            count
                        );
                    }
                    break;
                }
                next_message = message;
                eprintln!(
                    "{} incomplete todo(s). We poked the agent for you. Set NEXODE_RUN_AUTO_POKE=0 to disable.",
                    count
                );
            }
            None => break,
        }
    }
    Ok(())
}

async fn run_single_message_command_capture_with_auto_poke(
    agent: &mut crate::agent::Agent,
    message: &str,
) -> Result<String> {
    let mut next_message = message.to_string();
    let max_turns = run_command_auto_poke_max_turns();
    let mut outputs = Vec::new();
    let mut turns_completed = 0usize;
    let mut confidence_spike_challenged = false;
    let mut gate_digest_delivered = false;
    let mut incomplete_budget = crate::todo::IncompleteTodoBudget::default();
    loop {
        outputs.push(agent.run_once_capture(&next_message).await?);
        turns_completed += 1;
        if !run_command_auto_poke_enabled() {
            break;
        }
        let todos = run_todos(agent.session_id());
        let (actionable_todos, dependency_resume) = prepare_run_todo_follow_up(
            crate::background::global(),
            agent.session_id(),
            &todos,
            !run_command_auto_poke_limit_reached(turns_completed, max_turns),
        )
        .await;
        if let Some(message) = dependency_resume {
            incomplete_budget = crate::todo::IncompleteTodoBudget::default();
            gate_digest_delivered = false;
            next_message = message;
            continue;
        }
        if todos.iter().any(crate::todo::is_incomplete_todo)
            && !actionable_todos.iter().any(crate::todo::is_incomplete_todo)
        {
            eprintln!("{}", run_blocked_notice(&todos).unwrap_or_else(||
                "Stopped at the turn limit with unfinished todos waiting on background dependencies.".into()
            ));
            break;
        }
        if !incomplete_budget.allow_follow_up(&actionable_todos) {
            eprintln!(
                "We stopped poking after {} attempts without todo progress; unfinished todos remain.",
                crate::todo::INCOMPLETE_TODO_MAX_ATTEMPTS
            );
            break;
        }
        let gate_digest = take_run_gate_digest_if_turn_ended(
            agent.session_id(),
            gate_digest_delivered,
            &todos,
            crate::config::config().features.todo_gate_digest,
        );
        match build_run_auto_poke_follow_up_from_todos(
            crate::config::config().features.todo_gate_digest,
            &actionable_todos,
            confidence_spike_challenged,
            gate_digest,
        ) {
            Some(RunAutoPokeFollowUp::GateDigest { message }) => {
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        eprintln!(
                            "We stopped poking after {max_turns} turn(s); some quality-review points are still open."
                        );
                    }
                    break;
                }
                gate_digest_delivered = true;
                next_message = message;
                eprintln!(
                    "We asked the agent to double-check this turn's weak points. Set NEXODE_RUN_AUTO_POKE=0 to disable."
                );
                continue;
            }
            Some(RunAutoPokeFollowUp::ConfidenceSummary {
                message,
                confidence_spike_challenge,
                ..
            }) => {
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        outputs.push(format!(
                            "We stopped poking after {max_turns} turn(s); the agent's completion confidence still needs validation."
                        ));
                    }
                    break;
                }
                confidence_spike_challenged |= confidence_spike_challenge;
                next_message = message;
                continue;
            }
            Some(RunAutoPokeFollowUp::Incomplete { count, message }) => {
                let message = run_actionable_poke_message(message, &todos, &actionable_todos);
                gate_digest_delivered = false;
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        outputs.push(format!(
                            "We stopped poking after {max_turns} turn(s); {} todo(s) are still unfinished.",
                            count
                        ));
                    }
                    break;
                }
                next_message = message;
            }
            None => break,
        }
    }
    Ok(outputs.join("\n\n"))
}

fn restore_agent_session_if_requested(
    agent: &mut crate::agent::Agent,
    resume_session: Option<&str>,
) -> Result<()> {
    if let Some(session_id) = resume_session {
        agent.restore_session(session_id)?;
    }
    Ok(())
}

async fn run_single_message_command_ndjson(
    agent: &mut crate::agent::Agent,
    provider: std::sync::Arc<dyn crate::provider::Provider>,
    message: &str,
) -> Result<()> {
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let session_id = agent.session_id().to_string();
    let provider_name =
        run_report_provider_name(provider.as_ref(), agent.session_provider_key().as_deref());
    let mut stdout = std::io::stdout().lock();
    let mut state = NdjsonRunState {
        session_id: Some(session_id.clone()),
        ..NdjsonRunState::default()
    };
    write_json_line(
        &mut stdout,
        &serde_json::json!({
            "type": "start",
            "session_id": session_id,
            "provider": provider_name,
            "model": provider.model(),
        }),
    )?;

    let max_turns = run_command_auto_poke_max_turns();
    let mut next_message = message.to_string();
    let mut result: Result<()> = Ok(());
    let mut turns_completed = 0usize;
    let mut confidence_spike_challenged = false;
    let mut gate_digest_delivered = false;
    let mut incomplete_budget = crate::todo::IncompleteTodoBudget::default();
    loop {
        let turn_result = {
            let mut run_future = std::pin::pin!(agent.run_once_streaming_mpsc(
                &next_message,
                Vec::new(),
                None,
                event_tx.clone(),
            ));
            let mut run_result: Option<Result<()>> = None;
            loop {
                tokio::select! {
                    result = &mut run_future, if run_result.is_none() => {
                        run_result = Some(result);
                    }
                    event = event_rx.recv() => {
                        match event {
                            Some(event) => emit_ndjson_event(&mut stdout, &mut state, event)?,
                            None => break,
                        }
                    }
                }
                if run_result.is_some() {
                    while let Ok(event) = event_rx.try_recv() {
                        emit_ndjson_event(&mut stdout, &mut state, event)?;
                    }
                    break;
                }
            }
            run_result.unwrap_or(Ok(()))
        };

        if let Err(err) = turn_result {
            result = Err(err);
            break;
        }
        turns_completed += 1;
        if !run_command_auto_poke_enabled() {
            break;
        }
        let todos = run_todos(&session_id);
        let (actionable_todos, dependency_resume) = prepare_run_todo_follow_up(
            crate::background::global(),
            &session_id,
            &todos,
            !run_command_auto_poke_limit_reached(turns_completed, max_turns),
        )
        .await;
        if let Some(message) = dependency_resume {
            incomplete_budget = crate::todo::IncompleteTodoBudget::default();
            gate_digest_delivered = false;
            next_message = message;
            write_json_line(
                &mut stdout,
                &serde_json::json!({
                    "type": "auto_poke_dependency_resolved",
                    "session_id": session_id,
                    "message": next_message,
                }),
            )?;
            continue;
        }
        if todos.iter().any(crate::todo::is_incomplete_todo)
            && !actionable_todos.iter().any(crate::todo::is_incomplete_todo)
        {
            write_json_line(
                &mut stdout,
                &serde_json::json!({
                    "type": "auto_poke_stopped",
                    "session_id": session_id,
                    "reason": if run_completely_blocked_notice(&todos).is_some() { "completely_blocked" } else if run_external_blocker_notice(&todos).is_some() { "external_blocker" } else { "turn_limit" },
                    "message": run_blocked_notice(&todos),
                    "completely_blocked": todos.iter().any(crate::todo::todo_completely_blocked),
                    "waiting_on_background": todos.iter().any(|todo| !todo.waiting_on.is_empty()),
                    "waiting_on_external": todos.iter().any(crate::todo::todo_has_external_blocker),
                    "max_turns": max_turns,
                }),
            )?;
            break;
        }
        if !incomplete_budget.allow_follow_up(&actionable_todos) {
            writeln!(
                stdout,
                "{}",
                serde_json::json!({
                    "type": "auto_poke_stopped",
                    "reason": "unchanged_todos",
                    "max_attempts": crate::todo::INCOMPLETE_TODO_MAX_ATTEMPTS,
                    "incomplete_todos": todos.iter().filter(|todo| crate::todo::is_incomplete_todo(todo)).count(),
                })
            )?;
            break;
        }
        let gate_digest = take_run_gate_digest_if_turn_ended(
            agent.session_id(),
            gate_digest_delivered,
            &todos,
            crate::config::config().features.todo_gate_digest,
        );
        match build_run_auto_poke_follow_up_from_todos(
            crate::config::config().features.todo_gate_digest,
            &actionable_todos,
            confidence_spike_challenged,
            gate_digest,
        ) {
            Some(RunAutoPokeFollowUp::GateDigest { message }) => {
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        eprintln!(
                            "We stopped poking after {max_turns} turn(s); some quality-review points are still open."
                        );
                    }
                    break;
                }
                gate_digest_delivered = true;
                next_message = message;
                eprintln!(
                    "We asked the agent to double-check this turn's weak points. Set NEXODE_RUN_AUTO_POKE=0 to disable."
                );
                continue;
            }
            Some(RunAutoPokeFollowUp::ConfidenceSummary {
                total_todos,
                message,
                confidence_spike_challenge,
            }) => {
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        write_json_line(
                            &mut stdout,
                            &serde_json::json!({
                                "type": "auto_poke_stopped",
                                "session_id": session_id,
                                "completion_confidence_needs_validation": true,
                                "max_turns": max_turns,
                            }),
                        )?;
                    }
                    break;
                }
                confidence_spike_challenged |= confidence_spike_challenge;
                next_message = message;
                write_json_line(
                    &mut stdout,
                    &serde_json::json!({
                        "type": "auto_poke_confidence_summary",
                        "session_id": session_id,
                        "todos": total_todos,
                        "confidence_spike_challenge": confidence_spike_challenge,
                        "message": next_message,
                    }),
                )?;
                continue;
            }
            Some(RunAutoPokeFollowUp::Incomplete { count, message }) => {
                let message = run_actionable_poke_message(message, &todos, &actionable_todos);
                gate_digest_delivered = false;
                if run_command_auto_poke_limit_reached(turns_completed, max_turns) {
                    if let Some(max_turns) = max_turns {
                        write_json_line(
                            &mut stdout,
                            &serde_json::json!({
                                "type": "auto_poke_stopped",
                                "session_id": session_id,
                                "incomplete_todos": count,
                                "max_turns": max_turns,
                            }),
                        )?;
                    }
                    break;
                }
                next_message = message;
                write_json_line(
                    &mut stdout,
                    &serde_json::json!({
                        "type": "auto_poke",
                        "session_id": session_id,
                        "incomplete_todos": count,
                        "message": next_message,
                    }),
                )?;
            }
            None => break,
        }
    }

    match result {
        Ok(()) => {
            write_json_line(
                &mut stdout,
                &serde_json::json!({
                    "type": "done",
                    "session_id": session_id,
                    "provider": provider_name,
                    "model": provider.model(),
                    "text": state.text,
                    "usage": state.usage,
                    "upstream_provider": state.upstream_provider,
                    "connection_type": state.connection_type,
                    "connection_phase": state.connection_phase,
                    "status_detail": state.status_detail,
                    "requested_service_tier": state.requested_service_tier,
                    "reported_service_tier": state.served_service_tier,
                    "service_tier_report_status": state.service_tier_report_status(),
                    // Retained for compatibility; this is provider-reported
                    // metadata, not independently verified serving evidence.
                    "served_service_tier": state.served_service_tier,
                }),
            )?;
            Ok(())
        }
        Err(err) => {
            write_json_line(
                &mut stdout,
                &serde_json::json!({
                    "type": "error",
                    "session_id": session_id,
                    "provider": provider_name,
                    "model": provider.model(),
                    "message": format!("{err:#}"),
                }),
            )?;
            Err(err)
        }
    }
}

/// Legacy CLI adapters consume the payload even when an identity-aware peer
/// sends an explicit envelope. Unwrap iteratively to tolerate nested envelopes.
pub(super) fn unwrap_turn_event(
    mut event: crate::protocol::ServerEvent,
) -> crate::protocol::ServerEvent {
    while let crate::protocol::ServerEvent::TurnEvent { event: inner, .. } = event {
        event = *inner;
    }
    event
}

fn emit_ndjson_event(
    stdout: &mut impl Write,
    state: &mut NdjsonRunState,
    event: crate::protocol::ServerEvent,
) -> Result<()> {
    use crate::protocol::ServerEvent;

    match unwrap_turn_event(event) {
        ServerEvent::TextDelta { text } => {
            state.text.push_str(&text);
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "text_delta", "text": text }),
            )
        }
        ServerEvent::TextReplace { text } => {
            state.text = text.clone();
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "text_replace", "text": text }),
            )
        }
        ServerEvent::TextSuffixReplace { old_text, new_text } => {
            crate::message::replace_text_suffix(&mut state.text, &old_text, &new_text);
            write_json_line(
                stdout,
                &serde_json::json!({
                    "type": "text_suffix_replace", "old_text": old_text, "new_text": new_text,
                }),
            )
        }
        ServerEvent::ToolStart { id, name } => write_json_line(
            stdout,
            &serde_json::json!({ "type": "tool_start", "id": id, "name": name }),
        ),
        ServerEvent::ToolInput { delta, .. } => write_json_line(
            stdout,
            &serde_json::json!({ "type": "tool_input", "delta": delta }),
        ),
        ServerEvent::ToolExec { id, name } => write_json_line(
            stdout,
            &serde_json::json!({ "type": "tool_exec", "id": id, "name": name }),
        ),
        ServerEvent::ToolDone {
            id,
            name,
            output,
            error,
        } => write_json_line(
            stdout,
            &serde_json::json!({
                "type": "tool_done",
                "id": id,
                "name": name,
                "output": output,
                "error": error,
            }),
        ),
        ServerEvent::TokenUsage {
            input,
            output,
            cache_read_input,
            cache_creation_input,
            ..
        } => {
            state.usage = crate::agent::TokenUsage {
                input_tokens: input,
                output_tokens: output,
                cache_read_input_tokens: cache_read_input,
                cache_creation_input_tokens: cache_creation_input,
            };
            write_json_line(
                stdout,
                &serde_json::json!({
                    "type": "tokens",
                    "input": input,
                    "output": output,
                    "cache_read_input": cache_read_input,
                    "cache_creation_input": cache_creation_input,
                }),
            )
        }
        ServerEvent::ServiceTierReported { requested, tier } => {
            state.requested_service_tier = requested.clone();
            state.served_service_tier = Some(tier.clone());
            write_json_line(
                stdout,
                &serde_json::json!({
                    "type": "service_tier",
                    "requested": requested,
                    "tier": tier,
                    "report_status": state.service_tier_report_status(),
                }),
            )
        }
        ServerEvent::ConnectionType { connection } => {
            state.connection_type = Some(connection.clone());
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "connection_type", "connection": connection }),
            )
        }
        ServerEvent::ConnectionPhase { phase } => {
            state.connection_phase = Some(phase.clone());
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "connection_phase", "phase": phase }),
            )
        }
        ServerEvent::StatusDetail { detail } => {
            state.status_detail = Some(detail.clone());
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "status_detail", "detail": detail }),
            )
        }
        ServerEvent::MessageEnd { stop_reason } => write_json_line(
            stdout,
            &serde_json::json!({ "type": "message_end", "stop_reason": stop_reason }),
        ),
        ServerEvent::UpstreamProvider { provider } => {
            state.upstream_provider = Some(provider.clone());
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "upstream_provider", "provider": provider }),
            )
        }
        ServerEvent::SessionId { session_id } => {
            state.session_id = Some(session_id.clone());
            write_json_line(
                stdout,
                &serde_json::json!({ "type": "session", "session_id": session_id }),
            )
        }
        ServerEvent::Compaction {
            trigger,
            pre_tokens,
            messages_dropped,
            post_tokens,
            tokens_saved,
            duration_ms,
            messages_compacted,
            summary_chars,
            active_messages,
        } => write_json_line(
            stdout,
            &serde_json::json!({
                "type": "compaction",
                "trigger": trigger,
                "pre_tokens": pre_tokens,
                "messages_dropped": messages_dropped,
                "post_tokens": post_tokens,
                "tokens_saved": tokens_saved,
                "duration_ms": duration_ms,
                "messages_compacted": messages_compacted,
                "summary_chars": summary_chars,
                "active_messages": active_messages,
            }),
        ),
        ServerEvent::MemoryInjected {
            count,
            prompt_chars,
            computed_age_ms,
            ..
        } => write_json_line(
            stdout,
            &serde_json::json!({
                "type": "memory_injected",
                "count": count,
                "prompt_chars": prompt_chars,
                "computed_age_ms": computed_age_ms,
            }),
        ),
        ServerEvent::Interrupted => {
            write_json_line(stdout, &serde_json::json!({ "type": "interrupted" }))
        }
        ServerEvent::SoftInterruptInjected {
            content,
            display_role,
            point,
            tools_skipped,
            client_ids,
        } => write_json_line(
            stdout,
            &serde_json::json!({
                "type": "soft_interrupt_injected",
                "content": content,
                "display_role": display_role,
                "point": point,
                "tools_skipped": tools_skipped,
                "client_ids": client_ids,
            }),
        ),
        ServerEvent::BatchProgress { progress } => write_json_line(
            stdout,
            &serde_json::json!({ "type": "batch_progress", "progress": progress }),
        ),
        ServerEvent::Error {
            message,
            retry_after_secs,
            ..
        } => write_json_line(
            stdout,
            &serde_json::json!({
                "type": "error",
                "message": message,
                "retry_after_secs": retry_after_secs,
            }),
        ),
        ServerEvent::Ack { .. } | ServerEvent::Done { .. } | ServerEvent::Pong { .. } => Ok(()),
        _ => Ok(()),
    }
}

fn write_json_line(stdout: &mut impl Write, value: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

#[path = "commands/model_listing.rs"]
mod model_listing;

#[cfg(test)]
mod turn_envelope_tests;
pub use model_listing::run_model_command;

mod model_routes;
use model_routes::*;

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
