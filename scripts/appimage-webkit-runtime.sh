#!/usr/bin/env bash
# WebKitGTK release builds embed absolute subprocess paths. Keep those paths
# absolute inside bubblewrap without installing files into the host filesystem.
set -euo pipefail
[[ "${APPDIR:-}" == /* && -x "$APPDIR/usr/bin/nexode-desktop" ]] || exit 1
IFS= read -r helper_dir < "$APPDIR/usr/share/nexode-webkit-helper-path"
[[ "$helper_dir" == /usr/lib/*/webkit2gtk-4.1 || "$helper_dir" == /usr/lib/webkit2gtk-4.1 ]] || exit 1
umask 077
runtime="$(mktemp -d /tmp/XXXXXX)"
child=''
cleanup() {
  if [[ -n "$child" ]] && kill -0 "$child" 2>/dev/null; then
    kill -TERM "$child" 2>/dev/null || true
    wait "$child" 2>/dev/null || true
  fi
  # Only the private directory returned by mktemp may be removed. Never follow
  # a replacement symlink or accept a path outside this short runtime prefix.
  if [[ "$runtime" =~ ^/tmp/[A-Za-z0-9]{6}$ && -d "$runtime" && ! -L "$runtime" && -O "$runtime" ]]; then
    rm -rf -- "$runtime"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM HUP
cp --reflink=auto "$APPDIR/usr/lib/libwebkit2gtk-4.1.so.0" "$runtime/libwebkit2gtk-4.1.so.0"
cp -a "$APPDIR$helper_dir" "$runtime/w"
cp "$APPDIR/usr/bin/bwrap" "$runtime/b"
cp "$APPDIR/usr/bin/xdg-dbus-proxy" "$runtime/d"
"$APPDIR/usr/bin/mcp/node" - "$runtime" "$helper_dir" <<'NODE'
const fs = require('node:fs');
const [runtime, helper] = process.argv.slice(2);
const file = runtime + '/libwebkit2gtk-4.1.so.0';
const bytes = fs.readFileSync(file);
for (const [original, relocated] of [
  [helper + '/injected-bundle/', runtime + '/w/injected-bundle/'],
  [helper, runtime + '/w'],
  ['/usr/bin/xdg-dbus-proxy', runtime + '/d'],
  ['/usr/bin/bwrap', runtime + '/b'],
]) {
  const before = Buffer.from(original), after = Buffer.from(relocated);
  if (after.length > before.length) throw Error('WebKit runtime path exceeds reserved space');
  let at = 0, count = 0;
  while ((at = bytes.indexOf(before, at)) >= 0) {
    bytes.fill(0, at, at + before.length);
    after.copy(bytes, at);
    at += before.length;
    count++;
  }
  if (!count) throw Error('WebKit runtime path is incompatible with this bundle');
}
fs.writeFileSync(file, bytes);
NODE
export LD_LIBRARY_PATH="$runtime:$APPDIR/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export WEBKIT_FORCE_SANDBOX=1
"$APPDIR/usr/bin/nexode-desktop" "$@" &
child=$!
wait "$child"
child=''
