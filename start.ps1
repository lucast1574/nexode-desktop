param([switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
$cargoBin = Join-Path $env:USERPROFILE '.cargo/bin'
if (Test-Path (Join-Path $cargoBin 'cargo.exe')) { $env:PATH = "$cargoBin;$env:PATH" }
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) { throw 'Instala Rust desde https://rustup.rs' }
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot '.local/target-windows' }
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = '2' }
cargo build --locked --release
if ($LASTEXITCODE) { throw 'La compilación nativa falló' }
cargo build --manifest-path (Join-Path $PSScriptRoot 'nexode-orbit/Cargo.toml') --locked --release --no-default-features --bin nexode
if ($LASTEXITCODE) { throw 'La compilación del motor incluido falló' }
if (-not $BuildOnly) { & (Join-Path $env:CARGO_TARGET_DIR 'release/nexode-desktop.exe') }
