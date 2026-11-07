# Orbit development tools

Rust replacements for repository maintenance scripts. From the repository root:

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
