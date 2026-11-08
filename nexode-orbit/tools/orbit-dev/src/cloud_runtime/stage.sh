set -eu
umask 077
root="$HOME/.local/share/nexode/cloud-releases"
mkdir -p "$root"
upload=$(mktemp -d "$root/.upload.XXXXXX")
trap 'rm -rf "$upload"' EXIT
cat > "$upload/runtime.tar.gz"
printf '%s  %s\n' CHECKSUM "$upload/runtime.tar.gz" | sha256sum --check --status
release="$root/runtime-CHECKSUM"
mkdir "$upload/payload"
tar xzf "$upload/runtime.tar.gz" --no-same-owner --no-same-permissions -C "$upload/payload"
probe() {
  env -u NEXODE_RUNTIME_RELEASE_SEMVER -u NEXODE_RUNTIME_RELEASE_GIT_HASH \
    -u NEXODE_RUNTIME_RELEASE_GIT_DATE -u NEXODE_RUNTIME_RELEASE_GIT_TAG \
    "$1" --no-update --no-selfdev --version
}
probe "$upload/payload/nexode"
for command in receive credentials router-credentials cancel-return replace-receive group-protocol; do
  "$upload/payload/nexode" --no-update --no-selfdev cloud "$command" --help >/dev/null
done
# Serialize publication/activation. A stale lock requires manual inspection,
# never delete a lock that may belong to another live deployment.
lock="$root/.runtime-update-lock"
mkdir "$lock" || { echo 'another runtime update holds the lock' >&2; exit 1; }
trap 'rmdir "$lock"; rm -rf "$upload"' EXIT
if [ ! -e "$release" ]; then
  mv "$upload/payload" "$release"
else
  # Never silently reuse a modified release with the same archive identity.
  diff -qr "$upload/payload" "$release" >/dev/null
fi
probe "$release/nexode"
printf 'STAGED_RUNTIME=%s/nexode\n' "$release"
