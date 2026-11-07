#!/usr/bin/env bash
# Actual CLI/daemon group transfer on two isolated homes and repository disks.
set -euo pipefail
if [[ ${1:-} != --isolated ]]; then
  ORBIT_DEV_BIN=$("$(dirname "$0")/orbit_dev.sh" --build-only)
  BINARY=$(realpath -e "${NEXODE_BIN:?set NEXODE_BIN to the binary under test}")
  SCRIPT=$(realpath -e "$0")
  W=$(mktemp -d "${NEXODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/cloud-group-test.XXXXXX")
  mkdir -p "$W/home" "$W/runtime" "$W/tmp"
  exec env -i PATH=/usr/local/bin:/usr/bin:/bin LANG=C.UTF-8 HOME="$W/home" \
    TMPDIR="$W/tmp" XDG_RUNTIME_DIR="$W/runtime" NEXODE_RUNTIME_DIR="$W/runtime" \
    NEXODE_HOME="$W/home/.nexode" NEXODE_NO_AUTO_UPDATE=1 NEXODE_NO_TELEMETRY=1 \
    ORBIT_DEV_BIN="$ORBIT_DEV_BIN" NEXODE_NON_INTERACTIVE=1 W="$W" NEXODE_BIN="$BINARY" bash "$SCRIPT" --isolated
fi
RUNTIME=$(mktemp -d /tmp/jcg.XXXXXX)
mkdir "$RUNTIME/local" "$RUNTIME/cloud"
export NEXODE_RUNTIME_DIR="$RUNTIME/local" XDG_RUNTIME_DIR="$RUNTIME/local"
PIDS=()
cleanup() {
  local status=$?
  trap - EXIT
  for pid in "${PIDS[@]}"; do kill -TERM -- "-$pid" 2>/dev/null || true; done
  # Bound teardown even if a fixture daemon cannot complete graceful shutdown.
  for pid in "${PIDS[@]}"; do kill -KILL -- "-$pid" 2>/dev/null || true; done
  for pid in "${PIDS[@]}"; do wait "$pid" 2>/dev/null || true; done
  rm -rf -- "$RUNTIME"
  echo "Acceptance artifacts: $W"
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for prerequisite in bwrap git setsid rg timeout; do command -v "$prerequisite" >/dev/null; done
REPO="$W/local/project"
mkdir -p "$REPO" "$W/cloud/projects" "$W/cloud/home" "$NEXODE_HOME/sessions" "$NEXODE_HOME/todos" "$NEXODE_RUNTIME_DIR/durable-state/swarm"
git -C "$REPO" init -q
git -C "$REPO" config user.name 'Group migration fixture'
git -C "$REPO" config user.email 'fixture@example.invalid'
printf 'base\n' > "$REPO/file.txt"
git -C "$REPO" add .
git -C "$REPO" commit -qm base
printf 'local uncommitted\n' > "$REPO/file.txt"
printf 'untracked\n' > "$REPO/untracked.txt"
mkdir -p "$REPO/.nexode"
printf '{"requirements":[{"kind":"env_var","name":"GROUP_RESOURCE_TOKEN"}]}\n' > "$REPO/.nexode/cloud-environment.json"
export GROUP_RESOURCE_TOKEN=synthetic-group-resource
"$ORBIT_DEV_BIN" cloud-fixture seed-group "$NEXODE_HOME" "$REPO" "$NEXODE_RUNTIME_DIR"
{
  printf '#!/usr/bin/env bash\nset -euo pipefail\n'
  printf 'W=%q\nBINARY=%q\nRUNTIME=%q\n' "$W" "$NEXODE_BIN" "$RUNTIME"
  cat <<'TRANSPORT'
run_remote() {
  exec bwrap --die-with-parent --ro-bind / / --tmpfs /tmp --proc /proc --dev /dev \
    --bind "$W/cloud/projects" "$W/local" --bind "$W/cloud/home" "$W/home" \
    --bind "$RUNTIME/cloud" "$RUNTIME/cloud" \
    --ro-bind "$BINARY" /tmp/nexode-group-test --clearenv \
    --setenv PATH /usr/local/bin:/usr/bin:/bin --setenv HOME "$W/home" \
    --setenv NEXODE_HOME "$W/home/.nexode" --setenv NEXODE_NO_AUTO_UPDATE 1 \
    --setenv NEXODE_NO_TELEMETRY 1 --setenv NEXODE_NON_INTERACTIVE 1 \
    --setenv NEXODE_RUNTIME_DIR "$RUNTIME/cloud" --setenv XDG_RUNTIME_DIR "$RUNTIME/cloud" \
    --setenv NEXODE_SOCKET "$RUNTIME/cloud/cloud.sock" \
    --chdir / bash -c "$1"
}
if [[ "$1" == *env*receive* ]]; then printf 'receive\n' >> "$W/environment-transfers"; fi
if [[ -e "$W/lose-activation-response" && "$1" == *activate* ]]; then
  (run_remote "$1") > "$W/activation-output" 2>> "$W/activation-error"
  exit 255
fi
run_remote "$1"
TRANSPORT
} > "$W/transport"
chmod +x "$W/transport"
export NEXODE_SOCKET="$RUNTIME/local/local.sock"
setsid "$NEXODE_BIN" --no-update --no-selfdev serve > "$W/local-daemon.log" 2>&1 &
PIDS+=("$!")
setsid "$W/transport" '/tmp/nexode-group-test --no-update --no-selfdev serve' > "$W/cloud-daemon.log" 2>&1 &
PIDS+=("$!")
"$ORBIT_DEV_BIN" cloud-fixture wait-paths "$RUNTIME/local/local-debug.sock" "$RUNTIME/cloud/cloud-debug.sock"
if "$NEXODE_BIN" --no-update --no-selfdev debug sessions --socket "$NEXODE_SOCKET" > "$W/debug-output" 2> "$W/debug-error"; then
  echo 'FAIL: group migration must not require unrestricted debug control' >&2
  exit 1
fi
cd "$REPO"
move_group() {
  timeout --kill-after=5s 90s "$NEXODE_BIN" --no-update --no-selfdev cloud move --session group-root \
    --host fixture-cloud --remote-binary /tmp/nexode-group-test --transport "$W/transport" \
    --include-session independent --json "$@"
}
move_group --dry-run > "$W/preview.json"
[[ ! -e "$NEXODE_HOME/session_leases/group-root.json" ]]
[[ ! -e "$NEXODE_HOME/session_leases/group-worker.json" ]]
[[ ! -e "$NEXODE_HOME/session_leases/independent.json" ]]
# A successful remote activation with a lost response must never revive local agents.
touch "$W/lose-activation-response"
if move_group > "$W/uncertain.json" 2> "$W/uncertain.err"; then
  echo 'FAIL: simulated lost activation response reported success' >&2
  exit 1
fi
"$ORBIT_DEV_BIN" cloud-fixture assert-group-away "$NEXODE_HOME"
rm "$W/lose-activation-response"
move_group > "$W/recovered.json"
"$ORBIT_DEV_BIN" cloud-fixture assert-group-recovered "$NEXODE_HOME" "$W/cloud/home/.nexode" "$RUNTIME/cloud/durable-state/swarm"
cmp "$REPO/file.txt" "$W/cloud/projects/project/file.txt"
cmp "$REPO/untracked.txt" "$W/cloud/projects/project/untracked.txt"
"$ORBIT_DEV_BIN" cloud-fixture assert-group-environment "$W/cloud/home/.nexode/cloud-environment" "$W/environment-transfers"
if rg -q 'synthetic-group-resource' "$W/preview.json" "$W/recovered.json" "$W/uncertain.err"; then
  echo 'FAIL: opaque environment value appeared in command output' >&2
  exit 1
fi
# Single-member return must not silently split a migrated group.
if "$NEXODE_BIN" --no-update --no-selfdev cloud return --session group-worker \
    --host fixture-cloud --remote-binary /tmp/nexode-group-test --transport "$W/transport" > "$W/return.json" 2> "$W/return.err"; then
  echo 'FAIL: partial group return was allowed' >&2
  exit 1
fi
printf 'PASS: real-daemon swarm selection, explicit independent inclusion, preview, shared repository, preserved metadata, uncertain activation recovery and partial-return rejection\n'
