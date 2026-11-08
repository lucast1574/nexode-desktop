# Repository scope

This public repository contains Nexode Desktop at the root and the complete
Orbit engine, CLI/TUI and SDK in `nexode-orbit/`. Keep them in this repository;
do not introduce sibling checkouts, Git submodules or cross-repository CI checkouts.
The imported Orbit Git history is preserved. Its old public repository was deleted
after verifying the complete import; this is the single development repository.

Use `./start.ps1 -BuildOnly` on Windows or `bash scripts/build.sh` on Linux/macOS
to compile both binaries together. The SDK dependency is local to this checkout.
Keep the two Rust manifests and lockfiles: Orbit has its own internal workspace.

Validate Desktop with `cargo fmt --check`, `cargo test --locked` and
`cargo clippy --locked --all-targets -- -D warnings`. Engine changes also follow
`nexode-orbit/AGENTS.md`. Preserve configured Git identity and unrelated work.
Work on main and push authorized changes directly without a PR by default.
