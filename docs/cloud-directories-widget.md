# Cloud directories info widget

SSH/cloud TUI sessions show a **Cloud Directories** card alongside **Cloud VM** when terminal margins have room. Local sessions do not show it.

- **Agents** counts live Nexode agent sessions, including idle sessions and swarm workers. Multiple agents sharing a daemon process count separately. Connected clients are not additional agents.
- **Busy** counts those agents currently inside a turn, including tool execution. Both agent turn loops retain their streaming presence guard for the entire turn.
- `>` marks the current project. Current projects sort first, followed by busy projects, other live projects and inactive projects. Up to six rows are shown with an overflow count.
- Paths use VM-home-relative notation where possible. Count columns stay aligned, paths truncate by terminal display cells, and rows never wrap.

## Directory inventory

Cloud transfers preserve absolute repository paths, so there is no universal `/workspace` root. The read-only VM probe combines persisted transfer lease repository roots, the current project, live sessions' working directories, immediate home directories, immediate project siblings, and projects immediately inside `~/workspaces`, `~/projects` and `~/repos`. It canonicalizes symlink aliases and resolves Git subdirectories to their repository root. Existing directories with no live agents remain visible. It does not recursively enumerate system or source directories, and excludes hidden sibling directories unless a lease or live session identifies them as projects.

The per-session active PID registry is shared across daemons using the same VM Nexode home. Live PID checks remove exited owners, migration leases exclude sessions moved elsewhere, and metadata excludes debug/canary sessions without excluding swarm workers. Custom daemons with separate `NEXODE_HOME` registries are outside that registry's scope.

Fresh idle sessions may not have persisted metadata yet. The attached session uses its authoritative current-directory hint. Other live sessions without metadata appear in a `(directory pending)` row until their directory is saved, rather than making the inventory unavailable or guessing their directory.

The collector runs inside the native Rust binary on the remote host. Current journal metadata is preferred; otherwise a bounded, deadline-aware reader extracts only top-level metadata fields from the snapshot and skips transcript/tool payloads. Nested tool/transcript fields cannot supply the working directory or debug flags.

## Refresh and failure behavior

Collection runs off the render thread through bounded native SSH commands, with the existing five-second pause between telemetry cycles. The inventory has its own stale flag: failed reads retain the last good snapshot and never replace counts with zeros. Initial failures show **Directories unavailable**. Reconnecting also marks displayed counts stale.

The probe bounds directory scans, metadata reads, runtime and SSH output. Incomplete active-session scans fail rather than publish partial counts. Directory-row truncation is explicitly marked.

## Verification

```sh
cargo test --locked -p nexode-remote-inspect
justrust test -p nexode-tui --lib cloud_ -- --nocapture
justrust test -p nexode-tui --test cloud_widget_api
```

## Rust inventory protocol

The SSH panel calls `nexode __remote-inventory <directory> <session>` using the configured `NEXODE_SSH_BINARY`. This read-only internal command exits before provider and daemon initialization. The remote host must run a version containing this command; older hosts report inventory unavailable/stale rather than falling back to Python. The standalone `nexode-remote-inspect` binary exposes the same collector for tests and diagnostics.
