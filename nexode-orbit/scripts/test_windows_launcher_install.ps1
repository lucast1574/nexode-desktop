param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repoRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$installScript = Join-Path $repoRoot 'scripts\install.ps1'

$originalLocalAppData = $env:LOCALAPPDATA
$originalImportOnly = $env:NEXODE_INSTALL_PS1_IMPORT_ONLY
$testRoot = Join-Path $env:TEMP ("nexode-windows-launcher-tests-{0}" -f ([guid]::NewGuid().ToString('N')))
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null

function Assert-True($Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Assert-Equal($Expected, $Actual, [string]$Message) {
    if ($Expected -ne $Actual) {
        throw "$Message`nExpected: $Expected`nActual:   $Actual"
    }
}

function Assert-PathCount([string]$PathValue, [string]$Entry, [int]$ExpectedCount, [string]$Message) {
    $entryKey = ConvertTo-NexodePathKey $Entry
    $count = 0
    foreach ($candidate in (Split-NexodePathList $PathValue)) {
        if ((ConvertTo-NexodePathKey $candidate) -eq $entryKey) { $count += 1 }
    }
    Assert-Equal $ExpectedCount $count $Message
}

try {
    $env:LOCALAPPDATA = Join-Path $testRoot 'LocalAppData'
    $env:NEXODE_INSTALL_PS1_IMPORT_ONLY = '1'
    . $installScript

    $installDir = Join-Path $env:LOCALAPPDATA 'nexode\bin'
    $launcherPath = Join-Path $installDir 'nexode.exe'

    Write-Host 'test_launcher_path_localappdata'
    Assert-Equal $installDir (Get-DefaultNexodeInstallDir) 'default installer path should live under LOCALAPPDATA\nexode\bin'
    Assert-Equal $launcherPath (Join-Path (Get-DefaultNexodeInstallDir) 'nexode.exe') 'launcher path should be LOCALAPPDATA\nexode\bin\nexode.exe'

    Write-Host 'test_path_add_idempotent_dedupes_case_and_slashes'
    $installVariant = ($installDir.ToUpperInvariant() + '\')
    $currentPath = "C:\Tools;$installVariant;$installDir;C:\Tools\;C:\Other"
    $pathUpdate = Resolve-NexodePathUpdate -InstallDir $installDir -CurrentPath $currentPath
    Assert-Equal "$installDir;C:\Tools;C:\Tools\;C:\Other" $pathUpdate.Path 'install path update should prepend the canonical launcher dir without rewriting unrelated entries'
    Assert-PathCount $pathUpdate.Path $installDir 1 'updated PATH should contain exactly one nexode launcher dir'
    Assert-Equal 2 $pathUpdate.RemovedManagedEntries 'path update should remove both stale nexode launcher entries before re-adding one'
    Assert-Equal 0 $pathUpdate.RemovedDuplicateEntries 'path update should preserve unrelated duplicate entries'
    $secondUpdate = Resolve-NexodePathUpdate -InstallDir $installDir -CurrentPath $pathUpdate.Path
    Assert-Equal $false $secondUpdate.Changed 'second install path update should be idempotent'
    Assert-PathCount $secondUpdate.Path $installDir 1 'idempotent update should still contain exactly one launcher dir'

    Write-Host 'test_env_broadcast_called_once_when_path_changes'
    $script:setCalls = 0
    $script:broadcastCalls = 0
    $appliedPath = $null
    $setPathAction = { param($value) $script:setCalls += 1; $script:appliedPath = $value }
    $broadcastAction = { $script:broadcastCalls += 1 }
    $mockUpdate = Set-NexodeUserPath -InstallDir $installDir -CurrentPath 'C:\Tools' -SetUserPathAction $setPathAction -BroadcastAction $broadcastAction
    Assert-Equal 1 $script:setCalls 'user PATH setter should be called once when PATH changes'
    Assert-Equal 1 $script:broadcastCalls 'environment broadcast should be called once when PATH changes'
    Assert-Equal $true $mockUpdate.Broadcasted 'path update should report broadcast when changed'
    $noChangeUpdate = Set-NexodeUserPath -InstallDir $installDir -CurrentPath $script:appliedPath -SetUserPathAction $setPathAction -BroadcastAction $broadcastAction
    Assert-Equal 1 $script:setCalls 'user PATH setter should not be called when PATH is already correct'
    Assert-Equal 1 $script:broadcastCalls 'environment broadcast should not be called when PATH is unchanged'
    Assert-Equal $false $noChangeUpdate.Broadcasted 'unchanged path update should not report broadcast'

    Write-Host 'test_local_binary_version_output_parsing'
    Assert-Equal 'v0.47.0' (ConvertFrom-NexodeVersionOutput 'nexode v0.47.0 (f7f5898c)') 'local artifact version parser should accept normal nexode --version output'
    Assert-Equal 'v0.47.0' (ConvertFrom-NexodeVersionOutput 'nexode collects anonymous usage statistics. nexode v0.47.0 (f7f5898c)') 'local artifact version parser should accept fresh-profile telemetry before version output'
    Assert-Equal $null (ConvertFrom-NexodeVersionOutput 'not a nexode binary') 'local artifact version parser should reject unrelated output'
    $freshProfileBinary = Join-Path $testRoot 'fresh-profile-version.cmd'
    Set-Content -LiteralPath $freshProfileBinary -Value "@echo off`r`n>&2 echo nexode collects anonymous usage statistics.`r`necho nexode v0.47.0 (f7f5898c)" -NoNewline
    Assert-Equal 'v0.47.0' (Get-NexodeVersionFromBinary $freshProfileBinary) 'binary version probe should tolerate a successful fresh-profile telemetry notice on stderr'

    Write-Host 'test_windows_architecture_detection_prefers_native_arm64'
    Assert-Equal 'nexode-windows-x86_64' (Resolve-NexodeWindowsArtifact @('X64', 'AMD64')) 'x64 Windows should select the x64 release asset'
    Assert-Equal 'nexode-windows-aarch64' (Resolve-NexodeWindowsArtifact @('Arm64')) 'native ARM64 Windows should select the ARM64 release asset'
    Assert-Equal 'nexode-windows-aarch64' (Resolve-NexodeWindowsArtifact @('X64', 'AMD64', 'ARM64')) 'emulated x64 PowerShell on Windows ARM64 should prefer the native ARM64 release asset'
    Assert-Equal $null (Resolve-NexodeWindowsArtifact @('x86', 'unknown')) 'unsupported architectures should not silently select an asset'

    Write-Host 'test_release_checksum_validation'
    $checksumFile = Join-Path $testRoot 'checksum.bin'
    Set-Content -LiteralPath $checksumFile -Value 'known-content' -NoNewline
    $digest = (Get-FileHash -LiteralPath $checksumFile -Algorithm SHA256).Hash.ToLowerInvariant()
    $manifest = "$digest  nested/path/nexode-windows-x86_64.exe"
    $manifestBytes = [System.Text.Encoding]::UTF8.GetBytes($manifest)
    Assert-Equal $manifest (ConvertFrom-NexodeWebContent -Content $manifest) 'web response decoder should preserve string content'
    Assert-Equal $manifest (ConvertFrom-NexodeWebContent -Content $manifestBytes) 'web response decoder should decode Windows PowerShell 5.1 byte-array content as UTF-8'
    Assert-Equal $digest (Get-NexodeSha256FromManifest -ManifestText (ConvertFrom-NexodeWebContent -Content $manifestBytes) -AssetName 'nexode-windows-x86_64.exe') 'checksum parser should accept a manifest decoded from a byte-array web response'
    Assert-Equal $digest (Get-NexodeSha256FromManifest -ManifestText $manifest -AssetName 'nexode-windows-x86_64.exe') 'checksum parser should match release assets by file name'
    Assert-Equal $null (Get-NexodeSha256FromManifest -ManifestText $manifest -AssetName 'missing.exe') 'checksum parser should fail closed when the requested asset is absent'
    Assert-Equal $digest (Assert-NexodeFileChecksum -FilePath $checksumFile -ExpectedSha256 $digest -AssetName 'nexode-windows-x86_64.exe') 'checksum validation should accept the matching digest'
    $checksumThrew = $false
    try {
        Assert-NexodeFileChecksum -FilePath $checksumFile -ExpectedSha256 ('0' * 64) -AssetName 'nexode-windows-x86_64.exe' | Out-Null
    } catch {
        $checksumThrew = $true
    }
    Assert-Equal $true $checksumThrew 'checksum validation should reject a mismatched digest'
    Assert-Equal $false (Test-Path -LiteralPath $checksumFile) 'checksum validation should delete a mismatched download'
    $armManifest = "$digest  nested/path/nexode-windows-aarch64.exe"
    Assert-Equal $digest (Get-NexodeSha256FromManifest -ManifestText $armManifest -AssetName 'nexode-windows-aarch64.exe') 'checksum parser should match the Windows ARM64 release asset'

    Write-Host 'test_temp_cleanup_tolerates_windows_short_paths'
    $installText = Get-Content -LiteralPath $installScript -Raw
    Assert-True ($installText -match '(?m)^\s*Get-Item -LiteralPath \$TempDir -ErrorAction SilentlyContinue \|\r?\n\s*Remove-Item -Recurse -Force -ErrorAction SilentlyContinue\s*$') 'temporary cleanup should resolve the item before removal so Windows 8.3 TEMP paths are tolerated'

    Write-Host 'test_optional_setup_and_source_build_are_opt_in'
    Assert-Equal $false ([bool]$ConfigureAlacritty) 'core install should not install an optional terminal by default'
    Assert-Equal $false ([bool]$ConfigureHotkey) 'core install should not add login persistence by default'
    Assert-Equal $false ([bool]$BuildFromSource) 'installer should not start a source build by default'
    Assert-True ($installText.Contains('will not start a long source build automatically')) 'missing release assets should produce an explicit source-build opt-in message'
    Assert-True ($installText.Contains('"--locked", "-p", "nexode", "--bin", "nexode"')) 'source-build fallback should compile only the locked nexode binary target'

    Write-Host 'test_hotkey_shortcut_script_is_valid_powershell'
    $shortcutScript = Get-NexodeHotkeyShortcutScript -StartupShortcutPath "C:\Users\Test User\AppData\Roaming\nexode's hotkey.lnk" -NexodeExePath "C:\Program Files\nexode's bin\nexode.exe"
    Assert-True ($shortcutScript -match "(?m)^\`$shortcut\.TargetPath = 'powershell\.exe'\r?$") 'shortcut script should target PowerShell directly'
    Assert-True ($shortcutScript -match '(?m)^\$shortcut\.Arguments = .*ExecutionPolicy RemoteSigned.*--listen-windows-hotkey.*\r?$') 'shortcut script should launch the native listener with RemoteSigned'
    Assert-True (-not $shortcutScript.Contains('ExecutionPolicy Bypass')) 'shortcut script should not bypass PowerShell execution policy'
    Assert-True ($shortcutScript -match '(?m)^\$shortcut\.WindowStyle = 7\r?$') 'shortcut script should assign WindowStyle without escaping the variable name'
    Assert-True ($shortcutScript -match '(?m)^\$shortcut\.Save\(\)\r?$') 'shortcut script should call Save without escaping the variable name'
    Assert-True (-not $shortcutScript.Contains('`$shortcut')) 'shortcut script should not contain literal backticks before shortcut variables'
    [void][scriptblock]::Create($shortcutScript)
    Assert-True ($installText.Contains('NEXODE_WINDOWS_SETUP_SKIP_PROCESS_LIFECYCLE')) 'isolated verification should be able to create shortcut files without stopping or spawning real user listeners'

    Write-Host 'test_upgrade_replaces_launcher_no_extra_path'
    $sourceDir = Join-Path $testRoot 'sources'
    New-Item -ItemType Directory -Path $sourceDir -Force | Out-Null
    $sourceV1 = Join-Path $sourceDir 'nexode-v1.exe'
    $sourceV2 = Join-Path $sourceDir 'nexode-v2.exe'
    Set-Content -Path $sourceV1 -Value 'version-one' -NoNewline
    Set-Content -Path $sourceV2 -Value 'version-two' -NoNewline
    Install-NexodeLauncher -SourcePath $sourceV1 -LauncherPath $launcherPath | Out-Null
    Install-NexodeLauncher -SourcePath $sourceV2 -LauncherPath $launcherPath | Out-Null
    Assert-Equal 'version-two' (Get-Content -Path $launcherPath -Raw) 'upgrade should replace launcher contents with the new build'
    $tempLaunchers = @(Get-ChildItem -LiteralPath $installDir -Filter '.nexode-launcher-*.tmp.exe' -Force -ErrorAction SilentlyContinue)
    Assert-Equal 0 $tempLaunchers.Count 'launcher upgrade should clean temporary files'
    $upgradePath = Resolve-NexodePathUpdate -InstallDir $installDir -CurrentPath $pathUpdate.Path
    Assert-Equal $false $upgradePath.Changed 'upgrade should not add another PATH entry when launcher dir is already present'
    Assert-PathCount $upgradePath.Path $installDir 1 'upgrade should preserve exactly one launcher PATH entry'

    Write-Host 'test_running_launcher_can_be_replaced'
    $runningDir = Join-Path $testRoot 'running-launcher'
    New-Item -ItemType Directory -Path $runningDir -Force | Out-Null
    $runningLauncher = Join-Path $runningDir 'nexode.exe'
    $replacementLauncher = Join-Path $runningDir 'replacement.exe'
    Copy-Item -LiteralPath (Join-Path $env:WINDIR 'System32\ping.exe') -Destination $runningLauncher
    Copy-Item -LiteralPath (Join-Path $env:WINDIR 'System32\where.exe') -Destination $replacementLauncher
    $runningProcess = Start-Process -FilePath $runningLauncher -ArgumentList @('-n', '30', '127.0.0.1') -WindowStyle Hidden -PassThru
    try {
        Start-Sleep -Milliseconds 500
        Assert-Equal $false $runningProcess.HasExited 'test launcher process should still be running before replacement'

        Install-NexodeLauncher -SourcePath $replacementLauncher -LauncherPath $runningLauncher | Out-Null

        Assert-Equal (Get-FileHash -LiteralPath $replacementLauncher -Algorithm SHA256).Hash (Get-FileHash -LiteralPath $runningLauncher -Algorithm SHA256).Hash 'live upgrade should place the replacement at the stable launcher path'
        Assert-Equal $false $runningProcess.HasExited 'live upgrade should not terminate the process using the previous launcher'
        $runningBackups = @(Get-ChildItem -LiteralPath $runningDir -Filter '.nexode-launcher-old-*.exe' -Force -ErrorAction SilentlyContinue)
        Assert-Equal 1 $runningBackups.Count 'live upgrade should retain exactly one locked old launcher until the process exits'
    } finally {
        Stop-ProcessTree -ProcessId $runningProcess.Id
        try { Wait-Process -Id $runningProcess.Id -Timeout 10 -ErrorAction SilentlyContinue } catch {}
    }
    Remove-NexodeStaleLauncherBackups -LauncherDir $runningDir
    $runningBackups = @(Get-ChildItem -LiteralPath $runningDir -Filter '.nexode-launcher-old-*.exe' -Force -ErrorAction SilentlyContinue)
    Assert-Equal 0 $runningBackups.Count 'stale live-upgrade launchers should be removable after the old process exits'

    Write-Host 'test_launcher_replacement_failure_rolls_back'
    $rollbackDir = Join-Path $testRoot 'launcher-rollback'
    New-Item -ItemType Directory -Path $rollbackDir -Force | Out-Null
    $rollbackLauncher = Join-Path $rollbackDir 'nexode.exe'
    $rollbackSource = Join-Path $rollbackDir 'replacement.exe'
    Set-Content -LiteralPath $rollbackLauncher -Value 'known-good' -NoNewline
    Set-Content -LiteralPath $rollbackSource -Value 'replacement' -NoNewline
    $script:injectLauncherMoveFailure = $true
    function Move-Item {
        [CmdletBinding()]
        param(
            [string]$Path,
            [string]$LiteralPath,
            [Parameter(Mandatory = $true)][string]$Destination,
            [switch]$Force
        )
        $source = if ($PSBoundParameters.ContainsKey('LiteralPath')) { $LiteralPath } else { $Path }
        if ($script:injectLauncherMoveFailure -and $source -like '*.tmp.exe' -and $Destination -eq $rollbackLauncher) {
            $script:injectLauncherMoveFailure = $false
            throw 'simulated final launcher move failure'
        }
        $moveArgs = @{ Destination = $Destination }
        if ($PSBoundParameters.ContainsKey('LiteralPath')) { $moveArgs.LiteralPath = $LiteralPath } else { $moveArgs.Path = $Path }
        if ($Force) { $moveArgs.Force = $true }
        Microsoft.PowerShell.Management\Move-Item @moveArgs
    }
    $rollbackThrew = $false
    try {
        Install-NexodeLauncher -SourcePath $rollbackSource -LauncherPath $rollbackLauncher | Out-Null
    } catch {
        $rollbackThrew = $true
    } finally {
        Remove-Item Function:\Move-Item -ErrorAction SilentlyContinue
    }
    Assert-Equal $true $rollbackThrew 'launcher replacement should surface a final move failure'
    Assert-Equal 'known-good' (Get-Content -LiteralPath $rollbackLauncher -Raw) 'launcher replacement should restore the previous stable launcher after a final move failure'
    Assert-Equal 0 @(Get-ChildItem -LiteralPath $rollbackDir -Filter '.nexode-launcher-*.tmp.exe' -Force -ErrorAction SilentlyContinue).Count 'rollback should remove temporary launcher files'
    Assert-Equal 0 @(Get-ChildItem -LiteralPath $rollbackDir -Filter '.nexode-launcher-old-*.exe' -Force -ErrorAction SilentlyContinue).Count 'rollback should restore rather than retain the previous launcher backup'

    Write-Host 'test_uninstall_removes_launcher_and_only_nexode_path'
    $removeCurrentPath = "$installDir;C:\Keep;$installVariant;C:\Keep"
    $removeUpdate = Resolve-NexodePathUpdate -InstallDir $installDir -CurrentPath $removeCurrentPath -RemoveOnly
    Assert-Equal 'C:\Keep;C:\Keep' $removeUpdate.Path 'uninstall path cleanup should remove only nexode-managed entries and preserve unrelated entries'
    Assert-Equal 2 $removeUpdate.RemovedManagedEntries 'uninstall path cleanup should remove all nexode launcher dir variants'
    Assert-PathCount $removeUpdate.Path $installDir 0 'uninstall path cleanup should leave no nexode launcher dir entries'

    Write-Host 'All Windows launcher install tests passed.' -ForegroundColor Green
} finally {
    if ($null -eq $originalLocalAppData) { Remove-Item Env:LOCALAPPDATA -ErrorAction SilentlyContinue } else { $env:LOCALAPPDATA = $originalLocalAppData }
    if ($null -eq $originalImportOnly) { Remove-Item Env:NEXODE_INSTALL_PS1_IMPORT_ONLY -ErrorAction SilentlyContinue } else { $env:NEXODE_INSTALL_PS1_IMPORT_ONLY = $originalImportOnly }
    Remove-Item -LiteralPath $testRoot -Recurse -Force -ErrorAction SilentlyContinue
}
