#!/usr/bin/env bash
# Validation only. The standalone helper, not this script, performs signing.
set -euo pipefail
[[ "${GITHUB_REPOSITORY:-}" == nexode-agent/unconfigured && "${GITHUB_REF:-}" == refs/heads/master &&
   "${GITHUB_EVENT_NAME:-}" == workflow_dispatch ]] || { echo 'Requires canonical master dispatch' >&2; exit 1; }
[[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo 'Requires native Apple ARM64' >&2; exit 1; }
pin=$GITHUB_SHA
[[ "$(git rev-parse HEAD)" == "$GITHUB_SHA" ]]
git diff HEAD --exit-code -- tools/orbit-dev/src/handoff
fixture=signing-helper-validation
: "${ORBIT_DEV_BIN:?Build native tooling before exposing signing credentials}"
hash() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
case "${1:-}" in
prepare)
    mkdir "$fixture"
    cat > "$fixture/fixture.c" <<'C'
#include <stdio.h>
int main(void) { puts("nexode-signing-helper-native-ok"); return 0; }
C
    /usr/bin/clang -arch arm64 -O2 "$fixture/fixture.c" -o "$fixture/unsigned"
    [[ "$("./$fixture/unsigned")" == nexode-signing-helper-native-ok ]]
    echo "sha256=$(hash "$fixture/unsigned")" >> "$GITHUB_OUTPUT"
    exit 0
    ;;
sign) ;;
*) echo 'Usage: check_signing_helper_macos.sh prepare|sign' >&2; exit 1 ;;
esac
[[ "${TRUSTED_SHA256:-}" =~ ^[a-f0-9]{64}$ && "$(hash "$fixture/unsigned")" == "$TRUSTED_SHA256" ]]
for name in APPLE_CERTIFICATE_P12_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY; do
    [[ -n "${!name:-}" ]] || { echo "Missing credential: $name" >&2; exit 1; }
done
team_id=$("$ORBIT_DEV_BIN" signing-check team)
umask 077
work=$(mktemp -d)
keychain="$work/helper.keychain-db"
password=$(openssl rand -hex 32)
cleanup() {
    local status=$?
    if [[ -f "$work/keychains.txt" ]]; then
        "$ORBIT_DEV_BIN" signing-check keychains "$work/keychains.txt" || status=1
    fi
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
security list-keychains -d user > "$work/keychains.pending"
mv "$work/keychains.pending" "$work/keychains.txt"
printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$work/certificate.p12"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
"$ORBIT_DEV_BIN" signing-check keychains "$work/keychains.txt" "$keychain"
security find-identity -v -p codesigning "$keychain" > "$work/identities.txt"
certificate_sha1=$("$ORBIT_DEV_BIN" signing-check fingerprint "$work/identities.txt")
# Never export credentials to the helper process. Its native tools use a minimal env too.
unset APPLE_CERTIFICATE_P12_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY
# A wrong trusted digest must refuse before invoking a signer or creating output.
if "$ORBIT_DEV_BIN" sign-release --input "$fixture/unsigned" --output "$fixture/wrong-hash" \
    --sha256 0000000000000000000000000000000000000000000000000000000000000000 \
    --target aarch64-apple-darwin --identity "$certificate_sha1" --execute \
    > "$work/hash-refusal.txt" 2>&1; then
    echo 'Helper accepted a mismatched input hash' >&2; exit 1
fi
[[ ! -e "$fixture/wrong-hash" && ! -L "$fixture/wrong-hash" ]]
[[ "$(hash "$fixture/unsigned")" == "$TRUSTED_SHA256" ]]
args=(--input "$fixture/unsigned" --output "$fixture/signed" --sha256 "$TRUSTED_SHA256"
      --target aarch64-apple-darwin --identity "$certificate_sha1")
"$ORBIT_DEV_BIN" sign-release "${args[@]}" > "$work/plan.json"
[[ ! -e "$fixture/signed" && ! -L "$fixture/signed" ]]
"$ORBIT_DEV_BIN" signing-check plan "$work/plan.json"
"$ORBIT_DEV_BIN" sign-release "${args[@]}" --execute > "$work/execution.json"
requirement='anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and identifier "com.nexode.cli"'
requirement+=" and certificate leaf[subject.OU] = \"$team_id\""
/usr/bin/codesign --verify --strict --verbose=2 -R "=$requirement" "$fixture/signed"
[[ "$("./$fixture/signed")" == nexode-signing-helper-native-ok ]]
signed_hash=$(hash "$fixture/signed")
if "$ORBIT_DEV_BIN" sign-release "${args[@]}" --execute > "$work/refusal.txt" 2>&1; then
    echo 'Helper accepted an existing destination' >&2; exit 1
fi
[[ "$(hash "$fixture/signed")" == "$signed_hash" && "$(hash "$fixture/unsigned")" == "$TRUSTED_SHA256" ]]
"$ORBIT_DEV_BIN" signing-check receipt "$fixture" "$pin" "$signed_hash" "$team_id" "$work/execution.json"
