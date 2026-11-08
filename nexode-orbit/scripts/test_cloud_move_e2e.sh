#!/usr/bin/env bash
# End-to-end test for `nexode cloud move` / `nexode cloud return`.
#
# Two fully isolated "machines" on one box, neither touching the user's
# ~/.nexode or running sessions:
#   local: NEXODE_HOME=$W/local-home, repo at $REPO
#   cloud: a bubblewrap sandbox with its own HOME/NEXODE_HOME and its own
#          private $REPO path (tmpfs overlay), reached through a transport
#          script standing in for `ssh <host>`.
set -euo pipefail

# Re-exec with an allowlisted environment before starting any client or daemon.
# Even a caller-supplied W is only a parent: never reuse or delete an old run.
if [[ ${1:-} != --isolated ]]; then
  ORBIT_DEV_BIN=$("$(dirname "$0")/orbit_dev.sh" --build-only)
  NEXODE_BIN=$(realpath -e "${NEXODE_BIN:?set NEXODE_BIN to the nexode binary under test}")
  SCRIPT=$(realpath -e "$0")
  W=$(mktemp -d "${W:-${NEXODE_SCRATCH_DIR:-/tmp}}/cloud-test.XXXXXX")
  mkdir -m 700 "$W/home" "$W/runtime" "$W/tmp"
  EXTRA=()
  for name in CLOUD_NEXODE_BIN RETURN_NEXODE_BIN CLOUD_UPGRADE_TO; do
    if [[ -n ${!name:-} ]]; then EXTRA+=("$name=$(realpath -e "${!name}")"); fi
  done
  exec env -i PATH=/usr/local/bin:/usr/bin:/bin LANG=C.UTF-8 \
    HOME="$W/home" TMPDIR="$W/tmp" XDG_RUNTIME_DIR="$W/runtime" NEXODE_RUNTIME_DIR="$W/runtime" \
    NEXODE_TEST_WORKSPACE="$W" W="$W" NEXODE_BIN="$NEXODE_BIN" ORBIT_DEV_BIN="$ORBIT_DEV_BIN" \
    "${EXTRA[@]}" bash "$SCRIPT" --isolated
fi
REPO=$W/repo
printf -v REPO_Q %q "$REPO"
# Stop only processes carrying this run's unguessable workspace marker. This
# includes detached daemons and provider children, not the user's real server.
cleanup() {
  local status=$?
  trap - EXIT INT TERM
  env -u NEXODE_TEST_WORKSPACE timeout --kill-after=2s 12s "$ORBIT_DEV_BIN" cloud-fixture cleanup-workspace "$W" "$$" || status=1
  if (( status != 0 )); then echo "FAILED: artifacts retained at $W" >&2; fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
CLOUD_ROOT=$W/cloud-fs
mkdir -p "$W/local-home" "$CLOUD_ROOT/home" "$CLOUD_ROOT/repo"

pass() { printf '  \033[32mPASS\033[0m %s\n' "$*"; }
fail() { printf '  \033[31mFAIL\033[0m %s\n' "$*"; exit 1; }
check() { local what=$1; shift; if "$@"; then pass "$what"; else fail "$what"; fi; }
lease_is_here() { [[ -f $1 ]] && ! grep -q away_host "$1"; }

# ---- fake cloud transport ------------------------------------------------
{
  printf '#!/usr/bin/env bash\n'
  printf 'CLOUD_HOME=%q\nCLOUD_REPO=%q\nREPO=%q\nBINARY=%q\nWORKSPACE=%q\n' \
    "$CLOUD_ROOT/home" "$CLOUD_ROOT/repo" "$REPO" "${CLOUD_NEXODE_BIN:-$NEXODE_BIN}" "$W"
  cat <<'EOF'
exec bwrap --die-with-parent \
  --ro-bind /usr /usr --ro-bind /etc /etc --symlink usr/lib /lib --symlink usr/lib64 /lib64 \
  --symlink usr/bin /bin --symlink usr/bin /sbin --proc /proc --dev /dev --tmpfs /tmp \
  --bind "$CLOUD_HOME" /home/cloud --bind "$CLOUD_REPO" "$REPO" \
  --ro-bind "$BINARY" /opt/nexode/nexode-under-test \
  --clearenv --setenv HOME /home/cloud --setenv USER cloud --setenv PATH /opt/nexode:/usr/local/bin:/usr/bin \
  --setenv NEXODE_HOME /home/cloud/.nexode --setenv NEXODE_CLOUD_HOST_LABEL cloud-e2e \
  --setenv NEXODE_RUNTIME_DIR /home/cloud/runtime --setenv XDG_RUNTIME_DIR /home/cloud/runtime \
  --setenv NEXODE_TEST_WORKSPACE "$WORKSPACE" \
  --setenv NEXODE_NO_AUTO_UPDATE 1 --setenv NEXODE_NON_INTERACTIVE 1 --setenv NEXODE_NO_TELEMETRY 1 \
  --unshare-pid --unshare-ipc --unshare-uts --hostname cloud-e2e \
  bash -c "$1"
EOF
} > "$W/cloud-transport"
chmod +x "$W/cloud-transport"

export NEXODE_HOME=$W/local-home
# The product splits transport strings on whitespace, so resolve by PATH.
export PATH="$W:$PATH"
export NEXODE_CLOUD_TRANSPORT=cloud-transport
export NEXODE_CLOUD_HOST_LABEL=laptop-e2e
export NEXODE_NO_AUTO_UPDATE=1
export NEXODE_NO_TELEMETRY=1
unset NEXODE_SESSION_ID NEXODE_SOCKET
J() { timeout --kill-after=5s 90s "$NEXODE_BIN" --no-update --no-selfdev "$@"; }
# Local client used for returns. Set RETURN_NEXODE_BIN to simulate a local
# update between `cloud move` and `cloud return`.
JR() { timeout --kill-after=5s 90s "${RETURN_NEXODE_BIN:-$NEXODE_BIN}" --no-update --no-selfdev "$@"; }
CLOUD() { timeout --kill-after=5s 90s "$W/cloud-transport" "NEXODE_HOME=/home/cloud/.nexode $*"; }
RB=(--host cloud-e2e --remote-binary /opt/nexode/nexode-under-test)

echo "== workspace: $W"

# ---- test repo with committed, staged, unstaged, untracked and ignored state
mkdir -p "$REPO"; cd "$REPO"
git init -q -b main
git config user.name "E2E Tester"; git config user.email e2e@example.invalid
printf 'fn main() {\n    println!("v1");\n}\n' > main.rs
printf 'shared line 1\nshared line 2\nshared line 3\n' > notes.txt
printf 'target/\n.env\n' > .gitignore
git add -A; git commit -qm "initial"
git remote add origin https://example.invalid/test.git
printf 'fn main() {\n    println!("v2 local wip");\n}\n' > main.rs   # unstaged
echo "staged" > staged.txt; git add staged.txt                       # staged
echo "brand new" > untracked.txt                                     # untracked
echo "SECRET_TOKEN=abc" > .env                                       # ignored but allowlisted
mkdir -p target; echo junk > target/big.bin                          # ignored build output
LOCAL_HEAD=$(git rev-parse HEAD)

# ---- a real session JSON in the isolated local home ---------------------
SID=session_e2e_$(date +%s)
mkdir -p "$NEXODE_HOME/sessions" "$NEXODE_HOME/todos"
"$ORBIT_DEV_BIN" cloud-fixture seed-session "$NEXODE_HOME/sessions/$SID.json" "$SID" "$REPO"
echo '[{"id":"1","content":"finish v2","status":"in_progress","priority":"high"}]' > "$NEXODE_HOME/todos/$SID.json"

echo "== 1. dry run changes nothing"
J cloud move --session "$SID" "${RB[@]}" --dry-run --allow-active --json > "$W/dry.json"
check "dry run reports dry_run"  grep -q '"dry_run": true' "$W/dry.json"
check "no local lease after dry run" test ! -e "$NEXODE_HOME/session_leases/$SID.json"
check "local worktree untouched" grep -q "v2 local wip" "$REPO/main.rs"
check "local index untouched (staged.txt still staged)" bash -c "cd $REPO_Q && git diff --cached --name-only | grep -qx staged.txt"

echo "== 2. move to cloud"
J cloud move --session "$SID" "${RB[@]}" --allow-active --json > "$W/move.json"
cat "$W/move.json"
check "local lease says away on cloud-e2e" grep -q '"away_host": "cloud-e2e"' "$NEXODE_HOME/session_leases/$SID.json"
check "cloud has repo at same path" CLOUD "test -f $REPO_Q/main.rs"
check "cloud keeps uncommitted edit" CLOUD "grep -q 'v2 local wip' $REPO_Q/main.rs"
check "cloud keeps untracked file" CLOUD "test -f $REPO_Q/untracked.txt"
check "cloud keeps staged file"  CLOUD "test -f $REPO_Q/staged.txt"
check "cloud got allowlisted .env" CLOUD "grep -q SECRET_TOKEN $REPO_Q/.env"
check "cloud did NOT get target/" CLOUD "test ! -e $REPO_Q/target"
check "cloud HEAD == local HEAD" CLOUD "cd $REPO_Q && test \$(git rev-parse HEAD) = $LOCAL_HEAD"
check "cloud on branch main" CLOUD "cd $REPO_Q && test \$(git symbolic-ref --short HEAD) = main"
check "cloud keeps git identity" CLOUD "cd $REPO_Q && test \"\$(git config user.email)\" = e2e@example.invalid"
check "cloud has transcript" CLOUD "test -f /home/cloud/.nexode/sessions/$SID.json"
check "cloud has todos" CLOUD "grep -q 'finish v2' /home/cloud/.nexode/todos/$SID.json"
check "cloud lease says here" lease_is_here "$CLOUD_ROOT/home/.nexode/session_leases/$SID.json"
CLOUD "cat /home/cloud/.nexode/sessions/$SID.json" > "$W/cloud-session.json"
"$ORBIT_DEV_BIN" cloud-fixture assert-move-notice "$W/cloud-session.json" || fail "agent-facing migration notice"

echo "== 3. local copy can no longer run or overwrite"
refuse_move() { ! J cloud move --session "$SID" "${RB[@]}" --allow-active 2>/dev/null; }
check "second move is refused" refuse_move
J cloud where --session "$SID" | tee "$W/where.txt"
check "where shows cloud-e2e" grep -q cloud-e2e "$W/where.txt"

echo "== 4. work happens on both sides"
# cloud agent: commits the wip + adds a new file, leaves one uncommitted edit
CLOUD "cd $REPO_Q && git add -A && git commit -qm 'cloud: finish v2' && echo 'cloud feature' > cloud.txt && git add cloud.txt && git commit -qm 'cloud: add cloud.txt' && printf 'shared line 1\nshared line 2\nshared line 3 edited in cloud\n' > notes.txt"
# append a cloud-side turn to the transcript (what the cloud agent would do)
"$ORBIT_DEV_BIN" cloud-fixture append-cloud-turn "$CLOUD_ROOT/home/.nexode/sessions/$SID.json"
# meanwhile the user keeps editing locally in a non-conflicting spot
cd "$REPO"
printf 'shared line 1 edited locally\nshared line 2\nshared line 3\n' > notes.txt
echo "local only" > local.txt

echo "== 5. return merges both sides"
if [ -n "${CLOUD_UPGRADE_TO:-}" ]; then
  # Simulate the cloud host's nexode being updated while the session lives there.
  [ -n "${CLOUD_NEXODE_BIN:-}" ] || fail "CLOUD_UPGRADE_TO needs a writable CLOUD_NEXODE_BIN copy"
  cp "$CLOUD_UPGRADE_TO" "$CLOUD_NEXODE_BIN.next" && mv -f "$CLOUD_NEXODE_BIN.next" "$CLOUD_NEXODE_BIN"
  pass "cloud nexode upgraded mid-session to $(CLOUD '/opt/nexode/nexode-under-test --no-update --version')"
fi
JR cloud return --session "$SID" "${RB[@]}" --json > "$W/return.json"
cat "$W/return.json"
check "merge applied" grep -q '"result": "applied"' "$W/return.json"
check "local branch fast-forwarded to cloud commits" bash -c "cd $REPO_Q && git log --format=%s | grep -qx 'cloud: add cloud.txt'"
check "cloud committed file present" test -f "$REPO/cloud.txt"
check "cloud uncommitted edit merged" grep -q "edited in cloud" "$REPO/notes.txt"
check "local edit kept (same file, other hunk)" grep -q "edited locally" "$REPO/notes.txt"
check "local-only file kept" test -f "$REPO/local.txt"
check "ignored build output untouched" test -f "$REPO/target/big.bin"
check "local lease is home again" lease_is_here "$NEXODE_HOME/session_leases/$SID.json"
check "cloud lease no longer owns" grep -q '"away_host": "returned-complete"' "$CLOUD_ROOT/home/.nexode/session_leases/$SID.json"
"$ORBIT_DEV_BIN" cloud-fixture assert-return-notice "$NEXODE_HOME/sessions/$SID.json" || fail "returned transcript"

echo "== 6. round trip again, this time with a real conflict"
J cloud move --session "$SID" "${RB[@]}" --json > /dev/null
check "second move reuses cloud checkout" CLOUD "cd $REPO_Q && grep -q 'edited locally' notes.txt"
CLOUD "cd $REPO_Q && printf 'CLOUD VERSION\n' > main.rs"
cd "$REPO" && printf 'LOCAL VERSION\n' > main.rs
BEFORE=$(sha256sum "$REPO/main.rs")
J cloud return --session "$SID" "${RB[@]}" --json > "$W/return2.json"
cat "$W/return2.json"
check "conflict reported" grep -q '"result": "conflicts"' "$W/return2.json"
check "conflicting file named" grep -q 'main.rs' "$W/return2.json"
check "local file untouched on conflict" test "$(sha256sum "$REPO/main.rs")" = "$BEFORE"
check "cloud work kept as ref" bash -c "cd $REPO_Q && git cat-file -e refs/nexode-cloud/$SID/cloud:main.rs && git show refs/nexode-cloud/$SID/cloud:main.rs | grep -q 'CLOUD VERSION'"

echo
echo "ALL E2E CHECKS PASSED  (workspace $W)"
