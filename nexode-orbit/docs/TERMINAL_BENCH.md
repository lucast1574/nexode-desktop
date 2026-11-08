# Native Terminal-Bench runner

`orbit-dev terminal-bench` runs Harbor-format Linux tasks directly through Docker.
The runner, provider adapters, result parsing and campaign bookkeeping are Rust;
Harbor, uvx and a host Python interpreter are no longer required. Task verifiers
may still install their own dependencies inside disposable task containers.

```bash
scripts/run_terminal_bench_harbor.sh --path /tmp/terminal-bench-2 \
  --include-task-name regex-log --n-concurrent 1 -k 1 \
  --jobs-dir /tmp/nexode-tb2 --job-name regex-log-pilot
scripts/run_terminal_bench_claude.sh --path /tmp/terminal-bench-2.1 \
  --include-task-name regex-log --dry-run
```

The shell entrypoints delegate to `scripts/orbit_dev.sh`. Use `--help` for the
native options. `--dry-run` validates the task configuration without starting
containers, reading provider credentials or making inference requests.

Each attempt gets a new container and isolated Nexode home. The runner stages
the binary, sibling OpenSSL libraries, CA bundle and selected provider auth,
executes `nexode run --ndjson`, then uploads and runs `tests/test.sh`. It never
replaces the container's system certificate directory. Secrets travel in private
temporary env files or copied auth files, not command arguments or result metadata.
The environment's default user and explicit agent/verifier user overrides apply.
The container and images built by the runner are removed after success or failure;
pre-existing images are retained. Agent/verifier timeouts are bounded and reported
as failed trials, never manufactured zero rewards.

The task contract is `task.toml`, `instruction.md`, and `tests/test.sh`, with either
`environment/Dockerfile` or `environment.docker_image`. CPU, memory and GPU limits,
working directory, environment templates, timeouts and public/disabled networking
are supported. `storage_mb` is retained in the source task configuration; Docker
storage quotas are backend-specific and are not imposed by this runner. Advanced
Harbor features such as Compose, multiple steps, separate verifier environments,
MCP provisioning, custom artifact collection, healthchecks and network allowlists
are rejected explicitly before any trial. This is a native task runner, not a
Python plugin implementing Harbor's full plugin API.

Rewards are parsed from `/logs/verifier/reward.json` first, or `reward.txt` if JSON
is absent. Values must be finite numbers; malformed JSON cannot fall back to an
older text reward. Output contains per-trial `result.json`, verifier logs, agent
NDJSON, final payload, token usage and job `result.json` in the existing stitchable
layout. A zero reward is a completed evaluation, not an infrastructure error.

## Binary and authentication

A compatibility build avoids the glibc mismatch between a developer host and task
images. The default missing binary is built with
`scripts/build_linux_compat.sh /tmp/nexode-compat-dist`. Explicit missing binary
paths fail without silently selecting a different binary.

- `NEXODE_HARBOR_BINARY`, or `--binary`: host binary to stage.
- `NEXODE_HARBOR_BINARY_DIR`: compatibility build directory.
- `NEXODE_HARBOR_OPENAI_AUTH`: OpenAI OAuth file, default `~/.nexode/openai-auth.json`.
- `NEXODE_HARBOR_CLAUDE_AUTH`: Claude OAuth file, default `~/.nexode/auth.json`.
- `ANTHROPIC_API_KEY` / `NEXODE_HARBOR_ANTHROPIC_KEY`, or
  `NEXODE_HARBOR_ANTHROPIC_ENV`: native Anthropic API credentials.
- `OPENROUTER_API_KEY` / `NEXODE_HARBOR_OPENROUTER_KEY`, or
  `NEXODE_HARBOR_OPENROUTER_ENV`: OpenRouter credentials.
- `NEXODE_HARBOR_CA_BUNDLE`, `NEXODE_HARBOR_LIBSSL`, `NEXODE_HARBOR_LIBCRYPTO`:
  explicit host runtime files.
- `NEXODE_TB_MODEL` / `--model`: provider/model, default `openai/gpt-5.4`, or
  `anthropic-api/claude-opus-4-8` with the Claude entrypoint.
- `NEXODE_OPENAI_REASONING_EFFORT` and `NEXODE_ANTHROPIC_REASONING_EFFORT`: default high.
- `NEXODE_OPENAI_SERVICE_TIER`: default priority.
- `NEXODE_TASK_WORKDIR`: agent working directory override.

The optional extra preamble and opt-in legacy preamble remain available through
`NEXODE_HARBOR_EXTRA_PREAMBLE` and `NEXODE_HARBOR_LEGACY_PREAMBLE`. Retry hints require
`NEXODE_HARBOR_ENABLE_HINTS` plus `NEXODE_HARBOR_TASK_HINTS_FILE` (JSON keyed by task).
The normal task instruction remains unmodified unless these options are enabled.

## Datasets and campaigns

`--path` selects an installed task or directory of tasks. `--dataset NAME@VERSION`
resolves the legacy Harbor JSON registry directly, checks out each exact Git
commit and caches the tasks with their registry record. HTTPS repository URLs and
full commit IDs are required. Installed `terminal-bench@2.0` and `@2.1` directories
under `NEXODE_TB_DATASETS_DIR` (default `/tmp`) take precedence. Default local paths
remain `/tmp/terminal-bench-2` and `/tmp/terminal-bench-2.1`; `NEXODE_TB_PATH` overrides
them. Registry retrieval uses `NEXODE_TB_REGISTRY_URL` when explicitly supplied.

```bash
scripts/orbit_dev.sh terminal-bench-campaign \
  --campaign-dir ~/tb2-nexode-campaign --path /tmp/terminal-bench-2 \
  --task regex-log --task largest-eigenval -k 2
```

Campaigns deduplicate task lists, pin model/reasoning/service tier/binary SHA/auth
path/dataset/attempts/timeouts, reject drift, and preserve records in `campaign.json`
and `results.jsonl`. Completed recorded jobs and unrecorded existing jobs are
adopted only with the required number of finite numeric trial rewards for the
correct task. `--max-parallel-tasks` allows independent jobs to run concurrently;
each task job remains sequential. `--continue-on-failure` records incomplete tasks
and continues, matching the former campaign behavior.

## Validation

The Rust tests cover strict rewards, final NDJSON payload selection, campaign drift
and incomplete results. `terminal_bench_native` additionally validates an actual
Docker agent/verifier lifecycle with two concurrent attempts, synthetic credentials,
literal shell metacharacters, usage totals and campaign adoption/resume. Run it with
`ORBIT_TEST_DOCKER_IMAGE` set to a locally available Linux image containing sh/bash.
These tests make no paid model requests. Historical Python Harbor pilot scores are
not evidence of a new native runner leaderboard score.

Task configuration and reward precedence follow the official
[Harbor task schema](https://github.com/harbor-framework/harbor/blob/main/src/harbor/models/task/config.py)
and [verifier contract](https://github.com/harbor-framework/harbor/blob/main/src/harbor/verifier/verifier.py).
