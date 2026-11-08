//! Configuration file support for nexode
//!
//! Config is loaded from `~/.nexode/config.toml` (or `$NEXODE_HOME/config.toml`)
//! Environment variables override config file settings.

pub use nexode_config_types::{
    AgentsConfig, AmbientConfig, AuthConfig, AutoJudgeConfig, AutoReviewConfig, CompactionConfig,
    CompactionMode, CrossProviderFailoverMode, CustomWebSearchConfig, CustomWebSearchMethod,
    DiagramDisplayMode, DiagramPanePosition, DiffDisplayMode, DisplayConfig, FeatureConfig,
    GatewayConfig, HookCommands, HooksConfig, KeybindingsConfig, LatexRenderingMode,
    LaunchHotkeyEntry, LaunchHotkeysConfig, MarkdownSpacingMode, MenubarConfig, NamedProviderAuth,
    NamedProviderConfig, NamedProviderModelConfig, NamedProviderType, NativeScrollbarConfig,
    NotificationsConfig, PowerConfig, ProviderConfig, ReasoningDisplayMode, SafetyConfig,
    SessionConfig, SessionPickerResumeAction, SponsorsConfig, SwarmSpawnMode, SwarmStripLayout,
    TerminalConfig, UpdateChannel, WebSearchConfig, WebSearchEngine,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

const CONFIG_CACHE_CHECK_INTERVAL: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(500)
};

#[cfg(test)]
#[test]
fn kagi_websearch_documented_config_parses() {
    let docs = include_str!("../../../docs/kagi-web-search.md");
    let example = docs
        .split("```toml\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    let config: Config = toml::from_str(example).unwrap();
    assert_eq!(config.websearch.engine, WebSearchEngine::Kagi);
    assert!(!config.websearch.native_enabled());
    assert!(config.websearch.fallback_engines.is_empty());
    assert_eq!(config.websearch.kagi_api_key_env, "KAGI_API_KEY");
    let encoded = toml::to_string(&config.websearch).unwrap();
    let decoded: WebSearchConfig = toml::from_str(&encoded).unwrap();
    assert_eq!(decoded.engine, WebSearchEngine::Kagi);
    assert_eq!(decoded.kagi_api_key_env, "KAGI_API_KEY");
}

#[cfg(test)]
#[test]
fn custom_websearch_config_toml_roundtrip() {
    let input = r#"
engine = "custom"
prefer_native = false
fallback_engines = []
[custom]
endpoint = "https://search.example.test/api"
method = "POST"
query_param = "search"
count_param = "limit"
params = { format = "json" }
body = { options = { safe = true } }
headers = { Accept = "application/json" }
header_env = { Authorization = "SEARCH_AUTH" }
results_pointer = "/data/hits"
title_pointer = "/name"
url_pointer = "/link"
snippet_pointer = "/summary"
"#;
    let config: WebSearchConfig = toml::from_str(input).unwrap();
    assert_eq!(config.engine, WebSearchEngine::Custom);
    assert!(!config.native_enabled());
    assert_eq!(
        WebSearchEngine::parse("custom"),
        Some(WebSearchEngine::Custom)
    );
    assert!(WebSearchEngine::Custom.is_local());
    assert_eq!(WebSearchEngine::Custom.as_str(), "custom");
    let encoded = toml::to_string(&config).unwrap();
    let decoded: WebSearchConfig = toml::from_str(&encoded).unwrap();
    assert_eq!(
        serde_json::to_value(config).unwrap(),
        serde_json::to_value(decoded).unwrap()
    );
    let no_count: CustomWebSearchConfig =
        toml::from_str("endpoint = 'https://example.test'").unwrap();
    assert!(no_count.count_param.is_none());
    assert_eq!(no_count.method, CustomWebSearchMethod::Get);
    assert!(WebSearchConfig::default().custom.is_none());
}

#[cfg(test)]
#[test]
fn agents_jev_models_parse_and_default_unset() {
    let config: AgentsConfig = toml::from_str(
        r#"
memory_jev_model = "typesafe/jev-1.14"
browser_jev_model = "typesafe/jev-1.14"
voice_jev_model = "typesafe/jev-1.14"
"#,
    )
    .unwrap();
    assert_eq!(
        config.memory_jev_model.as_deref(),
        Some("typesafe/jev-1.14")
    );
    assert_eq!(
        config.browser_jev_model.as_deref(),
        Some("typesafe/jev-1.14")
    );
    assert_eq!(config.voice_jev_model.as_deref(), Some("typesafe/jev-1.14"));
    let defaults = AgentsConfig::default();
    assert!(defaults.memory_jev_model.is_none());
    assert!(defaults.browser_jev_model.is_none());
    assert!(defaults.voice_jev_model.is_none());
}

const CONFIG_ENV_KEYS: &[&str] = &[
    "NEXODE_HIBERNATION_IDLE_SECS",
    "NEXODE_HIBERNATION_MODE",
    "NEXODE_REQUEST_COMPRESSION",
    "HOME",
    "NEXODE_COMMIT_LOG_KEY",
    "NEXODE_RESUME_PROJECT_SCOPE",
    "NEXODE_SWARM_STALL_AFTER_SECS",
    "NEXODE_SWARM_STALL_NUDGE",
    "NEXODE_ACP_PROFILE",
    "NEXODE_ACP_TOOL_PROFILE",
    "NEXODE_ACTIVE_SESSIONS_MANAGER",
    "NEXODE_EXTERNAL_SESSIONS",
    "NEXODE_AMBIENT_ENABLED",
    "NEXODE_AMBIENT_MAX_INTERVAL",
    "NEXODE_AMBIENT_MIN_INTERVAL",
    "NEXODE_AMBIENT_MODEL",
    "NEXODE_AMBIENT_PROACTIVE",
    "NEXODE_AMBIENT_PROVIDER",
    "NEXODE_AMBIENT_VISIBLE",
    "NEXODE_ANIMATION_FPS",
    "NEXODE_AUTO_POKE",
    "NEXODE_AUTOJUDGE_ENABLED",
    "NEXODE_AUTOJUDGE_MODEL",
    "NEXODE_AUTOREVIEW_ENABLED",
    "NEXODE_AUTOREVIEW_MODEL",
    "NEXODE_AUTO_POKE",
    "NEXODE_AUTO_SERVER_RELOAD",
    "NEXODE_CHECK_UPDATES",
    "NEXODE_BING_API_KEY",
    "NEXODE_BLOCK_LID_CLOSE",
    "NEXODE_BING_API_KEY_ENV",
    "NEXODE_BING_MARKET",
    "NEXODE_CENTERED_TOGGLE_KEY",
    "NEXODE_CHAT_NATIVE_SCROLLBAR",
    "NEXODE_COMPACT_NOTIFICATIONS",
    "NEXODE_COPY_BADGE_ALT_LABEL",
    "NEXODE_COPY_SELECTION_TOGGLE_KEY",
    "NEXODE_COPILOT_PREMIUM",
    "NEXODE_GEMINI_FORCE_OAUTH",
    "GOOGLE_CLOUD_PROJECT",
    "GOOGLE_CLOUD_PROJECT_ID",
    "NEXODE_WAKE_MODE",
    "NEXODE_CROSS_PROVIDER_FAILOVER",
    "NEXODE_DEBUG_SOCKET",
    "NEXODE_DEFAULT_REASONING_DISPLAY",
    "NEXODE_DICTATION_COMMAND",
    "NEXODE_DICTATION_RECORDER",
    "NEXODE_DICTATION_KEY",
    "NEXODE_DICTATION_MODE",
    "NEXODE_DICTATION_TIMEOUT_SECS",
    "NEXODE_DIFF_LINE_WRAP",
    "NEXODE_DIFF_MODE",
    "NEXODE_DIFF_MODE_CYCLE_KEY",
    "NEXODE_DIAGRAM_PANE_TOGGLE_KEY",
    "NEXODE_DIAGRAM_PANE_VISIBILITY_TOGGLE_KEY",
    "NEXODE_DISABLE_BASE_TOOLS",
    "NEXODE_DISABLED_ANIMATIONS",
    "NEXODE_DISABLED_TOOLS",
    "NEXODE_DISCORD_BOT_TOKEN",
    "NEXODE_DISCORD_BOT_USER_ID",
    "NEXODE_DISCORD_CHANNEL_ID",
    "NEXODE_DISCORD_REPLY_ENABLED",
    "NEXODE_DISPLAY_CENTERED",
    "NEXODE_EFFORT_DECREASE_KEY",
    "NEXODE_EFFORT_INCREASE_KEY",
    "NEXODE_SPEED_DECREASE_KEY",
    "NEXODE_SPEED_INCREASE_KEY",
    "NEXODE_EMAIL_REPLY_ENABLED",
    "NEXODE_EMAIL_TO",
    "NEXODE_FOCUS_HOOK",
    "NEXODE_GATEWAY_BIND_ADDR",
    "NEXODE_GATEWAY_ENABLED",
    "NEXODE_GATEWAY_PORT",
    "NEXODE_HOME",
    "NEXODE_HOOK_PRE_TOOL",
    "NEXODE_HOOK_PRE_TOOL_TRANSFORM",
    "NEXODE_HOOK_PRE_TOOL_TRANSFORM_TIMEOUT_MS",
    "NEXODE_HOOK_PRE_TOOL_TIMEOUT_MS",
    "NEXODE_HOOK_POST_TOOL",
    "NEXODE_HOOK_POST_TOOL_FEEDBACK",
    "NEXODE_HOOK_POST_TOOL_FEEDBACK_TIMEOUT_MS",
    "NEXODE_HOOK_SESSION_END",
    "NEXODE_HOOK_SESSION_START",
    "NEXODE_HOOK_TURN_END",
    "NEXODE_HOOK_TURN_START",
    "NEXODE_IDLE_ANIMATION",
    "NEXODE_IMAP_HOST",
    "NEXODE_INFO_WIDGET_TOGGLE_KEY",
    "NEXODE_JADE_RELAY_API_BASE",
    "NEXODE_JADE_RELAY_ENABLED",
    "NEXODE_JADE_RELAY_LAUNCH_ENABLED",
    "NEXODE_JADE_RELAY_LAUNCH_WORKING_DIR",
    "NEXODE_JADE_RELAY_REPLY_ENABLED",
    "NEXODE_JADE_RELAY_SESSION_ID",
    "NEXODE_JADE_RELAY_TOKEN",
    "NEXODE_JADE_RELAY_TOKEN_ID",
    "NEXODE_JADE_RELAY_USER_ID",
    "NEXODE_KV_CACHE_MISS_NOTICES",
    "NEXODE_LATEX_RENDERING",
    "NEXODE_MARKDOWN_SPACING",
    "NEXODE_MEMORY_EMBEDDING_BACKEND",
    "NEXODE_MEMORY_EMBEDDING_BASE_URL",
    "NEXODE_MEMORY_EMBEDDING_DIM",
    "NEXODE_MEMORY_EMBEDDING_MODEL",
    "NEXODE_MEMORY_ENABLED",
    "NEXODE_MEMORY_JEV_PROVIDER",
    "NEXODE_BROWSER_JEV_PROVIDER",
    "NEXODE_VOICE_JEV_PROVIDER",
    "NEXODE_LAYA_BASE_URL",
    "LAYA_BASE_URL",
    "LAYA_API_KEY",
    "NEXODE_MEMORY_JEV_MODEL",
    "NEXODE_BROWSER_JEV_MODEL",
    "NEXODE_VOICE_JEV_MODEL",
    "NEXODE_ENABLE_MERMAID",
    "NEXODE_MEMORY_MODEL",
    "NEXODE_MEMORY_SIDECAR_ENABLED",
    "NEXODE_PERSIST_MEMORY_INJECTIONS",
    "NEXODE_MESSAGE_TIMESTAMPS",
    "NEXODE_MODEL",
    "NEXODE_MODEL_SWITCH_KEY",
    "NEXODE_MODEL_SWITCH_PREV_KEY",
    "NEXODE_MOUSE_CAPTURE",
    "NEXODE_NEW_TERMINAL_KEY",
    "NEXODE_NO_EMOJI",
    "NEXODE_NTFY_SERVER",
    "NEXODE_NTFY_TOPIC",
    "NEXODE_OPENAI_NATIVE_COMPACTION_MODE",
    "NEXODE_OPENAI_NATIVE_COMPACTION_THRESHOLD_TOKENS",
    "NEXODE_OPENAI_REASONING_EFFORT",
    "NEXODE_OPENAI_SERVICE_TIER",
    "NEXODE_OPENAI_TRANSPORT",
    "NEXODE_ANTHROPIC_REASONING_EFFORT",
    "NEXODE_PRESERVE_REASONING_CONTEXT",
    "NEXODE_PERFORMANCE",
    "NEXODE_PIN_IMAGES",
    "NEXODE_PIN_TODOS",
    "NEXODE_PREVENT_SLEEP_WHILE_STREAMING",
    "NEXODE_PROVIDER",
    "NEXODE_PROMPT_ENTRY_ANIMATION",
    "NEXODE_QUEUE_MODE",
    "NEXODE_REASONING_DISPLAY",
    "NEXODE_REDRAW_FPS",
    "NEXODE_SAME_PROVIDER_ACCOUNT_FAILOVER",
    "NEXODE_SCROLL_BOOKMARK_KEY",
    "NEXODE_SCROLL_DOWN_FALLBACK_KEY",
    "NEXODE_SCROLL_DOWN_KEY",
    "NEXODE_SCROLL_PAGE_DOWN_KEY",
    "NEXODE_SCROLL_PAGE_UP_KEY",
    "NEXODE_SCROLL_PROMPT_DOWN_KEY",
    "NEXODE_SCROLL_PROMPT_UP_KEY",
    "NEXODE_SCROLL_UP_FALLBACK_KEY",
    "NEXODE_SCROLL_UP_KEY",
    "NEXODE_SEARXNG_URL",
    "NEXODE_SHOW_AGENTGREP_OUTPUT",
    "NEXODE_SHOW_BASH_OUTPUT",
    "NEXODE_SHOW_DIFFS",
    "NEXODE_SHOW_THINKING",
    "NEXODE_SIDE_PANEL_TOGGLE_KEY",
    "NEXODE_SIDE_PANEL_NATIVE_SCROLLBAR",
    "NEXODE_SMTP_PASSWORD",
    "NEXODE_SPAWN_HOOK",
    "NEXODE_STREAM_IDLE_TIMEOUT_SECS",
    "NEXODE_MAX_RETRIES",
    "NEXODE_MCP_TOOLS",
    "NEXODE_MCP_TOOLS_TOKEN_THRESHOLD",
    "NEXODE_CODEMODE",
    "NEXODE_RETRY_BACKOFF_CAP_SECS",
    "NEXODE_SWARM_ENABLED",
    "NEXODE_SWARM_EFFORT",
    "NEXODE_SWARM_ROOT_EFFORT",
    "NEXODE_SWARM_MODEL",
    "NEXODE_SWARM_MAX_CONCURRENT_AGENTS",
    "NEXODE_SWARM_SPAWN_MODE",
    "NEXODE_SWARM_STRIP_LAYOUT",
    "NEXODE_TELEGRAM_BOT_TOKEN",
    "NEXODE_TELEGRAM_CHAT_ID",
    "NEXODE_TELEGRAM_REPLY_ENABLED",
    "NEXODE_TOOL_CALL_DETAILS",
    "NEXODE_TOOL_PROFILE",
    "NEXODE_TOOLS",
    "NEXODE_TRUSTED_EXTERNAL_AUTH_SOURCES",
    "NEXODE_TYPING_SCROLL_LOCK_TOGGLE_KEY",
    "NEXODE_UPDATE_CHANNEL",
    "NEXODE_VOICE_INPUT_KEY",
    "NEXODE_WEBSEARCH_ENGINE",
    "NEXODE_WEBSEARCH_FALLBACK_ENGINES",
    "NEXODE_WEBSEARCH_NATIVE_ALLOWED_DOMAINS",
    "NEXODE_WEBSEARCH_NATIVE_BLOCKED_DOMAINS",
    "NEXODE_WEBSEARCH_NATIVE_MAX_USES",
    "NEXODE_WEBSEARCH_PREFER_NATIVE",
    "NEXODE_WORKSPACE_DOWN_KEY",
    "NEXODE_WORKSPACE_LEFT_KEY",
    "NEXODE_WORKSPACE_RIGHT_KEY",
    "NEXODE_WORKSPACE_UP_KEY",
    "XDG_CONFIG_HOME",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigCacheFingerprint {
    path: Option<PathBuf>,
    modified: Option<SystemTime>,
    len: Option<u64>,
    env: Vec<(String, String)>,
}

impl ConfigCacheFingerprint {
    fn current() -> Self {
        let path = Config::path();
        let metadata = path.as_ref().and_then(|path| std::fs::metadata(path).ok());
        Self {
            path,
            modified: metadata
                .as_ref()
                .and_then(|metadata| metadata.modified().ok()),
            len: metadata.as_ref().map(std::fs::Metadata::len),
            env: config_env_fingerprint(),
        }
    }
}

struct ConfigCache {
    config: &'static Config,
    fingerprint: ConfigCacheFingerprint,
    last_checked: Instant,
    force_reload: bool,
}

static CONFIG_CACHE: LazyLock<RwLock<ConfigCache>> = LazyLock::new(|| {
    let config = leak_config(Config::load());
    // Fingerprint after the load: applying env overrides may set env vars
    // (e.g. copilot_premium -> NEXODE_COPILOT_PREMIUM), and fingerprinting
    // first would guarantee a spurious full reload on the next check.
    let fingerprint = ConfigCacheFingerprint::current();
    // Seed the global context-limit cache from named provider configs on first
    // load so every codepath (TUI info widget, compaction budget, model
    // switching) sees user-configured `context_window` values from the start.
    // Read from the loaded config directly to avoid recursing into config(),
    // which would deadlock on the still-initializing CONFIG_CACHE.
    populate_context_limits_from_config_ref(config);
    RwLock::new(ConfigCache {
        config,
        fingerprint,
        last_checked: Instant::now(),
        force_reload: false,
    })
});

fn leak_config(config: Config) -> &'static Config {
    Box::leak(Box::new(config))
}

/// Seed the global context-limit cache from a config reference directly.
///
/// Used during CONFIG_CACHE initialization (where calling config() would
/// deadlock) and shares its logic with
/// `crate::provider::populate_context_limits_from_config`.
fn populate_context_limits_from_config_ref(cfg: &Config) {
    crate::provider::populate_context_limits_from_config_value(cfg);
}

/// Get the global config instance.
///
/// The returned reference is backed by a reloadable process cache. Calls check
/// the config file path/metadata and relevant environment overrides on a short
/// throttle, not every frame. When those inputs change, the next checked call
/// reloads config.toml and invalidates dependent auth/model caches. Older
/// references remain valid for the duration of any in-flight operation.
pub fn config() -> &'static Config {
    let now = Instant::now();
    if let Ok(cache) = CONFIG_CACHE.read()
        && !cache.force_reload
        && now.duration_since(cache.last_checked) < CONFIG_CACHE_CHECK_INTERVAL
    {
        return cache.config;
    }

    let mut reload_reason = None;
    let mut changed_default_route = None;
    let config = {
        let mut cache = CONFIG_CACHE
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let now = Instant::now();
        if !cache.force_reload
            && now.duration_since(cache.last_checked) < CONFIG_CACHE_CHECK_INTERVAL
        {
            return cache.config;
        }

        let fingerprint = ConfigCacheFingerprint::current();
        cache.last_checked = now;
        if cache.force_reload || cache.fingerprint != fingerprint {
            reload_reason = Some(describe_config_reload(
                cache.force_reload,
                &cache.fingerprint,
                &fingerprint,
            ));
            let next = Config::load();
            // Direct file edits bypass set_default_model's account-pool sync.
            // Reconcile only a changed default in the same home, so unrelated
            // reloads never undo a deliberate pool ordering change.
            if cache.fingerprint.path == fingerprint.path
                && cache.config.provider.default_provider != next.provider.default_provider
            {
                changed_default_route = next.provider.default_provider.clone();
            }
            cache.config = leak_config(next);
            // Loading applies env overrides that can themselves set env vars
            // (e.g. copilot_premium propagates config -> NEXODE_COPILOT_PREMIUM).
            // Re-fingerprint after the load so those self-inflicted env changes
            // don't trigger a guaranteed second reload on the next check.
            cache.fingerprint = ConfigCacheFingerprint::current();
            cache.force_reload = false;
        }
        cache.config
    };

    // Auth lookups may consult config, so do this after releasing CONFIG_CACHE.
    if let Some(route) = changed_default_route
        && let Err(error) = crate::auth::account_pool::sync_order_with_default_route(&route)
    {
        crate::logging::error(&format!(
            "Could not sync account order after config edit: {error}"
        ));
    }

    if let Some(reason) = reload_reason {
        crate::logging::info(&format!("CONFIG_RELOAD {}", reason));
        // A config reload can change config-derived system prompt sections
        // (feature toggles, sponsors, ...), which legitimately invalidates the
        // KV cache prefix of warm sessions. Document it so a subsequent
        // harness-attributed cache miss is surfaced with this cause instead of
        // as an unexplained prompt mutation.
        crate::cache_invalidation::record("config reload", &reason);
        notify_config_reloaded();
        // Re-seed the global context-limit cache so user edits to named
        // provider `context_window` values take effect without a restart.
        crate::provider::populate_context_limits_from_config();
    }

    config
}

fn describe_config_reload(
    forced: bool,
    previous: &ConfigCacheFingerprint,
    next: &ConfigCacheFingerprint,
) -> String {
    let mut parts = Vec::new();
    if forced {
        parts.push("forced=true".to_string());
    }
    if previous.path != next.path {
        parts.push(format!(
            "path={:?}->{:?}",
            previous.path.as_ref().map(|p| p.display().to_string()),
            next.path.as_ref().map(|p| p.display().to_string())
        ));
    }
    if previous.modified != next.modified {
        parts.push("modified_changed=true".to_string());
    }
    if previous.len != next.len {
        parts.push(format!("len={:?}->{:?}", previous.len, next.len));
    }
    let env_changes = describe_env_changes(&previous.env, &next.env);
    if !env_changes.is_empty() {
        parts.push(format!("env=[{}]", env_changes.join(", ")));
    }
    if parts.is_empty() {
        "unchanged".to_string()
    } else {
        parts.join(" ")
    }
}

fn describe_env_changes(previous: &[(String, String)], next: &[(String, String)]) -> Vec<String> {
    let previous_map: BTreeMap<&str, &str> = previous
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let next_map: BTreeMap<&str, &str> = next
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let keys: BTreeSet<&str> = previous_map
        .keys()
        .chain(next_map.keys())
        .copied()
        .collect();

    keys.into_iter()
        .filter_map(|key| match (previous_map.get(key), next_map.get(key)) {
            (Some(previous), Some(next)) if previous != next => Some(format!(
                "{}:changed({}->{})",
                key,
                env_value_fingerprint(previous),
                env_value_fingerprint(next)
            )),
            (None, Some(next)) => Some(format!("{}:added({})", key, env_value_fingerprint(next))),
            (Some(previous), None) => Some(format!(
                "{}:removed({})",
                key,
                env_value_fingerprint(previous)
            )),
            _ => None,
        })
        .collect()
}

fn env_value_fingerprint(value: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    format!("len:{} hash:{:016x}", value.len(), hasher.finish())
}

fn config_env_fingerprint() -> Vec<(String, String)> {
    let mut values = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.to_string_lossy().to_string();
            if CONFIG_ENV_KEYS.contains(&key.as_str()) {
                Some((key, value.to_string_lossy().to_string()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    values
}

pub fn invalidate_config_cache() {
    let mut cache = CONFIG_CACHE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.force_reload = true;
    drop(cache);
    notify_config_reloaded();
}

fn notify_config_reloaded() {
    CONFIG_RELOAD_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    for listener in CONFIG_RELOAD_LISTENERS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
    {
        listener();
    }
}

/// Monotonic counter bumped every time the config cache reloads.
///
/// Callers that snapshot config-derived state (e.g. the TUI's parsed
/// keybindings) can poll this cheaply and re-derive their snapshot when the
/// generation changes, giving instant hot-reload of config edits without a
/// restart.
static CONFIG_RELOAD_GENERATION: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Current config reload generation. Increments after every cache reload.
pub fn config_reload_generation() -> u64 {
    CONFIG_RELOAD_GENERATION.load(std::sync::atomic::Ordering::Relaxed)
}

/// Listeners invoked after the config cache reloads.
///
/// Config is a foundational module, so instead of reaching up into higher-level
/// subsystems (auth cache, event bus) on reload, those subsystems register a
/// reaction here at startup. This keeps config free of upward dependencies and
/// breaks the config -> auth / config -> bus cycle edges.
/// Type of a config reload listener callback.
type ConfigReloadListener = fn();

static CONFIG_RELOAD_LISTENERS: LazyLock<RwLock<Vec<ConfigReloadListener>>> =
    LazyLock::new(|| RwLock::new(Vec::new()));

/// Register a callback to run after the config cache reloads.
///
/// Callbacks must be cheap and non-blocking; they run on whichever thread
/// triggers the reload. Intended to be called once per subsystem during
/// process startup.
pub fn on_config_reloaded(listener: fn()) {
    CONFIG_RELOAD_LISTENERS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(listener);
}

/// Scoped local resource bridge policy. Zero disables access.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudConfig {
    pub local_access_seconds: u32,
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            local_access_seconds: 86_400,
        }
    }
}

#[cfg(test)]
mod cloud_config_tests {
    use super::*;

    #[test]
    fn cloud_access_defaults_and_overrides() {
        let defaults: Config = toml::from_str("").unwrap();
        assert_eq!(defaults.cloud.local_access_seconds, 86400);
        for seconds in [0, 900, 172800] {
            let config: Config =
                toml::from_str(&format!("[cloud]\nlocal_access_seconds = {seconds}")).unwrap();
            assert_eq!(config.cloud.local_access_seconds, seconds);
        }
        assert!(toml::from_str::<Config>("[cloud]\nlocal_access_seconds = -1").is_err());
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Local resource access after cloud handoff.
    pub cloud: CloudConfig,
    /// Idle session memory hibernation policy.
    pub session: SessionConfig,

    /// Daemon behavior for autonomous wake requests.
    pub server: ServerConfig,

    /// Keybinding configuration
    pub keybindings: KeybindingsConfig,

    /// External dictation / speech-to-text integration
    pub dictation: DictationConfig,

    /// Display/UI configuration
    pub display: DisplayConfig,

    /// Feature toggles
    pub features: FeatureConfig,

    /// Web search tool configuration
    pub websearch: WebSearchConfig,

    /// Built-in tool exposure configuration
    pub tools: ToolConfig,

    /// Agent Client Protocol adapter configuration
    pub acp: AcpConfig,

    /// Auth trust / consent configuration
    pub auth: AuthConfig,

    /// Provider configuration
    pub provider: ProviderConfig,

    /// Named provider profiles, keyed by profile name.
    ///
    /// Example:
    /// [providers.my-gateway]
    /// type = "openai-compatible"
    /// base_url = "https://llm.example.com/v1"
    /// api_key_env = "MY_GATEWAY_API_KEY"
    pub providers: BTreeMap<String, NamedProviderConfig>,

    /// Agent-specific model defaults
    pub agents: AgentsConfig,

    /// Terminal window/pane spawning configuration
    pub terminal: TerminalConfig,

    /// Lifecycle hooks (external commands at turn/session/tool boundaries)
    pub hooks: HooksConfig,

    /// Ambient mode configuration
    pub ambient: AmbientConfig,

    /// Safety / notification configuration
    pub safety: SafetyConfig,

    /// Desktop notifications for interactive sessions (e.g. turn completion)
    pub notifications: NotificationsConfig,

    /// WebSocket gateway configuration (for iOS/web clients)
    pub gateway: GatewayConfig,

    /// Compaction configuration
    pub compaction: CompactionConfig,

    /// Power-management configuration (prevent sleep while streaming)
    pub power: PowerConfig,

    /// macOS menu bar session indicator
    pub menubar: MenubarConfig,

    /// Auto-review configuration
    pub autoreview: AutoReviewConfig,

    /// Auto-judge configuration
    pub autojudge: AutoJudgeConfig,

    /// Partner discovery configuration. Skipped when it matches the shipped
    /// default so saving config never bakes today's default into the file (see
    /// [`sponsors_is_default`]).
    #[serde(skip_serializing_if = "sponsors_is_default")]
    pub sponsors: SponsorsConfig,

    /// Global "launch a new nexode" hotkeys (macOS). Baked once by auto-import.
    pub launch_hotkeys: LaunchHotkeysConfig,

    /// `[desktop.*]` tables owned by Nexode Desktop (voice, workspace,
    /// appearance, ...). The CLI never interprets them, but it must round-trip
    /// them verbatim so a CLI settings save never wipes Desktop preferences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop: Option<toml::Table>,
}

/// Controls who owns autonomous wake execution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WakeMode {
    /// The daemon starts idle turns and interrupts running turns itself.
    #[default]
    Internal,
    /// The daemon emits a wake request and leaves turn scheduling to its operator.
    External,
}

impl WakeMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "internal" => Some(Self::Internal),
            "external" => Some(Self::External),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Ownership model for autonomous wake requests.
    pub wake_mode: WakeMode,
    /// Send mobile push notifications through the Nexode API when an account
    /// key is available. `None` means enabled; set `false` to opt out.
    pub mobile_push: Option<bool>,
}

/// Agent Client Protocol adapter configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AcpConfig {
    /// Client compatibility profile: "standard" (default), "extended", or "full".
    pub profile: String,
    /// Tool profile to request when `nexode acp` starts a daemon itself.
    pub tool_profile: String,
}

impl Default for AcpConfig {
    fn default() -> Self {
        Self {
            profile: "standard".to_string(),
            tool_profile: "acp".to_string(),
        }
    }
}

/// Controls how MCP server tools are exposed to the model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpToolsMode {
    /// Never change the cached tool list for MCP changes. Providers with
    /// native deferred loading (Claude, OpenAI gpt-5.4+) get MCP tools as
    /// deferred definitions; others get the fixed `mcp` search/call
    /// surface, with new tools described in the transcript.
    #[default]
    Auto,
    /// Always expose every MCP server tool as a top-level tool definition.
    /// Adding a server mid-session then invalidates the prompt cache.
    Eager,
    /// Same as `Auto` (kept for existing configs).
    Deferred,
}

impl McpToolsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Eager => "eager",
            Self::Deferred => "deferred",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "eager" => Some(Self::Eager),
            "deferred" => Some(Self::Deferred),
            _ => None,
        }
    }
}

/// Controls which tools are sent to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolConfig {
    /// Opt into Jev browser handoff. Direct browser actions remain available.
    pub browser_handoff: bool,
    /// Foreground time before automatic tool handoff, in milliseconds. Zero disables it.
    pub auto_background_timeout_ms: u64,
    /// Tool profile: "full" (default), "acp", "minimal"/"lite", or "none".
    pub profile: String,
    /// Explicit allow-list. When set, only these tools are exposed.
    /// Use "*" or "all" to expose all tools without an allow-list.
    pub enabled: Vec<String>,
    /// Tools to remove after applying profile/enabled.
    pub disabled: Vec<String>,
    /// Disable all built-in tools unless `enabled` is provided.
    pub disable_base_tools: bool,
    /// MCP tool exposure mode: auto (default), eager, or deferred.
    pub mcp_tools: McpToolsMode,
    /// Ignored. `auto` used to switch MCP exposure at this token estimate,
    /// which changed the cached tool list mid-session. Kept so existing
    /// configs and the env override still parse.
    #[serde(
        alias = "mcp_tools_threshold",
        alias = "mcp_tools_auto_threshold",
        alias = "mcp_tools_auto_threshold_tokens"
    )]
    pub mcp_tools_token_threshold: usize,
    /// Codemode: expose a `codemode` tool that runs model-written
    /// JavaScript in an embedded QuickJS sandbox able to call every other
    /// tool (including MCP tools). Enabled by default.
    pub codemode: bool,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            browser_handoff: false,
            auto_background_timeout_ms: 3_000,
            profile: String::new(),
            enabled: Vec::new(),
            disabled: Vec::new(),
            disable_base_tools: false,
            mcp_tools: McpToolsMode::Auto,
            mcp_tools_token_threshold: 8_000,
            codemode: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolSelection {
    pub allowed_tools: Option<HashSet<String>>,
    pub disabled_tools: HashSet<String>,
}

impl ToolConfig {
    pub fn selection(&self) -> ToolSelection {
        let mut allowed_tools = self.base_allowed_tools();
        let disabled_tools: HashSet<String> = self
            .disabled
            .iter()
            .map(|name| normalize_tool_name(name))
            .filter(|name| !name.is_empty())
            .collect();

        if let Some(allowed) = allowed_tools.as_mut() {
            for name in &disabled_tools {
                allowed.remove(name);
            }
        }

        ToolSelection {
            allowed_tools,
            disabled_tools,
        }
    }

    pub fn allowed_tools(&self) -> Option<HashSet<String>> {
        self.selection().allowed_tools
    }

    pub fn apply_to_allowed_set(&self, allowed: &mut HashSet<String>) {
        let selection = self.selection();
        if let Some(global_allowed) = selection.allowed_tools {
            allowed.retain(|name| global_allowed.contains(name));
        }
        for disabled in selection.disabled_tools {
            allowed.remove(&disabled);
        }
    }

    fn base_allowed_tools(&self) -> Option<HashSet<String>> {
        let (explicit, enables_all_tools) = self.normalized_enabled_tools();

        let profile = self.profile.trim().to_ascii_lowercase();
        if enables_all_tools {
            None
        } else if !explicit.is_empty() {
            Some(explicit)
        } else if self.disable_base_tools || matches!(profile.as_str(), "none" | "off" | "disabled")
        {
            Some(HashSet::new())
        } else if matches!(profile.as_str(), "acp") {
            Some(
                [
                    "bash",
                    "read",
                    "write",
                    "edit",
                    "apply_patch",
                    "agentgrep",
                    "ls",
                    "batch",
                    "mcp",
                ]
                .into_iter()
                .map(|name| name.to_string())
                .collect(),
            )
        } else if matches!(profile.as_str(), "minimal" | "lite" | "small") {
            Some(
                [
                    "bash",
                    "read",
                    "write",
                    "edit",
                    "apply_patch",
                    "agentgrep",
                    "ls",
                ]
                .into_iter()
                .map(|name| name.to_string())
                .collect(),
            )
        } else {
            None
        }
    }

    fn normalized_enabled_tools(&self) -> (HashSet<String>, bool) {
        let mut enabled = HashSet::new();
        let mut enables_all_tools = false;

        for name in &self.enabled {
            let normalized = normalize_tool_name(name);
            if normalized.is_empty() {
                continue;
            }
            if normalized == "*" || normalized.eq_ignore_ascii_case("all") {
                enables_all_tools = true;
            } else {
                enabled.insert(normalized);
            }
        }

        (enabled, enables_all_tools)
    }
}

fn normalize_tool_name(name: &str) -> String {
    let trimmed = name.trim().trim_matches('"');
    nexode_tool_types::resolve_tool_name(trimmed).to_string()
}

/// External dictation / speech-to-text integration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DictationConfig {
    /// Shell command to run. Must print the transcript to stdout.
    pub command: String,
    /// How to apply the resulting transcript.
    pub mode: crate::protocol::TranscriptMode,
    /// Optional in-app hotkey to trigger dictation.
    pub key: String,
    /// Maximum time to wait for the command to finish (0 = no timeout).
    pub timeout_secs: u64,
    /// Personal names or terms sent as recognition context to built-in voice
    /// transcription. Appended to the built-in vocabulary every user gets
    /// (Nexode names plus common coding agent terms), for this user only.
    pub vocabulary: Vec<String>,
    /// Built-in voice input: optional shell command that records the
    /// microphone and prints raw mono 16 kHz s16le PCM to stdout. Empty means
    /// auto-detect (pw-record, parecord, arecord, rec, ffmpeg).
    pub recorder: String,
    /// Keyboards `nexode companion` watches for Copilot-key push-to-talk
    /// (Linux evdev nodes, preferably stable `/dev/input/by-id/...` paths).
    /// Empty disables the companion's global voice listener.
    pub global_devices: Vec<std::path::PathBuf>,
}

impl Default for DictationConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            mode: crate::protocol::TranscriptMode::Send,
            key: "off".to_string(),
            timeout_secs: 90,
            vocabulary: Vec::new(),
            recorder: String::new(),
            global_devices: Vec::new(),
        }
    }
}

pub mod change_report;
mod config_file;
mod default_file;
mod display_summary;
mod env_overrides;

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "config_color_tests.rs"]
mod color_tests;

/// Whether integration discovery settings carry no information beyond the shipped
/// default, so `[sponsors]` can be left out of written config files.
///
/// Explicit opt-ins must be serialized so unrelated preference saves cannot
/// silently disable discovery on the next load.
fn sponsors_is_default(sponsors: &SponsorsConfig) -> bool {
    !sponsors.enabled && sponsors.endpoint == SponsorsConfig::default().endpoint
}
