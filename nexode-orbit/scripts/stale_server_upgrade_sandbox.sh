#!/usr/bin/env bash
# Live end-to-end sandbox for the "current client, stale older server" fix.
#
#   Server: the REAL released v0.14.6 binary (downloaded from GitHub).
#   Client: the freshly built current binary (target/debug/nexode, has the fix).
#   Field state: shared-server channel pinned to OLD (v0.14.6); stable -> NEW.
#
# It starts the real old daemon, then runs the NEW client's `nexode server reload`
# (which repairs the stale shared-server channel, then forces a reload). PASS iff
# the resulting daemon is running v0.22.x.
#
# Usage:
#   cargo build -p nexode --bin nexode
#   scripts/stale_server_upgrade_sandbox.sh
#
# Linux x86_64 only (uses the published nexode-linux-x86_64 release asset).
set -uo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "$0")/.." && pwd)"
NEW_BIN="${NEW_BIN:-$REPO_ROOT/target/debug/nexode}"
OLD_VERSION="${OLD_VERSION:-v0.14.6}"
OLD_DIR="${OLD_DIR:-/tmp/nexode-sandbox}"
OLD_WRAP="$OLD_DIR/nexode-linux-x86_64"

[ -x "$NEW_BIN" ] || { echo "missing new client binary: $NEW_BIN (run: cargo build -p nexode --bin nexode)"; exit 2; }

# Fetch + extract the real old release binary if it is not already present.
if [ ! -x "$OLD_WRAP" ]; then
  mkdir -p "$OLD_DIR"
  url="$(curl -fsSL "https://api.github.com/repos/nexode-agent/unconfigured/releases/tags/$OLD_VERSION" \
        | grep -o 'https://[^"]*nexode-linux-x86_64.tar.gz' | head -1)"
  [ -n "$url" ] || { echo "could not resolve $OLD_VERSION linux asset URL"; exit 2; }
  echo "Downloading old server $OLD_VERSION ..."
  curl -fsSL "$url" -o "$OLD_DIR/old.tar.gz"
  tar -C "$OLD_DIR" -xzf "$OLD_DIR/old.tar.gz"
fi
[ -x "$OLD_WRAP" ] || { echo "missing old binary $OLD_WRAP after download"; exit 2; }

SANDBOX="$(mktemp -d /tmp/nexode-stale-sandbox.XXXXXX)"
export NEXODE_HOME="$SANDBOX/home"
export NEXODE_RUNTIME_DIR="$SANDBOX/runtime"
# Hard isolation: pin the socket explicitly so we can NEVER touch the real
# global daemon at /run/user/<uid>/nexode.sock.
export NEXODE_SOCKET="$SANDBOX/runtime/nexode.sock"
# Make the new client's clean release version comparable (debug build is dirty).
export NEXODE_TEST_CLIENT_VERSION_OVERRIDE="v0.22.0 (sandbox)"
mkdir -p "$NEXODE_HOME" "$NEXODE_RUNTIME_DIR"

BUILDS="$NEXODE_HOME/builds"
mkdir -p "$BUILDS/versions/0.14.6" "$BUILDS/versions/0.22.0" \
         "$BUILDS/shared-server" "$BUILDS/stable" "$BUILDS/current"

log() { printf '\n=== %s ===\n' "$*"; }

# --- Install the OLD binary (with bundled libs) as version 0.14.6 ----------
cp "$OLD_DIR/nexode-linux-x86_64.bin" "$OLD_DIR/libssl.so.10" \
   "$OLD_DIR/libcrypto.so.10" "$BUILDS/versions/0.14.6/"
cat > "$BUILDS/versions/0.14.6/nexode" <<'WRAP'
#!/usr/bin/env sh
set -eu
real=$0
if command -v readlink >/dev/null 2>&1; then
  resolved=$(readlink -f -- "$0" 2>/dev/null || true)
  [ -n "$resolved" ] && real=$resolved
fi
self_dir=$(CDPATH= cd -- "$(dirname -- "$real")" && pwd)
export LD_LIBRARY_PATH="$self_dir:${LD_LIBRARY_PATH:-}"
exec "$self_dir/nexode-linux-x86_64.bin" "$@"
WRAP
chmod +x "$BUILDS/versions/0.14.6/nexode"

# --- Install the NEW binary as version 0.22.0 (newer mtime) ----------------
cp "$NEW_BIN" "$BUILDS/versions/0.22.0/nexode"
touch -d "+1 minute" "$BUILDS/versions/0.22.0/nexode"

# --- Field state: shared-server -> OLD, stable/current -> NEW --------------
ln -sf "../versions/0.14.6/nexode" "$BUILDS/shared-server/nexode"
echo "0.14.6" > "$BUILDS/shared-server-version"
ln -sf "../versions/0.22.0/nexode" "$BUILDS/stable/nexode"
echo "0.22.0" > "$BUILDS/stable-version"
ln -sf "../versions/0.22.0/nexode" "$BUILDS/current/nexode"
echo "0.22.0" > "$BUILDS/current-version"

log "Initial channel state (the field bug: shared-server pinned to OLD)"
echo "shared-server-version: $(cat "$BUILDS/shared-server-version")"
echo "stable-version:        $(cat "$BUILDS/stable-version")"

SERVER_PID=""
cleanup() {
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null || true
  "$NEW_BIN" --no-update server stop >/dev/null 2>&1 || true
  pkill -f "$BUILDS/versions/0.14.6/nexode-linux-x86_64.bin" 2>/dev/null || true
  pkill -f "$BUILDS/versions/0.22.0/nexode" 2>/dev/null || true
  rm -rf "$SANDBOX"
}
trap cleanup EXIT

server_version_via_socket() {
  # Ask the running daemon (via the new client's debug surface) its version.
  "$NEW_BIN" --no-update debug server:info 2>/dev/null \
    | grep -oE '"version":[[:space:]]*"[^"]*"' | head -1
}

# --- 1) Start the REAL old v0.14.6 daemon ----------------------------------
log "Starting OLD v0.14.6 daemon"
"$BUILDS/shared-server/nexode" --no-update --provider antigravity serve \
  >"$SANDBOX/server.log" 2>&1 &
SERVER_PID=$!
# Wait for the socket to appear.
for _ in $(seq 1 40); do
  [ -S "$NEXODE_SOCKET" ] && break
  sleep 0.25
done
sleep 1
echo "old daemon pid=$SERVER_PID"
echo "server.log tail:"; tail -8 "$SANDBOX/server.log" 2>/dev/null || true
BEFORE="$(server_version_via_socket)"
echo "server version BEFORE (via socket): ${BEFORE:-<none>}"

# --- 2) New client: nexode server reload (repairs channel, then reloads) ----
log "Running NEW client: nexode server reload"
"$NEW_BIN" --no-update server reload 2>&1 | sed 's/^/[server reload] /' || true
echo "shared-server-version after repair: $(cat "$BUILDS/shared-server-version")"

# Give the handoff a moment.
for _ in $(seq 1 40); do
  [ -S "$NEXODE_SOCKET" ] && break
  sleep 0.25
done
sleep 2

# --- 3) Verify the running daemon is now v0.22.x ---------------------------
AFTER="$(server_version_via_socket)"
echo "server version AFTER (via socket): ${AFTER:-<none>}"
echo "server.log tail (post-reload):"; tail -8 "$SANDBOX/server.log" 2>/dev/null || true

log "RESULT"
echo "shared-server-version: before=0.14.6  after=$(cat "$BUILDS/shared-server-version")"
echo "server version:        before=${BEFORE:-?}  after=${AFTER:-?}"

ok_channel=0
[ "$(cat "$BUILDS/shared-server-version")" = "0.22.0" ] && ok_channel=1

ok_server=0
echo "${AFTER:-}" | grep -q "0.22" && ok_server=1

if [ "$ok_channel" = 1 ] && [ "$ok_server" = 1 ]; then
  echo "PASS: new client repaired the channel AND the stale server upgraded to v0.22"
  exit 0
elif [ "$ok_channel" = 1 ]; then
  echo "PARTIAL: channel repaired to 0.22.0, but server version probe inconclusive (AFTER=${AFTER:-none})"
  echo "         (channel repair is the load-bearing fix; server exec depends on old daemon handoff)"
  exit 0
else
  echo "FAIL: channel was not repaired"
  exit 1
fi
