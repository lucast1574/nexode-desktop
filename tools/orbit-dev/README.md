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

`cargo xtask mermaid-fit-probe [--inner 73x46] [--image 1180x1470] [--font 8x16]`
reproduces the old/fixed diagram fit plans. `--png` decodes alpha bounds natively
in Rust; `--alpha-bbox L,T,R,B` accepts measured bounds. Camera and clipping
assertions return nonzero for failed conditions; a camera assertion without
visible content evidence is an error. No Pillow or Python is needed.

`cargo xtask verify-discovery-select [NEXODE_BINARY]` runs a real isolated Linux
daemon against a loopback-only catalog without inference calls. It verifies
browse/setup separation, explicit selection and off-catalog demand receipts.
Cleanup targets only processes carrying the private fixture workspace marker.

`cargo xtask measure-key-echo [NEXODE_BINARY] [--socket PATH] [--debug-socket PATH]`
uses a native vt100 emulator and an owned Unix PTY to distinguish first output
from the character actually appearing in the composer. It creates a new session
on the selected existing daemon, types eight characters without submitting a
prompt, and fails if any character never becomes visible. `--working-dir` and
`--json` support isolated acceptance. Capability replies survive split reads;
UTF-8 and composer history are recorded by the reader thread. Interruptions clean
up the owned client. Tests reject driver echo and measure delayed application echo.

`cargo xtask profile-real-spawn --binary PATH [--session ID] [--seconds 8]`
profiles an owned resumed client with native terminal emulation and Linux `perf`.
It prefers a recent session with content in the chosen working directory; absent
one, it creates a session on the selected existing daemon. `--socket`,
`--debug-socket`, `--working-dir` and `--perf` make that selection explicit.
Missing/failed perf evidence returns failure; cancellation kills the sampling
process group and closes the owned terminal. Unit tests do not attach perf to
any user process or claim a measured CPU improvement.

`cargo xtask bench-startup-visible-ready --tools nexode --runs 10` compares
visible startup and input readiness using native terminal emulation. It types
a short probe without submitting a prompt; selected installed tools still run
with their normal environment. Reports preserve missing runs and fail when
readiness is absent. Timing includes process creation. Tests use isolated shell
fixtures and do not launch third-party agents or use provider accounts.

`cargo xtask profile-single-spawn --binary PATH [--timeout 20] [--json]`
starts an owned temporary server and resumed PTY client, captures first-output
time and startup log lines, then closes both processes. Successful runs retain
the reported temporary log directory. Missing output or logs fail the check.
The native runner was also exercised against a real local Nexode binary with
a disposable HOME and a dummy credential, without submitting a model prompt.

`dump-fresh-spawn-screen`, `why-no-animation-area` and
`which-overlay-blocks-donut` now use a shared native isolated server/client
fixture, a disposable HOME and a dummy credential. No prompts are submitted.
They preserve screen row churn, layout/frame diagnostics and ordered overlay
queries. `--rows`/`--cols` control the real PTY size; `--fresh-session` actually
skips resume. An unavailable visual-debug frame is reported as such, not as
proof that an animation or layout assertion passed. All three commands were
exercised against a local Nexode binary; these are diagnostics, not performance
improvement claims.

`count-idle-draws [--window-s 3] [--type-slash] [--no-idle-animation]`
counts fresh draw samples independently of animation counters, measures actual
elapsed time and process CPU, and flags a saturated 240-sample buffer.
`pin-no-area-state [--samples 14] [--interval-s 1]` correlates draw samples with
current overlay/activity state. Both use the same isolated native UI fixture.
Malformed sample timestamps fail the measurement; an empty sample window is
reported as zero draws.

`measure-animation-cpu-cost` and `sweep-animation-fps` preserve the live-session
A/B and frame-rate comparisons. They explicitly enable the animation in the ON
case, reject absent animation/CPU/typing evidence, and expose `--socket`,
`--debug-socket`, `--working-dir` and `--session` for explicit selection.
Typing latency retains the original first-output definition: an animation frame
can satisfy it, so it is not a key-to-paint measurement. Load and missing samples
are reported. Native PTY/command-channel fixtures verify both ON/OFF paths;
validation does not attach to a user daemon or claim a CPU improvement.

`verify-light-theme [--binary PATH] [--seconds 12]` advertises black and white
terminal backgrounds through ordered OSC replies, captures fragmented truecolor
SGR sequences natively, and enforces the existing dim-text/accent contrast
checks. It isolates both daemons and homes. The local Nexode binary passed both
palettes with 19 distinct foreground colors per run. Color collection is
disabled in other probes so it does not alter their measurements.

`diagnose-idle-render-cost`, `probe-idle-state` and `profile-idle-donut`
replace the Python idle-state/CPU diagnostics. They use isolated native clients,
preserve monotonic repaint counters and blocker deltas, and keep strict input
preconditions. A missing composer makes the strict probe fail; it does not
produce a false successful latency measurement. The perf command retains both
flat and inclusive reports and its reported artifact directory, with bounded
interruptible subprocesses. Validation used an actual isolated client plus a
fake perf executable, not real CPU sampling.

`check-donut-animates-live` and `verify-donut-still-animates` explicitly enable
animations and establish an empty chat with local `/clear` before checking
rendered row motion. They reject inactive/missing rectangles or absent partial
repaints. The latter also checks slash-palette persistence and resume-overlay
cadence. Both passed with the local Nexode binary; the palette had no
disappearances and the overlay had no animation rectangle or moving rows.

`repro-slash-flicker` and `repro-input-flicker` now use native terminal history
and the shared screen/menu observers. Isolated typing uses a disposable
returning-user fixture so first-run onboarding cannot consume the test keys.
Each repeated input run gets a fresh server/client; `--live` creates a new session
on an explicitly selected existing daemon. `--keep` retains only artifacts,
never processes. Missing characters now fail rather than silently passing.
The local binary passed `/model` typing with all six characters visible and no
erasures; its slash palette stayed visible in every sample.

`repro-startup-lag` and `repro-real-spawn-lag` preserve startup-phase comparison
and resumed-session render-duty measurements. Both accept explicit socket and
working-directory selection. Startup comparison returns 3 when either phase
lacks samples, instead of inventing a conclusion. Real-spawn typing is still
first-output latency, explicitly labelled, and incomplete typing or saturated
sample buffers fail. Isolated daemon validation observed all 11 typed keys; the
short startup run was correctly inconclusive. The shared Python helper modules
and both of their remaining consumers have been removed.

`pair-service --host HOST [--bind 127.0.0.1:7644] [--run-as USER]` replaces the
legacy Python HTTP pairing fallback. It uses a private token file, constant-time
authentication, bounded native process execution and uncached JSON responses.
Bearer auth is supported alongside the legacy token query. Tests use real
loopback HTTP and a fixture CLI; no machine or cloud service is contacted.

`verify-login-qr /absolute/path/to/nexode` replaces the Python browserless
login acceptance matrix. It uses private temporary homes, piped/PTY stdin and
separate output streams; no browser or OAuth exchange is completed. The native
local CLI passed all eight cases, including headless auto-detection and JSON
output with QR opt-in set.

`verify-auth-import /absolute/path/to/nexode` replaces the Python credential
import CLI suite. Nine cases cover both providers, private storage, byte/inode/
mode preservation, invalid and oversized payloads, validation before stdin,
incremental EOF handling, concurrent one-winner import, PTY rejection and stalled
input timeout. All nine passed against the local CLI with synthetic secrets,
isolated homes and blocked provider proxies; no personal credentials were read.

`bench-nexode-maki --maki PATH --output result.json` replaces the Python/pyte
resource benchmark. `--tools nexode` can run Nexode alone without a Maki install.
Native process-tree accounting deduplicates roots and reports PSS, RSS, private
memory, CPU and rendered echo across isolated clients; the dedicated Nexode
server is included. Reports are written atomically after each seeded job.
Validation exercised a real Nexode daemon/client and a Maki CLI fixture, including
secret-env exclusion, no-splash configuration, all 20 echoes and process cleanup.
No real Maki performance comparison was run or claimed during migration.

`repro-idle-session-rss [--binary PATH] [--sessions 20] [--rounds 4]`
opens/closes native subscriptions and records server RSS, anonymous memory,
allocator statistics and a final memory report. Framed acknowledgements must
match the subscription, and owned servers are reaped on every exit. The default
uses synthetic credentials; `--source-home PATH` explicitly snapshots selected
local credentials/config privately without writable symlinks. `--trim` enables
optional gdb malloc trimming, and privilege elevation additionally needs
`--sudo`. Neither is used by validation. A short real-daemon check verified
protocol/accounting/cleanup, not long-term memory-retention behavior.

`heap-diff-idle-sessions --binary PATH [--sessions 30] [--rounds 4] [--settle 40]`
compares jemalloc live heaps before and after isolated idle-session churn on Linux.
The binary must support jemalloc profiling. Optional `--abrupt` and `--realistic`
exercise disconnect and bootstrap traffic; `--gdb PATH`, `--jeprof PATH` and
explicit `--sudo` configure the profiler. Private homes use synthetic credentials;
no personal credentials are copied. Artifacts include both heaps, debugger logs,
`diff.txt` and `diff_cum.txt`. Owned processes are cleaned on failure and interrupt.

`update-cloud-runtime BINARY [--host nexode-cloud] [--ssh-config PATH]`
stages a checksum-verified Linux runtime and dynamic libraries over SSH. The
optional `--activate-launcher` switches the backend symlink with rollback on
verification failure; it never reloads the daemon. Staging retries SSH transport
failures, but activation never retries automatically. Requires Linux, `ldd`, and
SSH access configured by the operator. Unit tests use local shell fixtures only.

`test-desktop-selfdev [--nexode-binary PATH] [--bridge-binary PATH]`
`[--desktop-repo PATH] [--output-dir NEW_DIRECTORY] [--compile-only]` compiles
and runs the real Rust SDK acceptance client. It checks five working-directory
paths, Desktop/CLI mode boundaries, prepared tools and prompts, context-only
persistence and reattachment. Build the runtime and harness API bridge first.
The SDK build is offline, so its dependencies must already be cached. Artifacts
must be outside both checkouts; all daemons use private homes and sockets with
no inherited credentials. `--compile-only` starts no runtime. Fixture tests
verify orchestration and cleanup separately from real runtime acceptance.

`test-caching-detailed [--debug-socket PATH] [--working-dir PATH]` sends the four
inherited caching prompts and emits per-turn usage, totals and cache diagnostics
as JSON. `test-oauth-usage [--debug-socket PATH] [--credentials PATH] [--claude PATH]`
`[--prompt TEXT] [--working-dir PATH]` compares Claude CLI and Nexode usage and
captures quota before, between and after their calls. These are explicitly live
commands and consume model quota when used against real services. Tests use
private fake sockets, synthetic tokens and loopback HTTP. `--usage-endpoint`
accepts only the default Anthropic endpoint or local HTTP fixtures. Missing usage
fails instead of reporting zero; sessions are destroyed even after failed turns.
The 10% cache-read weight is a diagnostic convention, not a pricing guarantee.

`generate-release-notes TAG [PREVIOUS_TAG]` renders the curated local changelog,
falling back to conventional-commit groups. It selects the previous release from
the numeric index, ancestor tags or sorted tags and appends the GitHub comparison
link. `scripts/generate_release_notes.sh` remains a compatible native launcher.
No release is created or published by this command.

### Runtime and cloud CLI acceptance

On Linux, run `orbit-dev runtime-upgrade-acceptance --binary /absolute/path/to/nexode`.
It launches credential-free daemons in private user/network namespaces, verifies
invalid/busy/same-version/downgrade refusals and a real exec/reconnect with exact
history and PID preservation. `--artifacts` must name a new short directory;
otherwise evidence is retained under `NEXODE_SCRATCH_DIR` or `~/.nexode/scratch`.

`NEXODE_SCRATCH_DIR=/short/path orbit-dev test-cloud-session-cli --binary /absolute/path/to/nexode`
checks real CLI status/wait/log/send parsing, adversarial prompt quoting, response
validation, redacted problem logs, read-only doctor and traversal rejection. Its
native transport parses the generated shell source without executing it. No cloud
account, provider, remote host or existing daemon is used.

`orbit-dev compile-time-probe [--skip-build] [--timing-html PATH] [--touch PATH]
[--profile selfdev] [--package nexode] [--bin nexode] [--feature-profile default]
[--json PATH] [--top 12]` replaces embedded Python in `compile_time_probe.sh`.
It preserves the Cargo timing JSON summary, including serial-stack spans and
frontend/codegen section durations, and bounds the build duration.

The demo scripts use `kitty-window id TITLE` / `kitty-window title ID` to read
Kitty JSON from stdin and `replay-keys FILE [--dry-run]` to replay recorded events
with native argument vectors. Dry-run emits JSON actions without typing keys.
`strip-selfdev-marker FILE` removes only the benchmark's exact trailing marker.

`orbit-dev expand-badge-headless --binary /absolute/path/to/nexode` starts a private daemon and real PTY client, renders the edit fixture and sends CSI-u Alt+Shift+E bytes. It checks Inline → FullInline without inserted text, preserving artifacts on failure.

`orbit-dev test-cloud-credentials /absolute/path/to/nexode [--case NAME]` checks 14 synthetic OAuth/router handoff scenarios using isolated local directories and pipes. It rejects secrets in diagnostics and archives (including gzip tar), verifies private file permissions, conflict rollback, dry-run behavior, refreshed-token return and one-way API-key transfer. `--case return_conflict_ownership_recovery` selects the ownership recovery scenario.

The compositor verifier now launches `capture-alt-shift-e OUTPUT` (native raw terminal mode with automatic restoration) and reads niri JSON using `niri-window app APP_ID` / `niri-window focused`. The keyboard capture is tested with a real PTY; the compositor check still requires an interactive niri/Kitty/wtype environment.
