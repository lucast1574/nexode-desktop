# Orbit development tools

Rust replacements for repository maintenance scripts. From the repository root:

`cargo xtask post-discord-release --tag TAG --release-file release.json` previews
release announcements offline. Omit `--release-file` only for an intended live
announcement with `GH_TOKEN`, `GITHUB_REPOSITORY` and `DISCORD_RELEASE_WEBHOOK`.
The release workflow remains guarded by the configured canonical repository.

```sh
cargo xtask check-branding
cargo xtask check-modules
cargo xtask check-boundaries
cargo xtask check-code-size
cargo xtask check-test-size
cargo xtask score-plans plans.json scores.json
cargo xtask version-sdk v1.0.0-preview.42
cargo xtask timeout 900 cargo test --locked -p nexode-storage
cargo xtask package-browser /path/to/browser-agent-bridge/extension /path/to/dist
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml
```

`version-sdk` edits the SDK and runtime manifests in this checkout. It does not publish
packages or change npm visibility. Preview builds keep the `preview` distribution tag.
The SDK remains TypeScript.

`timeout` executes an argument vector directly, returns the command's exit status,
and kills the process tree when the timeout is reached (exit 124). On Unix it creates
an isolated process group; Windows uses the system taskkill utility. Compilation of
this tool happens before the timeout begins.

`package-browser` expects an existing external Bridge extension source checkout.
It emits Firefox XPI, Chromium ZIP and Safari ZIP packages using Rust, preserving
the upstream public Chrome key, native-host identity, browser permissions and license.
It does not download source, modify a browser profile, sign packages, or publish them.
The Safari ZIP still needs the platform's app conversion/signing before installation.
The external bridge host and CLI are already Rust; extension JS remains responsible
for browser APIs and page DOM execution.

`log-build` is an internal command used by `scripts/dev_cargo.sh`. It appends a JSONL
record from `NEXODE_LOG_*` environment variables and arguments. Newly created logs
use mode 0600 on Unix. The wrapper uses a separate Cargo target directory so logging
does not contend with the agent's build directory. No Python is involved.

The tool has its own Cargo workspace and lockfile to avoid compiling the agent's UI,
providers and native integrations for maintenance commands.

The size checks preserve the existing baselines, reject new oversized files and growth,
and only rewrite a baseline with an explicit `--update`. They may expose existing
size debt; migration does not silently relax those gates.

Safety budgets: `cargo xtask check-panics`, `check-swallowed-errors`, and `check-wildcards`. Checks preserve existing baselines. Use `--dump` to inspect counts and `--update` only for intentional baseline changes.

`cargo xtask update-rehearsal --help` runs the local updater sandbox in Rust. It supports rate limits, one dropped download, bad checksums, manual/up-to-date scenarios, preparation and headless checks. Only marked sandbox directories can be reset; the default is `nexode-update-rehearsal-rust` under the user cache.

Diagnostics: `cargo xtask find-unlocked-env-tests [source-directory]` and `cargo xtask tb-compare <jobs-directory> [baseline.tsv]`.

Repository assets: `cargo xtask downloads-badge` and `cargo xtask star-history` use `gh api` pagination. Supply `--repo OWNER/REPO` or `GITHUB_REPOSITORY`. `--input pages.json` allows offline verification; `--output` changes the destination. The chart accepts `--today YYYY-MM-DD` for reproducible rendering.

Architecture reports: `cargo xtask compile-isolation [--json] [--top N] [--strict-target-state]` and `cargo xtask analyze-root-crate [--json] [--full]`. Both are read-only; isolation failures are advisory unless strict mode is requested.

Local test servers: `cargo xtask browser-fixture [--port N]` and `cargo xtask fake-openai-server <port>`. The provider requires `FAKE_LOG` and supports `FAKE_LABEL`, `FAKE_CONTROL_DIR`, and `FAKE_PORT_FILE`. Both bind only loopback. `scripts/orbit_dev.sh --build-only` prepares a native binary before tests clear their environment.

Live browser acceptance is opt-in: `BROWSER_SESSION=... cargo xtask test-browser-handoff-live --tab-id ID`. It requires an existing disposable fixture/about:blank tab and configured inference credentials. Unit tests never run this live command.

`NEXODE_SCRATCH_DIR=/isolated/scratch cargo xtask test-cloud-detached-turn --binary /absolute/path/to/nexode` exercises a real detached turn against the native fake provider, busy rejection, completion, private logs and session inspection. It uses Linux process ownership markers for cleanup.

`cargo xtask bench-resume-picker /absolute/path/to/nexode --runs 3` measures picker loading, population and terminal restoration using an owned Unix PTY. Add `--cancel-loading` to measure cancellation. `cargo xtask bench-resume-suite --samples 20` runs the offline Rust benchmark matrices with the existing Justrust build runner and preserves metadata, raw logs and structured results; `--binary PATH` adds the PTY measurements.

`cargo xtask bench-startup /absolute/path/to/nexode --runs 3 --check` measures help/version, socket readiness and cold-client startup profiles in isolated Linux sandboxes. It uses a native PTY, enforces the original configurable budgets, rejects missing required measurements, and cleans up only processes bearing its unique workspace marker. It requires neither Python nor `script`/`timeout`. Budget failures exit with status 2.

Discovery and attribution fixtures are native Rust:

- `cargo xtask mock-sponsor serve --db /tmp/sponsor.sqlite` starts the local reference service. The `signup`, `confirm`, and `account` subcommands exercise signed, expiring, one-use links and immutable first-acquisition attribution in SQLite.
- `cargo xtask benchmark-attribution --catalog-file catalog.json` evaluates saved entries; `--live --endpoint URL` checks browse/select and `--live-web` also checks referral redirects. Missing or unknown CLI attribution fails the gate, even with a lower `--min-score`.
- `cargo xtask openrelay-discovery-server --ready-file /tmp/discovery-ready.json` serves the local OpenRelay listing. `openrelay-check` validates readiness metadata, RPC chain identity, and recorded agent traces for the existing live shell runner.

`cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --test discovery_services` runs the public CLI/HTTP round trips with an empty PATH. It creates no real sponsor accounts or external model calls.

`cargo xtask demo-shop` replaces the offline storefront simulator. It preserves search, cart, budget checks and the intentionally paused checkout (exit 3), with integer-cent totals and atomic state writes. The demo launcher exposes the built native binary as `nexode-demo-shop`; `demo-session` selects its new session without an embedded interpreter.

`cargo xtask bench-reload-storm --socket /path/to/server.sock -n 40 [--home PATH] [--chunk BYTES]` concurrently resumes selected sessions and measures arrival of history. The Rust async client preserves target-session subscriptions and fixed-size chunk experiments, handles JSON spacing, and reports failures as a nonzero exit instead of silently passing an empty or failed run.

Application tests use `orbit-fixture` for authentication subprocesses, browser session daemons, MCP stdio, SSH frames and bridge restart/rollback. `test_support.rs` builds that binary in this independent workspace when needed; CI or local runs can prebuild it and set `ORBIT_TEST_FIXTURE_BIN` to its absolute path. No provider credentials or real browsers are required. Voice process-name tests re-execute their own Rust test binary.

Release gates: `cargo xtask check-release-ci SHA --repo OWNER/REPO --timeout 0`
checks the latest exact-commit push CI attempt. Preview-only advisory exceptions
require explicit `--preview` and complete step evidence. `preview-release`,
`reconcile-preview-release`, and `verify-preview-arm-runtime` preserve the
existing gated preview workflow; publication remains intentionally unconfigured.
The ARM runtime command requires a native Linux ARM64 runner and executes only
the uploaded archive. Tests use local repositories and fake GitHub responses.

`apply-release-recovery` retains the exact-byte historical repair allowlist and
Git identity checks; it records the hashes of release helpers from the trusted
workflow commit. `verify-public-release TAG --sha SHA` checks complete metadata,
downloaded hashes and unpatched provenance; `--allow-draft` is explicit. Release
jobs build the maintenance tool from the workflow commit outside the tag checkout,
so historical tags never need to contain the current tooling.

Native handoff: `native-handoff seal|verify|smoke --lane linux-arm64|macos-arm64`
binds binaries to the exact source, run, attempt, toolchain and build receipt.
`native-handoff-preflight` reads only GitHub environment policies and secret names.
`sign-release --help` replaces the external Python signer; it plans by default
and only signs with explicit `--execute` on the target operating system.
`signing-check` supplies shared identity, keychain, smoke and receipt guards.
Signing workflows build `ORBIT_DEV_BIN` before exposing protected credentials.

OAuth diagnostics: `cargo xtask oauth-helper` creates a private PKCE flow valid for
15 minutes; `cargo xtask oauth-helper --exchange` reads the returned code or callback
from stdin and atomically saves credentials, preserving unrelated keys. Pending state
lives in `$NEXODE_HOME/oauth-helper.json` (default `~/.nexode`).
`cargo xtask oauth-probe headers|identity|originator [MODEL] --dry-run` previews the
original diagnostic request variants without credentials or network traffic. Omit
`--dry-run` only to explicitly make live provider calls. `--auth-file` and `--account`
select credentials; responses redact the access token. Unit tests use loopback servers.

`cargo xtask memory-snapshot [--json] [--include-aux] [--socket PATH]` reports
Linux smaps RSS/PSS, anonymous, shared/private and swap memory by server/client.
`cargo xtask monitor [--socket PATH]` displays the Unix debug event dashboard,
with subscription, periodic pings, automatic reconnect and bounded UTF-8 framing.
Both commands replace their Python diagnostics; neither starts an inference turn.

`cargo xtask test-ci-suites [all|lib-bins|provider-matrix|e2e] [--parallel]` runs
the existing CI suites with locked dependencies, progress events and per-suite
timeouts. Unix interruption/timeout terminates the owned process group.
`--timeout-scale` adjusts budgets; execution stops at the first failed suite.
Hosted Linux compilation now invokes `validate-preview-version` and
`verify-hosted-linux-abi` for native version/ELF/GLIBC/OpenSSL/TLS checks. Its offline
Rust orchestration tests preserve the x86 and ARM failure matrix; they do not
prove a real tenant-hosted build or native ARM runtime behavior.

`cargo xtask audit-terminal-bench CAMPAIGN [--min-trials 5]` audits Harbor
configuration, nested resource overrides, trial artifacts and suspicious log
mentions offline. Invalid configs cannot appear in the submit-ready group.
Partial runs remain reported separately. This does not submit benchmark results.

`cargo xtask waybar-status [--no-cloud] [--once]` emits native Waybar JSON.
It subscribes to local debug events and refreshes cloud counts independently.
Remote checks require a matching native `orbit-dev` on already-running hosts;
`--remote-binary PATH` selects it. `--cloud-probe [--home PATH]` counts presence
markers locally, including shared-daemon sessions. No remote Python is invoked.

`cargo xtask remote-check --host HOST --working-dir PATH [--code CODE]` replaces
the HTTP pairing/WebSocket diagnostic, preserving private token caching and
subscribe/history/agent-turn checks. It answers pings and correlates completion IDs.
It performs a live remote turn only when explicitly run; tests use loopback.

`cargo xtask compare-token-usage [--runs N] [--json] [--verbose]` performs the
existing three-prompt live Nexode/Claude CLI comparison. Binary and model flags
allow explicit matching installations. Both output modes now fail for missing
usage, partial runs or more than 50% total-token overhead; no live providers are
used by unit tests. Undefined percentage differences serialize as JSON null.

`cargo xtask fuzz-niri-insert-point [SEED] [COUNT] [--output PATH]` generates
the same KDL stress constructs and keeps only configurations accepted by the real
`niri validate`. The Rust seed stream is deterministic but differs from Python.
The default corpus remains `~/.nexode/scratch/fuzz719/valid_inputs.json`; validation
timeouts fail instead of silently treating tool failures as valid inputs. This
command generates a corpus; the setup-hints Rust tests verify insertion behavior.

`verify-browser-cold-start` replaces the real Firefox acceptance script. It
requires PID 1 in a private Linux PID/network namespace, copies only bridge and
extension assets into a fresh profile, establishes the zombie precondition in
Rust and writes the same readiness/tab evidence. SIGINT/SIGTERM explicitly exit
namespace PID 1 to clean up browser descendants. Migration tests use fixtures;
a real Firefox acceptance run remains a separate opt-in check.
