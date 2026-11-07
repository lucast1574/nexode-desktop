# Lifecycle Hooks

nexode can run external commands at well-defined lifecycle points so other
programs can observe or gate agent behavior without forking nexode. Hooks
complement the [spawn hook](SPAWN_HOOK.md) (which controls *where headed
sessions appear*); lifecycle hooks tell you *what is happening inside them*.

## Configuration

```toml
# ~/.nexode/config.toml
[hooks]
turn_start    = "~/bin/nexode-turn-busy"       # observer
turn_end      = "~/bin/nexode-turn-notify"     # observer
session_start = ""                            # observer
session_end   = ""                            # observer
pre_tool      = "~/bin/nexode-tool-policy"     # gate
post_tool     = ""                            # observer
post_tool_feedback = "~/bin/nexode-vale"       # feeds output back to the model
pre_tool_timeout_ms = 5000
post_tool_feedback_timeout_ms = 10000
```

All eight command keys (`pre_tool_transform`, `turn_start`, `turn_end`,
`session_start`, `session_end`, `pre_tool`, `post_tool`, and
`post_tool_feedback`) accept either a command string or an array of strings.
Independent plugins can share an event without a dispatcher:

```toml
[hooks]
turn_end = ["~/bin/nexode-turn-notify", "~/bin/nexode-event-log"]
pre_tool = ["~/bin/nexode-tool-policy", "~/bin/nexode-audit-policy"]
```

Empty or whitespace-only commands are skipped. Observer commands are spawned
in declaration order but run independently, so completion order is unspecified.
Gate commands run sequentially in declaration order, even after a block, and
the first blocking result is returned. Transformers and feedback hooks also
run sequentially in declaration order, as described below.

Env overrides (always win; empty value disables a config hook):
`NEXODE_HOOK_TURN_START`, `NEXODE_HOOK_TURN_END`, `NEXODE_HOOK_SESSION_START`, `NEXODE_HOOK_SESSION_END`,
`NEXODE_HOOK_PRE_TOOL`, `NEXODE_HOOK_POST_TOOL`, `NEXODE_HOOK_PRE_TOOL_TIMEOUT_MS`,
`NEXODE_HOOK_POST_TOOL_FEEDBACK`, `NEXODE_HOOK_POST_TOOL_FEEDBACK_TIMEOUT_MS`,
`NEXODE_HOOK_PRE_TOOL_TRANSFORM`, `NEXODE_HOOK_PRE_TOOL_TRANSFORM_TIMEOUT_MS`.
Command overrides accept a command string or a TOML array of strings, for
example `NEXODE_HOOK_TURN_END='["notify-one", "notify-two"]'`.

## Common contract

- The hook command line is parsed shell-style (quotes and backslash escapes
  work) but executed **directly**, not through a shell. A leading `~/` in the
  program path is expanded.
- The hook runs in the session working directory when known.
- Every hook receives:

| Variable | Meaning |
| --- | --- |
| `NEXODE_HOOK_EVENT` | `turn_start`, `turn_end`, `session_start`, `session_end`, `pre_tool`, `post_tool`, `post_tool_feedback`, `pre_tool_transform` |
| `NEXODE_HOOK_SESSION_ID` | Session the event belongs to |
| `NEXODE_HOOK_CWD` | Session working directory |
| `NEXODE_HOOK_PAYLOAD` | JSON object mirroring all fields (capped at 16 KB) |
| `NEXODE_HOOKS_DISABLED` | Always `1`; suppresses hooks in nested nexode calls (recursion guard) |

## Observer hooks

`turn_start`, `turn_end`, `session_start`, `session_end`, and `post_tool` are
**observers**: spawned detached, fire-and-forget. The agent does not wait for
their completion; failures are only logged.

### `turn_start`

Fires before model generation and the first tool call in the `chat` entrypoint
(which uses the streaming turn loop) and one-shot `run` path. Extra fields:
`NEXODE_HOOK_MODEL` and `NEXODE_HOOK_SOURCE` (`chat` or `run`, respectively).

There are no distinct `resume` or `ambient` source values dispatched by the
current implementation. Those labels in older config comments are not a
runtime guarantee.

### `turn_end`

Fires when an agent turn completes (streaming turn path, which covers TUI,
desktop, swarm workers, and headless sessions, plus one-shot `nexode run`).

Extra fields: `NEXODE_HOOK_STATUS` (`ok`/`error`), `NEXODE_HOOK_DURATION_MS`,
`NEXODE_HOOK_MODEL`, `NEXODE_HOOK_LAST_ASSISTANT_TEXT` (first 4000 chars),
`NEXODE_HOOK_PROMPT_TOKENS`, `NEXODE_HOOK_INPUT_TOKENS`,
`NEXODE_HOOK_OUTPUT_TOKENS`, `NEXODE_HOOK_CACHE_READ_TOKENS`,
`NEXODE_HOOK_CACHE_WRITE_TOKENS`, and
`NEXODE_HOOK_ERROR` (on failure).

Token fields are summed from assistant messages in the current turn. `PROMPT_TOKENS` is the provider-reported full prompt size when available; `INPUT_TOKENS` is the raw provider-reported input count.
These values are not normalized across providers; for example, cached input may be included in the OpenAI input count
but is reported separately by Anthropic. Cache fields are omitted when no
assistant message in the turn reports that metric.

### `session_start` / `session_end`

`session_start` fires when an agent session becomes active, with
`NEXODE_HOOK_SOURCE` = `create` (brand new), `attach` (existing session object
attached), or `resume` (restored by id). `session_end` fires on normal close
(`NEXODE_HOOK_SOURCE=close`).

### `post_tool`

Fires after every tool call. Extra fields:

| Variable | Meaning |
| --- | --- |
| `NEXODE_HOOK_TOOL_NAME` | Resolved tool name |
| `NEXODE_HOOK_TOOL_CALL_ID` | Provider tool call id |
| `NEXODE_HOOK_TOOL_INPUT` | Tool input JSON (capped at 16 KB) |
| `NEXODE_HOOK_TOUCHED_PATHS` | Newline-separated absolute paths named by the input (`file_path`, `path`, `paths`, `apply_patch` headers, ...) |
| `NEXODE_HOOK_STATUS` | `ok` / `error` |
| `NEXODE_HOOK_DURATION_MS` | Tool execution time |
| `NEXODE_HOOK_OUTPUT_BYTES` | Output size (on success) |
| `NEXODE_HOOK_ERROR` | Error message (on failure) |

`NEXODE_HOOK_TOUCHED_PATHS` is derived from the tool input, so it lists paths a
tool was asked to read or write. It does not track files a `bash` command
happened to modify.

## Feedback hook: `post_tool_feedback`

`post_tool_feedback` runs **synchronously after every successful tool call**
and can add text to the result the model sees. It gets the same env fields as
`post_tool` plus the full tool input JSON on stdin.

- Trimmed **stdout** is appended to the tool result under a
  `[post_tool_feedback]` marker (capped at 8 KB per hook). Empty stdout adds
  nothing.
- The exit code is ignored, since linters usually exit non-zero when they
  report findings. Stderr is discarded.
- Timeouts (`post_tool_feedback_timeout_ms`, default 10s) and spawn failures
  add nothing and are logged.
- Multiple hooks run in declaration order and their outputs are concatenated.

Keep it fast and filter early: it runs on every tool call, including reads.

```bash
#!/usr/bin/env bash
# ~/bin/nexode-vale: lint prose the agent just wrote
case "$NEXODE_HOOK_TOOL_NAME" in
  write|edit|multiedit|apply_patch) ;;
  *) exit 0 ;;
esac
while IFS= read -r path; do
  case "$path" in
    *.md|*.mdx|*.txt|*.rst) [ -f "$path" ] && vale --output=line "$path" ;;
  esac
done <<<"$NEXODE_HOOK_TOUCHED_PATHS"
exit 0
```

## Gate hook: `pre_tool`

`pre_tool` runs **synchronously before every tool call** and can block it:

- The hook receives `NEXODE_HOOK_TOOL_NAME` plus the full tool input JSON on
  **stdin** (and a 16 KB-truncated copy in `NEXODE_HOOK_TOOL_INPUT`).
- **Exit 0**: allow the call.
- **Exit 2**: block the call. The hook's stderr (trimmed, capped at 2000
  chars) is returned to the model as the tool error, so the model can adapt.
- **Anything else fails open** with a logged warning: other exit codes,
  timeout (`pre_tool_timeout_ms`, default 5s), missing binary, spawn errors.

### JSON output (Codex/Claude compatible)

On exit 0 a gate may print one JSON object on stdout. Non-JSON stdout is
ignored, so plain exit-code scripts keep working. Recognized fields:

```json
{
  "hookSpecificOutput": {
    "permissionDecision": "deny",
    "permissionDecisionReason": "rm -rf is not allowed",
    "updatedInput": { "command": "ls" },
    "additionalContext": "note shown to the model"
  },
  "systemMessage": "also shown to the model"
}
```

- `permissionDecision: "deny"` (or legacy top-level `"decision": "block"`
  with `reason`) blocks the call with that reason.
- `permissionDecision: "ask"` also blocks, with a "requires user approval"
  reason. nexode has no interactive approval prompt for hooks yet.
- `updatedInput` (an object) replaces the tool input. With several gates,
  later gates receive the rewritten input. A block discards rewrites.
- `additionalContext` and `systemMessage` are appended to the tool result
  (or block error) under a `[pre_tool context]` marker.

Fail-open is deliberate: a broken policy script should degrade to "no policy"
rather than brick every session. If you need fail-closed semantics, make the
hook itself robust (it is your trust boundary, not nexode).

## Tool-input transformers: `pre_tool_transform`

Transformers are synchronous external plugins that can replace a tool's JSON
input before it is validated and executed. Configure one or more in order:

```toml
[hooks]
pre_tool_transform = ["~/.nexode/plugins/rtk-transform"]
pre_tool_transform_timeout_ms = 500
```

Each transformer receives the current complete tool-input JSON on stdin and
`NEXODE_HOOK_TOOL_NAME`, `NEXODE_HOOK_SESSION_ID`, and `NEXODE_HOOK_CWD` in its
environment. It may print a replacement JSON object to stdout. Empty stdout,
invalid JSON, a non-zero exit, a spawn error, or a timeout leaves the input
unchanged. This fail-open contract makes optional integrations safe to enable.

Transformers compose in configuration order and run before the `pre_tool`
policy gate, so policies evaluate the final transformed input. Keep plugins
small and tool-specific. For example, an RTK plugin should only change
`bash.command` when `rtk rewrite` returns a replacement.

### Example policy script

```bash
#!/usr/bin/env bash
# ~/bin/nexode-tool-policy
# stdin: tool input JSON. Env: NEXODE_HOOK_TOOL_NAME, NEXODE_HOOK_SESSION_ID...
input=$(cat)

case "$NEXODE_HOOK_TOOL_NAME" in
  bash)
    if grep -qE 'rm -rf /([^a-zA-Z]|$)|mkfs|dd if=' <<<"$input"; then
      echo "blocked: destructive shell command" >&2
      exit 2
    fi
    ;;
  write|edit)
    if grep -q '"file_path":"/etc/' <<<"$input"; then
      echo "blocked: writes to /etc are not allowed" >&2
      exit 2
    fi
    ;;
esac
exit 0
```

## Example: tmux status + desktop notification on turn end

```bash
#!/usr/bin/env bash
# ~/bin/nexode-turn-notify
if [ "$NEXODE_HOOK_STATUS" = ok ]; then icon=✅; else icon=❌; fi
tmux display-message "nexode $icon ${NEXODE_HOOK_SESSION_ID:0:12}" 2>/dev/null
notify-send "nexode turn $NEXODE_HOOK_STATUS" \
  "${NEXODE_HOOK_LAST_ASSISTANT_TEXT:0:120}" 2>/dev/null
exit 0
```

## Example: JSON event log of all hook activity

Point several hooks at one script and fan out on `NEXODE_HOOK_EVENT`:

```bash
#!/usr/bin/env bash
# ~/bin/nexode-event-log
echo "$NEXODE_HOOK_PAYLOAD" >> ~/.local/state/nexode-events.jsonl
```

```toml
[hooks]
turn_end      = "~/bin/nexode-event-log"
session_start = "~/bin/nexode-event-log"
session_end   = "~/bin/nexode-event-log"
post_tool     = "~/bin/nexode-event-log"
```

## Design notes

- Hook lookups are config-driven and re-read on config reload; you can add or
  change hooks without restarting nexode.
- Hot paths (`pre_tool`/`post_tool`) check whether a hook is configured before
  building any payload, so unconfigured hooks cost ~nothing.
- The recursion guard (`NEXODE_HOOKS_DISABLED=1`) means a hook may safely call
  `nexode` CLI commands without re-triggering hooks in that nested process.
