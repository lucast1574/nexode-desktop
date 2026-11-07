
backend=/opt/nexode-cloud/nexode
[ -L "$backend" ] || { echo 'refusing to replace a non-symlink backend' >&2; exit 1; }
previous=$(readlink "$backend")
# Preserve the old target on disk for a coordinated rollback.
printf '%s\n' "$previous" > "$root/previous-backend-$(date +%s)-$$"
next="$backend.runtime-update.$$"
sudo -n ln -s "$release/nexode" "$next"
sudo -n mv -Tf "$next" "$backend"
if ! probe "$backend"; then
  sudo -n ln -s "$previous" "$next"
  sudo -n mv -Tf "$next" "$backend"
  echo 'launcher verification failed, restored previous backend' >&2
  exit 1
fi
printf 'ACTIVATED_LAUNCHER=%s\nPREVIOUS_BACKEND=%s\n' "$release/nexode" "$previous"
echo 'Daemon unchanged. Coordinate a state-preserving protocol reload separately.'
