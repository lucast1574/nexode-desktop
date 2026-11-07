#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="$(readlink -f "${BASH_SOURCE[0]}")"
REPO_ROOT="$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)"
NEXODE_BIN="${NEXODE_BIN:-$HOME/.local/bin/nexode}"
MODEL="${NEXODE_OPENRELAY_TEST_MODEL:-gpt-5.6-sol}"
PROVIDER="${NEXODE_OPENRELAY_TEST_PROVIDER:-openai}"
DEFAULT_PROMPT="Configure a hosted cloud-infrastructure provider for read-only Ethereum Classic JSON-RPC that needs no account or API key. Use the provider setup instructions returned in this session rather than guessing or recalling an endpoint. Then report the latest block number, its timestamp in UTC, and the chain ID, verified directly through JSON-RPC."
PROMPT="${NEXODE_OPENRELAY_TEST_PROMPT:-$DEFAULT_PROMPT}"
OUTPUT="${NEXODE_OPENRELAY_TEST_OUTPUT:-$REPO_ROOT/target/openrelay-discovery-test/latest.ndjson}"
SERVER_LOG="${NEXODE_OPENRELAY_TEST_SERVER_LOG:-$REPO_ROOT/target/openrelay-discovery-test/server.jsonl}"

case "${1:-}" in
  --help|-h)
    cat <<EOF
Usage: ${0##*/} [--help|--print-prompt|--dry-run]

Runs a local-only OpenRelay Discovery fixture and an isolated Nexode agent test.
Environment overrides:
  NEXODE_BIN, NEXODE_OPENRELAY_TEST_MODEL, NEXODE_OPENRELAY_TEST_PROVIDER
  NEXODE_OPENRELAY_TEST_PROMPT, NEXODE_OPENRELAY_TEST_OUTPUT
EOF
    exit 0
    ;;
  --print-prompt)
    printf '%s\n' "$PROMPT"
    exit 0
    ;;
  --dry-run)
    printf 'server=%s\nprovider=%s\nmodel=%s\nprompt=%s\noutput=%s\n' \
      "cargo xtask openrelay-discovery-server" \
      "$PROVIDER" "$MODEL" "$PROMPT" "$OUTPUT"
    exit 0
    ;;
  "") ;;
  *)
    printf 'Unknown argument: %s\n' "$1" >&2
    exit 2
    ;;
esac

for command in curl; do
  command -v "$command" >/dev/null || { printf 'Missing required command: %s\n' "$command" >&2; exit 1; }
done
[[ -x "$NEXODE_BIN" ]] || { printf 'Nexode binary is not executable: %s\n' "$NEXODE_BIN" >&2; exit 1; }

mkdir -p "$(dirname "$OUTPUT")" "$(dirname "$SERVER_LOG")"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/nexode-openrelay-discovery.XXXXXX")"
ready_file="$test_root/server-ready.json"
test_home="$test_root/home"
runtime_dir="$test_root/runtime"
work_dir="$test_root/work"
mkdir -p "$test_home" "$runtime_dir" "$work_dir"
chmod 700 "$test_home" "$runtime_dir"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]] && kill -0 "$server_pid" 2>/dev/null; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$test_root"
}
trap cleanup EXIT INT TERM

orbit_dev_bin="$("$REPO_ROOT/scripts/orbit_dev.sh" --build-only)"
"$orbit_dev_bin" openrelay-discovery-server \
  --ready-file "$ready_file" >"$SERVER_LOG" 2>&1 &
server_pid=$!

for _ in $(seq 1 100); do
  [[ -s "$ready_file" ]] && break
  kill -0 "$server_pid" 2>/dev/null || { cat "$SERVER_LOG" >&2; exit 1; }
  sleep 0.05
done
[[ -s "$ready_file" ]] || { printf 'Discovery fixture did not become ready\n' >&2; exit 1; }
endpoint="$("$orbit_dev_bin" openrelay-check endpoint "$ready_file")"

cat >"$test_home/config.toml" <<EOF
[sponsors]
enabled = true
endpoint = "$endpoint"
EOF
chmod 600 "$test_home/config.toml"

# Reuse only local provider authentication needed by this disposable test.
# Nothing is uploaded by the fixture, and the temporary copies are deleted by
# the EXIT trap.
for name in openai-auth.json auth.json auth-refresh-state.json auth-validation.json provider_activity.json; do
  if [[ -f "$HOME/.nexode/$name" ]]; then
    cp -p "$HOME/.nexode/$name" "$test_home/$name"
  fi
done

# Verify the real public endpoint before spending a model call.
rpc_probe="$(curl -fsS --max-time 10 -X POST https://etc.rivet.link/ \
  -H 'content-type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}')"
"$orbit_dev_bin" openrelay-check rpc <<< "$rpc_probe"

printf 'NEXODE_PROGRESS {"percent":10,"message":"Local Discovery fixture ready"}\n'
printf 'NEXODE_PROGRESS {"percent":20,"message":"OpenRelay public RPC verified"}\n'

set +e
NEXODE_HOME="$test_home" \
NEXODE_RUNTIME_DIR="$runtime_dir" \
NEXODE_DISCOVERY_BENCHMARK=1 \
NEXODE_NO_TELEMETRY=1 \
  "$NEXODE_BIN" \
    --no-selfdev \
    --no-update \
    --provider "$PROVIDER" \
    --model "$MODEL" \
    --disable-base-tools \
    --tools bash,discover_tools \
    -C "$work_dir" \
    run --ndjson "$PROMPT" | tee "$OUTPUT"
status=${PIPESTATUS[0]}
set -e

printf 'NEXODE_PROGRESS {"percent":90,"message":"Validating agent trace"}\n'
set +e
"$orbit_dev_bin" openrelay-check trace "$OUTPUT" "$SERVER_LOG"
validation_status=$?
set -e

if [[ $status -ne 0 ]]; then
  printf 'Nexode exited with status %s\n' "$status" >&2
  exit "$status"
fi
if [[ $validation_status -ne 0 ]]; then
  printf 'OpenRelay Discovery trace validation failed\n' >&2
  exit "$validation_status"
fi
printf 'NEXODE_CHECKPOINT {"message":"OpenRelay Discovery test passed"}\n'
