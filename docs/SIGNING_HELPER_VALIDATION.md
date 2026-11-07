# Native signing-helper validation

The dispatch-only workflow validates Nexode Orbit's Rust signing helper,
`cargo xtask sign-release`. Its contract was ported from justrust revision
`17c344f2e83f836598b82715950497c0879aa713`; the source and license attribution
remain in NOTICE. The workflow no longer downloads or executes the Python helper.

Before credentials, the native macOS ARM64 job tests the Rust helper, builds the
maintenance binary from the exact workflow source, and compiles a fixed C fixture.
The signing step checks its trusted SHA256, refuses wrong hashes and existing
outputs, plans without signing, then invokes absolute `/usr/bin/codesign` with
an installed certificate fingerprint. Verification requires the Apple Developer
ID chain, `com.nexode.cli`, and the expected Team ID. Signing never edits the
unsigned source. The temporary keychain search list is restored on exit.

The helper also supports Windows PE headers and installed certificate thumbprints
through an explicit absolute SDK signtool path. Execution requires the target OS;
the default is a read-only plan. Tests use synthetic files and mock signers.

The protected `native-handoff-signing` environment and canonical master guard
remain. This migration does not enable workflows, install credentials, sign real
binaries, publish releases or submit to Apple. Only the public receipt is uploaded
by an authorized workflow run; it identifies the current Orbit source revision.

Local checks:

```sh
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml handoff::
cargo test --locked --manifest-path tools/orbit-dev/Cargo.toml --test native_handoff
bash -n scripts/check_signing_helper_macos.sh scripts/sign_native_handoff_macos.sh
```

The Rust port still needs native Apple and Windows acceptance. The following
records describe the historical upstream Python implementation only.

## Recorded native acceptance

On October 28, 2026, protected run `40632462506`, attempt 1, passed at Nexode
source `001a400fa2969711d48932dc3c40707a749bc54b`, with exact helper revision
`17c344f2e83f836598b82715950497c0879aa713`. Required-reviewer/master-only
protection remained enabled, and normal reviewer approval used the existing user
authorization to validate the dedicated signing identity.

The real Python helper invoked native `/usr/bin/codesign` and its verifier on
the fixed C fixture. The additional Apple chain/identifier/Team ID requirement
passed and the signed ARM64 fixture executed successfully. Wrong-hash execution
created no output, the read-only plan created no output, the original fixture's
hash stayed unchanged, and a repeat execution into the existing destination was
refused without modifying it. All 17 pinned helper tests and 7 credential-free
validation fixtures also passed in the real macOS job.

Public receipt artifact `12791579782` (`signing-helper-receipt-1`) has archive
SHA256 `2d0b731719b256c408aa6d41554dfa1602260e1f4b9f25105aaaa38709507958`.
Its downloaded JSON is preserved exactly in
[`evidence/signing-helper-validation-40632462506.json`](evidence/signing-helper-validation-40632462506.json).
Only the receipt was uploaded, not the fixture executable or credential material.
This proves native execution of the standalone helper, not an additional Nexode
release build, notarization, Windows signing, or end-to-end native Rust CLI binary
execution. The installed Rust CLI's plan/host-guard checks and the separately
accepted Nexode notarization remain distinct evidence.
