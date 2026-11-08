# Native Discovery, browser and swarm benchmarks

The runners below are implemented in `tools/orbit-dev`. Use `cargo xtask COMMAND`
or `bash scripts/orbit_dev.sh COMMAND`; no Python interpreter is used by their
orchestration, case validation, trace parsing, HTTP fixtures or scoring.

## Discovery

`benchmark-discovery` preserves catalog coverage validation, natural prompt checks,
case/tool filtering, full-tool and discovery-only modes, retry-until-hit positive
trials, single-attempt negative controls, deadline cancellation and version-2 JSON
reports. Both historical Discovery and current Integration output vocabulary are
accepted. A direct selection fails a listing trial and stops it immediately.
Use `--catalog-file FILE --dry-run` for a completely offline catalog gate.

`benchmark-discovery-rate` preserves independent trials, case/tag filters, browse
recall, negative-control precision, exact selection receipts, invalid-provider
retry/backoff, binary SHA-256/version identity and version-2 reports. Selection
requires matching action, product, category, listed status and a substantive
reason. Bypass detection only inspects agent tool input. Off-catalog receipts now
populate `off_catalog_selects`; the previous runner defined that field but never
filled it. Every retry receives a fresh workspace, including invalid retries.

`compare-discovery-rate BEFORE.json AFTER.json` prints the same metric comparison
as the former embedded script. `scripts/compare_discovery_rate.sh` invokes both
native runners and this reporter. Live commands still use explicitly selected
provider credentials and can incur inference costs; offline validation does not.

## Browser handoff

`benchmark-browser-handoff --self-test` exercises trace interpretation, paired
scoring and real loopback HTTP requests for all six task/phase combinations. It
does not launch a browser or inference. Live options are described in
[`scripts/benchmark_browser_handoff.md`](../scripts/benchmark_browser_handoff.md).

Live runs retain the exact-binary fingerprint, prepared isolated home requirement,
existing browser-session requirement, exclusive tab lock, loopback fixture-tab
preflight, alternating pair order, direct-action guard, independent final DOM
verification, synthetic form/search validation and transcript/server artifacts.
Missing model-call counts, token totals or prices remain unknown. A successful
fixture request does not by itself prove a correct visible page. Failures remain
in all-attempt counts and timings. Unlike the old script, a run with invalid trials
returns exit 1, making failures usable in automation; the speed target remains a
separate report field. No live paid/browser comparison was run for this migration.

## Swarm coordination and external take-home benchmark

`test-swarm --fresh --binary PATH` (alias `test-swarm-debug`) runs against an
isolated real daemon. `--socket DEBUG_SOCKET` targets an existing daemon while
creating and cleaning its own temporary sessions/workspace. Checks cover current
independent headless root swarms, no automatic headless coordinator election,
member timestamps, explicit-scope broadcast, DM rejection, non-git provenance,
proposal/context timestamps, unauthorized approval/rejection, diagnostic schemas,
event queries and help. Empty touch/conflict lists are reported as empty
observations, not proof of recording.

The former script assumed that sessions sharing a directory also shared a swarm
and that the first headless session became coordinator. Both contradict the
current runtime's explicit isolation rules. Coordinator plan validation remains
covered by the real `server::comm_plan::tests` runtime suite (10 tests), including
valid approval, dependency cycles and no mutation on rejected plans. Run it with:

```sh
cargo test --locked --no-default-features -p nexode-app-core --lib server::comm_plan::tests
```

The migrated checks exposed and fixed three integration defects: command-handler
errors closed the debug transport before replying; context lookup split inside
`session:<id>` swarm IDs; explicit broadcasts recognized path IDs but not current
session IDs. Regressions cover replies followed by a ping on the same connection,
colon-preserving context lookup and explicit broadcast scope.

`benchmark-swarm [single|swarm|both]` (alias `benchmark-takehome`) preserves the
asynchronous single-agent/swarm comparison, cycle sampling, timing, historical
thresholds and JSON results. It accepts `--single-only`, `--swarm-only`,
`--source`, `--socket`, `--timeout` in minutes, `--check-interval` in seconds,
`--output`, and `--evaluator-command`. `TAKEHOME_SOURCE` and `BENCHMARK_TIMEOUT`
remain supported. Workspaces are newly created copies; existing ones are never
deleted. Jobs are cancelled before final evaluation, and cleanup is restricted
to owned session/swarm identities. Failed or missing cycle evaluations cannot
masquerade as the historical baseline. Reports distinguish best observed cycles
from final artifact cycles.

The **external** `original_performance_takehome` challenge itself is Python and
is not part of this repository. Its default evaluator remains the explicit argv
`python3 tests/submission_tests.py -v`; override `--evaluator-command` for another
dataset/evaluator. Rust runs that external command directly, without a shell.
The orchestration was tested with a native socket fixture and a shell evaluator,
including both arms, owned cleanup, errors and source preservation. No paid
optimization run was performed.

## Validation

```sh
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --bin orbit-dev discovery::benchmark
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --test discovery_benchmark
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --bin orbit-dev browser_fixture::benchmark
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --test swarm_benchmark
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --bin orbit-dev swarm_benchmark
```

Discovery coverage includes actual subprocess streaming, early decision
termination, deadlines, schema/leakage checks, historical vocabulary, selection
discipline, auth-failure exclusion and CLI report generation. The migrated swarm
coordination suite also passed against the rebuilt runtime.
