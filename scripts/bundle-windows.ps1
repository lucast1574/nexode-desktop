param(
    [string]$DesktopBinary,
    [string]$EngineBinary,
    [string]$CacheDirectory,
    [string]$NodeArchive,
    [string]$McpArchive,
    [string]$OutputDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($env:OS -ne 'Windows_NT') { throw 'Este empaquetado requiere Windows x64.' }
$repo = Split-Path $PSScriptRoot -Parent
if (!$CacheDirectory) { $CacheDirectory = Join-Path $repo '.local/windows-bundle' }
if (!$OutputDirectory) { $OutputDirectory = Join-Path $repo 'dist' }
$CacheDirectory = [IO.Path]::GetFullPath($CacheDirectory)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
New-Item -ItemType Directory -Force -Path $CacheDirectory, $OutputDirectory | Out-Null

function Invoke-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program terminó con código $LASTEXITCODE" }
}
function Get-VerifiedArchive([string]$Path, [string]$Url, [string]$Hash) {
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) {
        Invoke-WebRequest -Uri $Url -OutFile $Path
    }
    if ((Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash -ne $Hash) {
        throw "SHA256 incorrecto: $Path"
    }
    return (Resolve-Path -LiteralPath $Path).Path
}

Push-Location $repo
$oldPath = $env:PATH
try {
    if (!$DesktopBinary) {
        Invoke-Checked cargo @('build', '--locked', '--release')
        $desktopMetadata = & cargo metadata --format-version 1 --no-deps
        if ($LASTEXITCODE -ne 0) { throw 'No se pudo localizar el target desktop.' }
        $DesktopBinary = Join-Path ($desktopMetadata | ConvertFrom-Json).target_directory 'release/nexode-desktop.exe'
    }
    if (!$EngineBinary) {
        $manifest = Join-Path $repo 'nexode-orbit/Cargo.toml'
        $desktopPath = (Resolve-Path -LiteralPath $DesktopBinary).Path
        $engineTarget = Split-Path (Split-Path $desktopPath -Parent) -Parent
        Invoke-Checked cargo @('build', '--manifest-path', $manifest, '--target-dir', $engineTarget, '--locked', '--release', '--no-default-features', '--bin', 'nexode')
        $EngineBinary = Join-Path (Split-Path $desktopPath -Parent) 'nexode.exe'
    }
    $DesktopBinary = (Resolve-Path -LiteralPath $DesktopBinary).Path
    $EngineBinary = (Resolve-Path -LiteralPath $EngineBinary).Path
    if (!$NodeArchive) { $NodeArchive = Join-Path $CacheDirectory 'node-v24.22.2-win-x64.zip' }
    if (!$McpArchive) { $McpArchive = Join-Path $CacheDirectory 'mcp-237d8d86.tar.gz' }
    $NodeArchive = Get-VerifiedArchive $NodeArchive 'https://nodejs.org/dist/v24.22.2/node-v24.22.2-win-x64.zip' 'ca1bff5d8ead13c9f2ea404a176db46bd430704d5b5113734049fa365e7799e2'
    $revision = '237d8d86e9f65906886e8b73d12a2afda2b2e6a4'
    $McpArchive = Get-VerifiedArchive $McpArchive "https://codeload.github.com/lucast1574/nexode-mcp/tar.gz/$revision" '8087cd448dbd8603f24abd2e1797de36008ac615bf09b4e3521a050705c36ec4'
    # Every invocation has its own staging area; a running installed app is untouched.
    $stage = Join-Path $CacheDirectory ('stage-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $stage | Out-Null
    Expand-Archive -LiteralPath $NodeArchive -DestinationPath $stage
    Invoke-Checked tar @('-xzf', $McpArchive, '-C', $stage)
    $node = Join-Path $stage 'node-v24.22.2-win-x64'
    $source = Join-Path $stage "nexode-mcp-$revision"
    $env:PATH = "$node;$oldPath"
    $npm = Join-Path $node 'npm.cmd'
    Invoke-Checked $npm @('--prefix', $source, 'ci', '--ignore-scripts')
    Invoke-Checked $npm @('--prefix', $source, 'run', 'build')
    Invoke-Checked $npm @('--prefix', $source, 'prune', '--omit=dev', '--ignore-scripts')
    $bundle = Join-Path $stage 'Nexode-Orbit'
    $mcp = Join-Path $bundle 'mcp'
    New-Item -ItemType Directory -Path $mcp -Force | Out-Null
    Copy-Item -LiteralPath $DesktopBinary -Destination (Join-Path $bundle 'nexode-desktop.exe')
    Copy-Item -LiteralPath $EngineBinary -Destination (Join-Path $bundle 'nexode.exe')
    Copy-Item -LiteralPath (Join-Path $node 'node.exe') -Destination $mcp
    Copy-Item -LiteralPath (Join-Path $node 'LICENSE') -Destination (Join-Path $mcp 'NODE-LICENSE')
    foreach ($entry in @('dist', 'node_modules', 'package.json', 'package-lock.json', 'LICENSE')) {
        $path = Join-Path $source $entry
        if (Test-Path -LiteralPath $path) { Copy-Item -LiteralPath $path -Destination $mcp -Recurse }
    }
    foreach ($document in @('README.md', 'HARNESS.md', 'APPIMAGE.md', 'SERVICE-CATALOG.md', 'VALIDATION.md')) {
        Copy-Item -LiteralPath (Join-Path $repo $document) -Destination $bundle
    }
    Copy-Item -LiteralPath (Join-Path $repo 'assets/fonts/OFL.txt') -Destination (Join-Path $bundle 'FONT-LICENSE.txt')
    if (Test-Path -LiteralPath (Join-Path $repo 'LICENSE')) {
        Copy-Item -LiteralPath (Join-Path $repo 'LICENSE') -Destination $bundle
    }
    Invoke-Checked (Join-Path $mcp 'node.exe') @('--check', (Join-Path $mcp 'dist/index.js'))
    $zip = Join-Path $OutputDirectory 'Nexode-Orbit-Windows-x64.zip'
    Compress-Archive -LiteralPath $bundle -DestinationPath $zip -Force
    $digest = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    Set-Content -LiteralPath "$zip.sha256" -Value "$digest  $([IO.Path]::GetFileName($zip))" -Encoding ascii
    Write-Host "Creado: $zip"
    Write-Host "Staging conservado para inspección: $bundle"
} finally {
    $env:PATH = $oldPath
    Pop-Location
}
