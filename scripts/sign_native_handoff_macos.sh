#!/usr/bin/env bash
# Only called after same-run provenance and native smoke verification.
set -euo pipefail
for name in APPLE_CERTIFICATE_P12_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_SIGNING_IDENTITY \
            APPLE_NOTARY_KEY_P8_BASE64 APPLE_NOTARY_KEY_ID APPLE_NOTARY_ISSUER; do
    [[ -n "${!name:-}" ]] || { echo "Required signing credential missing: $name" >&2; exit 1; }
done
: "${ORBIT_DEV_BIN:?Build native tooling before exposing signing credentials}"
[[ -x "$ORBIT_DEV_BIN" ]] || { echo "Native maintenance binary missing" >&2; exit 1; }
# A full certificate name avoids substring selection and rules out ad-hoc signing.
team_id=$("$ORBIT_DEV_BIN" signing-check team)
[[ "${GITHUB_REPOSITORY:-}" == nexode-agent/unconfigured && "${GITHUB_REF:-}" == refs/heads/master ]] || {
    echo 'Signing requires canonical master' >&2; exit 1;
}
[[ "$(uname -s)" == Darwin ]] || { echo 'Signing requires macOS' >&2; exit 1; }
umask 077
work=$(mktemp -d)
keychain="$work/handoff.keychain-db"
password=$(openssl rand -hex 32)
cleanup() {
    local status=$?
    if [[ -f "$work/keychains.txt" ]]; then
        # Restore the original search list before removing our temporary identity.
        "$ORBIT_DEV_BIN" signing-check keychains "$work/keychains.txt" || status=1
    fi
    security delete-keychain "$keychain" >/dev/null 2>&1 || true
    rm -rf -- "$work"
    exit "$status"
}
trap cleanup EXIT
# Snapshot before create-keychain, which can itself change the search list.
security list-keychains -d user > "$work/keychains.pending"
mv "$work/keychains.pending" "$work/keychains.txt"
printf '%s' "$APPLE_CERTIFICATE_P12_BASE64" | base64 --decode > "$work/certificate.p12"
printf '%s' "$APPLE_NOTARY_KEY_P8_BASE64" | base64 --decode > "$work/AuthKey.p8"
security create-keychain -p "$password" "$keychain"
security set-keychain-settings -lut 21600 "$keychain"
security unlock-keychain -p "$password" "$keychain"
security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -s -k "$password" "$keychain" >/dev/null
# An explicit codesign --keychain alone is not sufficient for identity discovery
# on a fresh runner. Preserve existing trust-chain keychains in the search list.
"$ORBIT_DEV_BIN" signing-check keychains "$work/keychains.txt" "$keychain"
echo 'Signing keychain imported and added to user search list'
# Resolve an exact full name in this isolated keychain, then sign by SHA1 rather
# than letting codesign choose a partial-name match from another certificate.
security find-identity -v -p codesigning "$keychain" > "$work/identities.txt"
certificate_sha1=$("$ORBIT_DEV_BIN" signing-check fingerprint "$work/identities.txt")
mkdir signed-handoff
cp handoff/nexode signed-handoff/nexode
chmod 755 signed-handoff/nexode
codesign --force --sign "$certificate_sha1" --keychain "$keychain" \
    --identifier com.nexode.cli --options runtime --timestamp signed-handoff/nexode
echo 'Developer ID signature created'
# Match justrust's Apple Developer ID chain and stable-identifier requirement,
# additionally binding the leaf Team ID to the exact identity selected above.
requirement='anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and identifier "com.nexode.cli"'
requirement+=" and certificate leaf[subject.OU] = \"$team_id\""
# The leading '=' selects inline requirement text, otherwise codesign opens a file.
codesign --verify --strict --verbose=2 -R "=$requirement" signed-handoff/nexode
# Exercise the signed image with the same credential-free, bounded native smoke
# checks before sending anything to Apple's notary service.
"$ORBIT_DEV_BIN" signing-check smoke signed-handoff
# CLI executables cannot carry stapled tickets. Preserve the accepted submission
# result with the signed ZIP instead of claiming stapler or offline Gatekeeper acceptance.
ditto -c -k --keepParent signed-handoff/nexode "$work/nexode.zip"
xcrun notarytool submit "$work/nexode.zip" --key "$work/AuthKey.p8" \
    --key-id "$APPLE_NOTARY_KEY_ID" --issuer "$APPLE_NOTARY_ISSUER" \
    --wait --timeout 20m --output-format json > signed-handoff/notarization.json
"$ORBIT_DEV_BIN" signing-check notarization signed-handoff/notarization.json
cp "$work/nexode.zip" signed-handoff/nexode.zip
# The original provenance describes the unsigned binary only. Record the
# transformation without silently replacing the original build digest.
"$ORBIT_DEV_BIN" signing-check provenance handoff signed-handoff
