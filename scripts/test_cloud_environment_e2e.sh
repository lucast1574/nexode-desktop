#!/usr/bin/env bash
# Real CLI acceptance with isolated local/cloud homes and synthetic credentials.
set -euo pipefail
if [[ ${1:-} != --isolated ]]; then
  BINARY=$(realpath -e "${NEXODE_BIN:?set NEXODE_BIN to the binary under test}")
  SCRIPT=$(realpath -e "$0")
  ORBIT_DEV_BIN=${ORBIT_DEV_BIN:-$("$(dirname "$SCRIPT")/orbit_dev.sh" --build-only)}
  ORBIT_DEV_BIN=$(realpath -e "$ORBIT_DEV_BIN")
  W=$(mktemp -d "${NEXODE_SCRATCH_DIR:-${TMPDIR:-/tmp}}/cloud-environment-test.XXXXXX")
  mkdir -p "$W/home" "$W/runtime" "$W/tmp"
  exec env -i PATH=/usr/local/bin:/usr/bin:/bin LANG=C.UTF-8 HOME="$W/home" \
    TMPDIR="$W/tmp" XDG_RUNTIME_DIR="$W/runtime" NEXODE_RUNTIME_DIR="$W/runtime" \
    NEXODE_HOME="$W/home/.nexode" NEXODE_NO_AUTO_UPDATE=1 NEXODE_NO_TELEMETRY=1 \
    NEXODE_NON_INTERACTIVE=1 W="$W" NEXODE_BIN="$BINARY" ORBIT_DEV_BIN="$ORBIT_DEV_BIN" bash "$SCRIPT" --isolated
fi
trap 'echo "Acceptance artifacts: $W"' EXIT
for prerequisite in bwrap rg; do
  command -v "$prerequisite" >/dev/null || { echo "Missing test prerequisite: $prerequisite" >&2; exit 1; }
done
mkdir -p "$W/local/project space/.git" "$W/cloud/projects/project space/.git" \
  "$W/home/.config/gh" "$W/cloud/home" "$W/bin" "$W/results"
REPO="$W/local/project space"
CLOUD_REPO="$W/cloud/projects/project space"
printf 'opaque-environment-sentinel\n' > "$REPO/.env"
printf 'opaque-credential-sentinel\n' > "$W/home/.config/gh/hosts.yml"
export CLOUD_TEST_RESOURCE_TOKEN=opaque-token-sentinel
{
  printf '#!/usr/bin/env bash\nset -euo pipefail\n'
  printf 'W=%q\nBINARY=%q\n' "$W" "$NEXODE_BIN"
  cat <<'TRANSPORT'
# Only the final argument is the remote command. No payload appears in argv.
exec bwrap --die-with-parent --ro-bind / / --tmpfs /tmp --unshare-pid --proc /proc --dev /dev \
  --bind "$W/cloud/projects" "$W/local" --bind "$W/cloud/home" "$W/home" \
  --ro-bind "$BINARY" /tmp/nexode-env-test --clearenv \
  --setenv PATH /usr/local/bin:/usr/bin:/bin --setenv HOME "$W/home" \
  --setenv NEXODE_HOME "$W/home/.nexode" --setenv NEXODE_NO_AUTO_UPDATE 1 \
  --setenv NEXODE_NO_TELEMETRY 1 --setenv NEXODE_NON_INTERACTIVE 1 \
  --setenv NEXODE_RUNTIME_DIR "$W/home/runtime" --setenv XDG_RUNTIME_DIR "$W/home/runtime" \
  --chdir / bash -c "${!#}"
TRANSPORT
} > "$W/bin/ssh"
chmod +x "$W/bin/ssh"
export PATH="$W/bin:$PATH" NEXODE_CLOUD_REMOTE_BINARY=/tmp/nexode-env-test
cd "$REPO"
run_env() { "$NEXODE_BIN" --no-update --no-selfdev cloud env "$@"; }
expect_rejection() {
  if "$@"; then echo "FAIL: rejected request unexpectedly succeeded" >&2; exit 1; fi
}
assert_status() {
  "$ORBIT_DEV_BIN" cloud-fixture assert-environment-status "$1" "$2"
}
push_request() {
  run_env push --host isolated-cloud --request "$2" > "$W/results/$1.json" 2> "$W/results/$1.err"
  assert_status "$W/results/$1.json" true
}
push_request project '{"action":"fetch","resource":{"kind":"project_file","path":".env"}}'
push_request token '{"action":"fetch","resource":{"kind":"env_var","name":"CLOUD_TEST_RESOURCE_TOKEN"}}'
push_request credential '{"action":"fetch","resource":{"kind":"credential","name":"github"}}'
cmp "$REPO/.env" "$CLOUD_REPO/.env"
cmp "$W/home/.config/gh/hosts.yml" "$W/cloud/home/.config/gh/hosts.yml"
[[ $(stat -c '%a' "$CLOUD_REPO/.env") == 600 ]]
[[ $(stat -c '%a' "$W/cloud/home/.config/gh/hosts.yml") == 600 ]]
"$ORBIT_DEV_BIN" cloud-fixture assert-environment-token "$W/cloud/home/.nexode/cloud-environment"
# Different existing data must never be overwritten by a later transfer.
printf 'newer-cloud-credential\n' > "$W/cloud/home/.config/gh/hosts.yml"
printf 'newer-cloud-file\n' > "$CLOUD_REPO/.env"
push_request sync '{"action":"sync"}'
[[ $(cat "$CLOUD_REPO/.env") == newer-cloud-file ]]
[[ $(cat "$W/cloud/home/.config/gh/hosts.yml") == newer-cloud-credential ]]
# A handoff callback requires matching, fresh local and receiving leases.
mkdir -p "$NEXODE_HOME/session_leases" "$W/cloud/home/.nexode/session_leases"
NOW=$(date -u +%Y-%m-%dT%H:%M:%SZ)
printf '{"session_id":"test-session","epoch":1,"away_host":"isolated-cloud","repo_root":"%s","updated_at":"%s"}\n' "$REPO" "$NOW" > "$NEXODE_HOME/session_leases/test-session.json"
printf '{"session_id":"test-session","epoch":1,"repo_root":"%s","updated_at":"%s"}\n' "$REPO" "$NOW" > "$W/cloud/home/.nexode/session_leases/test-session.json"
export NEXODE_CLOUD_ENV_SESSION=test-session NEXODE_SSH_REMOTE=isolated-cloud
printf 'opaque-callback-sentinel\n' > "$REPO/.env.callback"
printf '%s' '{"action":"fetch","resource":{"kind":"project_file","path":".env.callback"}}' | \
  run_env fulfill --session test-session > "$W/results/callback.json" 2> "$W/results/callback.err"
assert_status "$W/results/callback.json" true
cmp "$REPO/.env.callback" "$CLOUD_REPO/.env.callback"
NEXODE_CLOUD_ENV_SESSION=other expect_rejection run_env fulfill --session test-session <<< '{"action":"status"}' > "$W/results/wrong-session.json"
assert_status "$W/results/wrong-session.json" false
NEXODE_SSH_REMOTE=other expect_rejection run_env fulfill --session test-session <<< '{"action":"status"}' > "$W/results/wrong-host.json"
assert_status "$W/results/wrong-host.json" false
printf '{"session_id":"test-session","epoch":1,"away_host":"isolated-cloud","repo_root":"%s","updated_at":"2000-01-01T00:00:00Z"}\n' "$REPO" > "$NEXODE_HOME/session_leases/test-session.json"
expect_rejection run_env fulfill --session test-session <<< '{"action":"status"}' > "$W/results/expired.json"
assert_status "$W/results/expired.json" false
# Restricted API rejects arbitrary files, executable environment controls and SSH keys.
for REQUEST in \
  '{"action":"fetch","resource":{"kind":"project_file","path":"../private"}}' \
  '{"action":"fetch","resource":{"kind":"env_var","name":"BASH_ENV"}}' \
  '{"action":"fetch","resource":{"kind":"credential","name":"ssh"}}'; do
  expect_rejection run_env push --host isolated-cloud --request "$REQUEST" > "$W/results/rejected.json"
  assert_status "$W/results/rejected.json" false
done
# Statuses and metadata must never contain synthetic secret bytes.
if rg -l 'opaque-(environment|credential|token|callback)-sentinel' \
  "$W/results" "$REPO/.nexode/cloud-environment.json" "$CLOUD_REPO/.nexode/cloud-environment.json"; then
  echo 'FAIL: a secret appeared in status or metadata' >&2
  exit 1
fi
printf 'PASS: opaque project/credential/environment transfers, replay, no-overwrite, lease binding, expiry, and secret-free statuses\n'
