[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][int]$ParentPid,
    [Parameter(Mandatory = $true)][string]$CurrentExe,
    [Parameter(Mandatory = $true)][string]$TargetDir,
    [Parameter(Mandatory = $true)][string]$ReadyFile,
    [Parameter(Mandatory = $true)][string]$ExpectedVersion,
    [Parameter(Mandatory = $true)][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][string]$LockFile,
    [Parameter(Mandatory = $true)][string]$FailureLog,
    [switch]$Silent,
    [switch]$NoRestartOnRollback
)

$ErrorActionPreference = "Stop"
$phase = "initialize"
$lock = $null
$backup = "$TargetDir.update-backup"
$newProcess = $null
$targetMoved = $false
$installStarted = $false
$oldApplicationStopped = $false
$hadUninstallRegistration = $false
$uninstallKey = "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
$registryBackup = "$FailureLog.registry.reg"
$regExe = "$env:SystemRoot\System32\reg.exe"

function Set-Phase([string]$value) {
    $script:phase = $value
    Write-Output "gogoke update phase=$value"
}

function Write-Failure([string]$message) {
    $parent = Split-Path -Parent $FailureLog
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [IO.File]::WriteAllText($FailureLog, "phase=$phase`r`n$message`r`n", [Text.Encoding]::UTF8)
}

function Assert-Absolute([string]$path, [string]$label) {
    $isDriveAbsolute = $path -match '^[A-Za-z]:[\\/]'
    $isUncAbsolute = $path -match '^[\\/]{2}[^\\/]+[\\/][^\\/]+(?:[\\/]|$)'
    if (-not ($isDriveAbsolute -or $isUncAbsolute)) { throw "$label must be absolute" }
}

function Get-Sha256Hex([string]$path) {
    $stream = [IO.File]::OpenRead($path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try {
            $hash = $sha256.ComputeHash($stream)
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
    return ([BitConverter]::ToString($hash)).Replace('-', '')
}

function Invoke-Reg([string[]]$arguments) {
    if (-not (Test-Path -LiteralPath $regExe -PathType Leaf)) { throw "reg.exe is unavailable" }
    $previousPreference = $ErrorActionPreference
    $previousExitCode = $global:LASTEXITCODE
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = $null
        & $regExe @arguments 1>$null 2>$null
        $exitCode = $global:LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
        $global:LASTEXITCODE = $previousExitCode
    }
    if ($null -eq $exitCode) { throw "reg.exe did not return an exit code" }
    return [int]$exitCode
}

try {
    Assert-Absolute $Installer "Installer"
    Assert-Absolute $CurrentExe "CurrentExe"
    Assert-Absolute $TargetDir "TargetDir"
    Assert-Absolute $ReadyFile "ReadyFile"
    Assert-Absolute $LockFile "LockFile"
    Assert-Absolute $FailureLog "FailureLog"
    if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) { throw "verified installer is missing" }
    if ($ExpectedSha256 -notmatch '^[0-9A-Fa-f]{64}$') { throw "ExpectedSha256 is invalid" }

    Set-Phase "lock"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LockFile) | Out-Null
    $lock = [IO.File]::Open($LockFile, [IO.FileMode]::OpenOrCreate,
        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)

    Set-Phase "wait-parent"
    $parent = Get-Process -Id $ParentPid -ErrorAction SilentlyContinue
    if ($parent -and -not $parent.WaitForExit(30000)) { throw "gogoke did not exit within 30 seconds" }
    $oldApplicationStopped = $true

    Set-Phase "verify-installer"
    $installerHash = Get-Sha256Hex $Installer
    if (-not [string]::Equals($installerHash, $ExpectedSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "installer changed after application authorization"
    }

    Set-Phase "prepare-target"
    if ((Test-Path -LiteralPath $TargetDir) -and
        ((Get-Item -LiteralPath $TargetDir -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "target directory is a reparse point"
    }
    if (Test-Path -LiteralPath $backup) { throw "an earlier update backup still exists" }
    $hadUninstallRegistration = (Invoke-Reg @("query", $uninstallKey)) -eq 0
    if ($hadUninstallRegistration) {
        if ((Invoke-Reg @("export", $uninstallKey, $registryBackup, "/y")) -ne 0) {
            throw "could not back up gogoke install registration"
        }
    }
    if (Test-Path -LiteralPath $TargetDir) {
        $owned = Test-Path -LiteralPath (Join-Path $TargetDir "gogoke.exe") -PathType Leaf
        if (-not $owned) { throw "target directory is not owned by gogoke" }
        Move-Item -LiteralPath $TargetDir -Destination $backup
        $targetMoved = $true
    }

    Set-Phase "install"
    $installerArgs = @()
    if ($Silent) { $installerArgs += "/S" }
    $installerArgs += ("/D=" + $TargetDir)
    $installStarted = $true
    $installerProcess = Start-Process -FilePath $Installer -ArgumentList $installerArgs -Wait -PassThru
    if ($installerProcess.ExitCode -ne 0) { throw "installer exited with code $($installerProcess.ExitCode)" }

    Set-Phase "launch"
    $newExe = Join-Path $TargetDir "gogoke.exe"
    if (-not (Test-Path -LiteralPath $newExe -PathType Leaf)) { throw "installed gogoke executable is missing" }
    if (Test-Path -LiteralPath $ReadyFile) { Remove-Item -LiteralPath $ReadyFile -Force }
    $newProcess = Start-Process -FilePath $newExe -ArgumentList @("--gogoke-update-ready=$ReadyFile") -PassThru

    Set-Phase "wait-ready"
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $ReadyFile -PathType Leaf) {
            $readyVersion = ([IO.File]::ReadAllText($ReadyFile)).Trim()
            if ($readyVersion -ne $ExpectedVersion) { throw "new application reported the wrong version" }
            Set-Phase "complete"
            Remove-Item -LiteralPath $ReadyFile -Force
            if (Test-Path -LiteralPath $backup) { Remove-Item -LiteralPath $backup -Recurse -Force }
            if (Test-Path -LiteralPath $registryBackup) { Remove-Item -LiteralPath $registryBackup -Force }
            if (Test-Path -LiteralPath $FailureLog) { Remove-Item -LiteralPath $FailureLog -Force }
            exit 0
        }
        $newProcess.Refresh()
        if ($newProcess.HasExited) { throw "new application exited before reporting readiness" }
        Start-Sleep -Milliseconds 100
    }
    throw "new application did not report readiness within 30 seconds"
} catch {
    Write-Failure $_.Exception.Message
    if ($newProcess -and -not $newProcess.HasExited) {
        Stop-Process -Id $newProcess.Id -Force -ErrorAction SilentlyContinue
        $null = $newProcess.WaitForExit(5000)
    }
    try {
        Set-Phase "rollback"
        if ($targetMoved -or $installStarted) {
            if ($installStarted -and -not $targetMoved) {
                $newUninstaller = Join-Path $TargetDir "uninstall.exe"
                if (Test-Path -LiteralPath $newUninstaller -PathType Leaf) {
                    $null = Start-Process -FilePath $newUninstaller -ArgumentList "/S" -Wait -PassThru
                }
            }
            if (Test-Path -LiteralPath $TargetDir) { Remove-Item -LiteralPath $TargetDir -Recurse -Force }
            if ($targetMoved) {
                if (-not (Test-Path -LiteralPath $backup)) { throw "update backup is missing during rollback" }
                Move-Item -LiteralPath $backup -Destination $TargetDir
            }
            if ($installStarted -and $hadUninstallRegistration -and
                -not (Test-Path -LiteralPath $registryBackup -PathType Leaf)) {
                throw "gogoke install registration backup is missing during rollback"
            }
            if (Test-Path -LiteralPath $registryBackup -PathType Leaf) {
                if ((Invoke-Reg @("import", $registryBackup)) -ne 0) {
                    throw "could not restore gogoke install registration"
                }
                Remove-Item -LiteralPath $registryBackup -Force
            } elseif ($installStarted -and -not $hadUninstallRegistration -and
                (Invoke-Reg @("query", $uninstallKey)) -eq 0) {
                if ((Invoke-Reg @("delete", $uninstallKey, "/f")) -ne 0) {
                    throw "could not remove new gogoke install registration"
                }
            }
        } elseif (Test-Path -LiteralPath $registryBackup -PathType Leaf) {
            Remove-Item -LiteralPath $registryBackup -Force
        }
        if ($oldApplicationStopped -and -not $NoRestartOnRollback -and
            (Test-Path -LiteralPath $CurrentExe -PathType Leaf)) {
            Start-Process -FilePath $CurrentExe | Out-Null
        }
    } catch {
        Write-Failure "rollback failed: $($_.Exception.Message)"
    }
    exit 1
} finally {
    if ($lock) { $lock.Dispose() }
}
