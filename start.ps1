param([string]$Distribution = 'Ubuntu')
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
if (-not (Test-Path node_modules)) { npm ci; if ($LASTEXITCODE) { throw 'npm ci falló' } }
npm run build
if ($LASTEXITCODE) { throw 'La compilación de la interfaz falló' }
$windowsPath = $PSScriptRoot.Replace('\', '/')
$linuxPath = (wsl -d $Distribution --exec wslpath -a $windowsPath).Trim()
# These are data arguments passed to bash, never interpolated into shell code.
wsl -d $Distribution --exec bash ./scripts/start.sh $linuxPath
