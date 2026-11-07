# Native memory benchmarks

Run `scripts/orbit_dev.sh bench-memory-cli --sessions 5 --tools nexode_memory_off
nexode_memory_on --json-out results.json` for interactive process-tree memory, or
`bench-headless-memory --sessions 1 5 --turns 5 --tools nexode claude` for concurrent
persistent model sessions. Both commands run on Linux and use `/proc` PSS in MiB.
Headless turns call the configured models and can incur the provider's normal cost;
the test suite uses synthetic loopback providers and protocol fixtures instead.

Interactive tools retain the original ten choices: Nexode memory on/off, Pi,
Prime Agent, Codex, OpenCode, Copilot CLI, Cursor Agent, Claude Code and Antigravity.
A real PTY and terminal emulator answer terminal queries, wait for visible content,
type `jqx92` without Enter, require rendered input and erase the probe. Each session
reports actual first-visible and input-ready times separately from settling time.
PSS, RSS, process counts and unreadable-process counts accompany readiness results.
Missing tools, timeouts and absent input echo return failure, not a successful zero.

Headless protocols remain Nexode debug sessions, Claude stream JSON, Codex
app-server JSON-RPC, Pi RPC and OpenCode HTTP. All requested sessions complete each
turn concurrently, then memory is sampled before proceeding to the next turn. The
original five read-only prompts cycle for larger `--turns` values. These prompts
request read-only work; provider permission settings are not a sandbox guarantee.
Codex retains its explicit read-only sandbox. Results retain tool/session counts,
idle/turn samples, errors, version and tool/history/last-response checks. Errors
now also produce a nonzero process exit, while partial JSON is preserved.

The original model/account/binary overrides remain supported; `--timeout`,
`--settle`, `--nexode-bin` and `--claude-bin` are also explicit options. OpenAI
credentials are copied into private temporary Codex/OpenCode/Pi homes. Nexode auth
and config, and Claude credentials/settings, are copied instead of mutable symlinks
to the user's originals. Optional Nexode local embedding files are copied into the
benchmark home; large caches therefore increase setup time. The memory-on benchmark
reports embedding initialization status, including builds without that feature.
`NEXODE_BENCH_MEMORY_DUMP` retains the optional per-session-count daemon dump.

Process accounting includes owned descendants, process groups and detached workers
carrying a unique inherited benchmark marker. Cleanup verifies PID start times.
The former global Prime process-name kill is removed: existing unrelated workers
are neither counted nor terminated. Shared pre-existing services that do not belong
to the benchmark are intentionally outside its measurements.

`memory_bench_native` verifies Claude/Codex/Pi protocol turns, OpenCode HTTP sessions,
real PTY input echo and bounded silent-process timeouts. Set
`ORBIT_TEST_NEXODE_BINARY` to a built runtime for a two-session, two-turn actual
Nexode test against the local fake OpenAI provider. No real provider credentials
are needed for this validation.
