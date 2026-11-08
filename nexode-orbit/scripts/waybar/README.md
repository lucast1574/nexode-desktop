# Waybar session status

Build `cargo build --release --locked --manifest-path tools/orbit-dev/Cargo.toml`
and install `tools/orbit-dev/target/release/orbit-dev` on PATH (or set
`ORBIT_DEV_BIN`). Install the `nexode-sessions-status` launcher into `~/.config/waybar/` and make it executable.
Configure a streaming custom module (do not set an interval):

```json
"custom/nexode-sessions": {
  "exec": "~/.config/waybar/nexode-sessions-status",
  "return-type": "json",
  "restart-interval": 5,
  "format": "{}",
  "tooltip": true
}
```

The streaming, session, and swarm counters remain local and use text labels.
Cloud adds **streaming/alive**, for example `Cloud 2/7`. The tooltip labels both counts.
Cloud counts include root, detached, and internal/swarm sessions on the account's
managed machine and running ephemeral machines. Arbitrary third-party SSH hosts
are not included.

Cloud snapshots read the same `active_pids` and `streaming_pids` registries used
by Nexode's presence indicator, checking PID liveness and counting sessions rather
than unique daemon PIDs. No debug-control permissions or daemon restarts are
needed. Streaming means a model response is generating, not merely that an agent
has a running lifecycle status. Dead-owner markers are ignored.

Cloud polling runs independently of local socket events, every 30 seconds after
the previous probe completes. API or SSH failures show `?/?`, not misleading zero
counts. Machine inventory/status queries are read-only. SSH is attempted only
for machines reported as running, using Nexode's existing private SSH configs and
host-key verification. The managed SSH proxy may authorize the connection, and
an SSH connection can affect the host's idle timer. No wake command is issued,
although a host stopping between the status check and proxy connection can race
with the proxy's normal wake behavior.

Requirements: `~/.local/bin/nexode`, account login, prepared cloud SSH configs,
SSH, and a native `orbit-dev` executable on PATH on each running cloud host.
Use `--remote-binary /absolute/path/to/orbit-dev` for a different remote location.
The executable must match the remote operating system and architecture. Missing
remote tooling produces unknown counts; this module never installs it or runs Python.
Use `--no-cloud` for local-only status without API or SSH calls. `--once` emits one
snapshot. `--cloud-probe --home PATH` reads an isolated local presence registry.

Run regression tests:

```sh
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml waybar::
```
