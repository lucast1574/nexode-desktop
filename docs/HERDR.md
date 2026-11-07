# Herdr integration contract

Nexode has built-in terminal routing for Herdr. When a headed session launch is requested from a client with `HERDR_ENV=1` and `HERDR_PANE_ID`, Nexode splits the calling pane to the right, focuses the new pane, and starts the resumed Nexode session there. `HERDR_BIN_PATH` is honored when present.

This covers visible swarm spawns, resume-in-new-terminal, self-development launches, and restart restores because they all use the shared terminal launcher. A configured `[terminal].spawn_hook` still takes precedence.

`pane run` starts the session inside the new pane's interactive shell, so the launcher runs `<nexode command> && exit`: the pane closes when Nexode exits cleanly (quit, `swarm stop`), and stays open on a non-zero exit so the error remains readable.

Inside a Herdr pane, the TUI also reports `idle`/`working`, the session id, and its `nexode --resume` command through `"$HERDR_BIN_PATH" pane report-agent`, and releases the pane on quit (`crates/nexode-tui/src/tui/herdr.rs`). Herdr older than 0.9.2 rejects the `--` resume separator as a usage error (exit 2) and drops the whole report. On that specific error Nexode retries once without the resume command and keeps reporting state only for the rest of the process.

## Current compatibility

Nexode already:

- forwards `HERDR_ENV`, `HERDR_SOCKET_PATH`, `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID`, `HERDR_BIN_PATH`, `HERDR_SESSION`, and `HERDR_AGENT` from the requesting client to server-side spawn and focus paths;
- recognizes Herdr as a masking terminal multiplexer for Mermaid graphics capability detection;
- exports stable lifecycle observer hooks for `session_start`, `session_end`, `turn_start`, and `turn_end`;
- exports `NEXODE_HOOK_SESSION_ID`, `NEXODE_HOOK_CWD`, event fields, and a JSON `NEXODE_HOOK_PAYLOAD`;
- resumes a native session with `nexode --resume <session-id>`.

## Recommended first Herdr integration

The initial upstream Herdr integration should provide **native session identity plus screen-manifest state**, matching Herdr's Claude Code and Codex model. Nexode's current hooks reliably identify session and turn boundaries, but do not yet provide a complete authoritative `blocked` lifecycle. Reporting only `working` and `idle` as lifecycle authority would suppress Herdr's screen fallback and make approval/question detection worse.

On Nexode `session_start`, the Herdr hook should send one newline-delimited JSON request to `HERDR_SOCKET_PATH`:

```json
{
  "id": "herdr:nexode:<unique-request-id>",
  "method": "pane.report_agent_session",
  "params": {
    "pane_id": "<HERDR_PANE_ID>",
    "source": "herdr:nexode",
    "agent": "nexode",
    "seq": 1,
    "agent_session_id": "<NEXODE_HOOK_SESSION_ID>",
    "session_start_source": "startup"
  }
}
```

The sequence must be monotonically increasing for the source. Map Nexode hook sources as follows where possible:

- `create` or `attach` to `startup`
- `resume` to `resume`

Herdr should restore the session with:

```text
nexode --resume <agent_session_id>
```

Nexode session IDs are opaque strings and fit Herdr's ID-based session reference model. No transcript path is needed.

## Required Herdr-side work

A first-class integration cannot be shipped only as a remote detection manifest. Herdr currently hard-codes known agent kinds, official session sources, restore commands, and install targets. The upstream implementation needs:

1. Add `nexode` to `IntegrationTarget`, CLI parsing, labels, command discovery, recommendations, status, install, and uninstall handling.
2. Install a config-safe Nexode session hook adapter without overwriting an existing user hook. If Herdr cannot safely compose the single Nexode hook command, coordinate a small multi-hook or native-emitter addition in Nexode first.
3. Accept `("herdr:nexode", "nexode")` as an official session source.
4. Persist its ID session reference and map it to `nexode --resume <id>` during restore.
5. Add Nexode process detection and a bundled screen manifest for idle, working, and blocked UI states.
6. Keep screen-manifest detection authoritative until Nexode exposes complete blocked, approval-result, interrupt, and exit transitions.
7. Add integration versioning, replacement-source handling, schema/UI wiring, install/uninstall tests, restore-plan tests, detection fixtures, and documentation.

Relevant upstream files as of Herdr commit `eacea2daf0b72973173b728936b27478374f2cd2`:

- `src/integration/{mod.rs,registry.rs,targets.rs,actions.rs,version.rs}`
- `src/integration/assets/`
- `src/api/schema/integrations.rs`
- `src/agent_resume.rs`
- `src/detect/mod.rs`
- `src/terminal/state.rs`

## Future full lifecycle authority

A later Nexode/Herdr protocol can report `working`, `idle`, `blocked`, and `unknown` through `pane.report_agent`, then call `pane.release_agent` on process exit. Do not enable this authority from turn hooks alone. It needs explicit Nexode events for permission/question blocking, approval resolution, cancellation/interrupt, reconnect/reload transfer, and abnormal termination so Herdr never displays a stale working or idle state.

Official references:

- <https://herdr.dev/docs/integrations/>
- <https://herdr.dev/docs/socket-api/>
- <https://herdr.dev/docs/agents/>
- <https://herdr.dev/docs/session-state/>
