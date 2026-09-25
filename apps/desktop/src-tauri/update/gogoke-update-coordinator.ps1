[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$ReleaseSetDirectory,
    [Parameter(Mandatory = $true)][int]$ParentPid,
    [Parameter(Mandatory = $true)][string]$CurrentExe,
    [Parameter(Mandatory = $true)][string]$TargetDir,
    [Parameter(Mandatory = $true)][string]$ReadyFile,
    [Parameter(Mandatory = $true)][string]$ExpectedVersion,
    [Parameter(Mandatory = $true)][string]$ExpectedGenerationId,
    [Parameter(Mandatory = $true)][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][string]$ExpectedManifestSha256,
    [Parameter(Mandatory = $true)][string]$ExpectedSignatureSha256,
    [Parameter(Mandatory = $true)][string]$LockFile,
    [Parameter(Mandatory = $true)][string]$FailureLog,
    [Parameter(Mandatory = $true)][string]$StateFile,
    [Parameter(Mandatory = $true)][string]$RetentionNoticeFile,
    [switch]$Silent,
    [switch]$NoRestartOnRollback
)

$ErrorActionPreference = "Stop"
$phase = "initialize"
$lock = $null
$lifecycleLock = $null
$backup = "$TargetDir.update-backup"
$newProcess = $null
$targetMoved = $false
$installStarted = $false
$oldApplicationStopped = $false
$hadUninstallRegistration = $false
$oldInstallInstanceId = $null
$newInstallInstanceId = $null
$oldExeSha256 = $null
$uninstallKey = "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
$uninstallRegistryPath = "Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
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

function Get-InstallRegistration {
    if (-not (Test-Path -LiteralPath $uninstallRegistryPath)) { return $null }
    return Get-ItemProperty -LiteralPath $uninstallRegistryPath
}

function Assert-PhysicalDirectory([string]$path, [string]$label) {
    if (-not (Test-Path -LiteralPath $path -PathType Container)) { throw "$label is missing" }
    $item = Get-Item -LiteralPath $path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$label is a reparse point"
    }
}

function Write-RetainedBackupState {
    $state = [IO.File]::ReadAllText($StateFile) | ConvertFrom-Json -ErrorAction Stop
    if ($state.offer.version -cne $ExpectedVersion) {
        throw "prepared update state changed before completion"
    }
    [IO.File]::WriteAllText($RetentionNoticeFile,
        "The update is ready. The previous installation remains at $backup for owned-file cleanup.`r`n",
        [Text.UTF8Encoding]::new($false))
    $state.status = "installed_backup_retained"
    $state.lastError = "Previous installation retained for owned-file cleanup: $backup"
    $partial = "$StateFile.$([Guid]::NewGuid().ToString('N')).part"
    $payload = $state | ConvertTo-Json -Depth 10
    [IO.File]::WriteAllText($partial, $payload, [Text.UTF8Encoding]::new($false))
    try {
        [IO.File]::Replace($partial, $StateFile, $null)
    } finally {
        if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force }
    }
}

function Assert-InstallRegistration([string]$expectedInstance) {
    $registration = Get-InstallRegistration
    if (-not $registration -or $registration.InstallInstanceId -ne $expectedInstance -or
        $registration.InstallDomain -ne "OWNER_RELEASE" -or
        -not [string]::Equals([IO.Path]::GetFullPath($registration.InstallLocation),
            $targetFull, [StringComparison]::OrdinalIgnoreCase)) {
        throw "gogoke installation registration changed during update"
    }
}

function Acquire-LifecycleLock {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        try {
            return [IO.File]::Open($lifecyclePath, [IO.FileMode]::OpenOrCreate,
                [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        } catch [IO.IOException] {
            Start-Sleep -Milliseconds 100
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "gogoke install lifecycle lock was not reacquired"
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
    Assert-Absolute $ReleaseSetDirectory "ReleaseSetDirectory"
    Assert-Absolute $CurrentExe "CurrentExe"
    Assert-Absolute $TargetDir "TargetDir"
    Assert-Absolute $ReadyFile "ReadyFile"
    Assert-Absolute $LockFile "LockFile"
    Assert-Absolute $FailureLog "FailureLog"
    Assert-Absolute $StateFile "StateFile"
    Assert-Absolute $RetentionNoticeFile "RetentionNoticeFile"
    if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) { throw "verified installer is missing" }
    if ($ExpectedSha256 -notmatch '^[0-9A-Fa-f]{64}$') { throw "ExpectedSha256 is invalid" }
    if ($ExpectedManifestSha256 -notmatch '^[0-9A-Fa-f]{64}$' -or
        $ExpectedSignatureSha256 -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "expected signed sidecar hashes are invalid"
    }
    if ($ExpectedGenerationId -cnotmatch '^[0-9a-f]{64}$') { throw "ExpectedGenerationId is invalid" }
    $targetFull = [IO.Path]::GetFullPath($TargetDir).TrimEnd('\')
    $targetRoot = [IO.Path]::GetPathRoot($targetFull).TrimEnd('\')
    if ([string]::Equals($targetFull, $targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "target directory cannot be a volume root"
    }
    $expectedCurrent = Join-Path $targetFull "gogoke.exe"
    if (-not [string]::Equals([IO.Path]::GetFullPath($CurrentExe), $expectedCurrent,
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "current executable does not belong to target directory"
    }
    if (-not [string]::Equals([IO.Path]::GetFullPath((Split-Path -Parent $Installer)),
        [IO.Path]::GetFullPath($ReleaseSetDirectory), [StringComparison]::OrdinalIgnoreCase)) {
        throw "installer is outside the signed release set"
    }

    Set-Phase "lock"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LockFile) | Out-Null
    $lock = [IO.File]::Open($LockFile, [IO.FileMode]::OpenOrCreate,
        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $lifecyclePath = Join-Path (Split-Path -Parent $targetFull) "gogoke-install-lifecycle.lock"
    $lifecycleLock = Acquire-LifecycleLock

    Set-Phase "wait-parent"
    $parent = Get-Process -Id $ParentPid -ErrorAction SilentlyContinue
    if ($parent -and -not $parent.WaitForExit(30000)) { throw "gogoke did not exit within 30 seconds" }
    $oldApplicationStopped = $true

    Set-Phase "verify-release-set"
    $verifyArgument = '--gogoke-verify-install-set="' + $ReleaseSetDirectory + '"'
    $verifyProcess = Start-Process -FilePath $CurrentExe -ArgumentList @($verifyArgument) -Wait -PassThru -WindowStyle Hidden
    if ($verifyProcess.ExitCode -ne 0) { throw "Owner signed release set failed verification" }
    $manifestFile = Join-Path $ReleaseSetDirectory "SHA256SUMS.windows"
    $signatureFile = Join-Path $ReleaseSetDirectory "SHA256SUMS.windows.sig"
    if (-not [string]::Equals((Get-Sha256Hex $manifestFile), $ExpectedManifestSha256,
        [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals((Get-Sha256Hex $signatureFile), $ExpectedSignatureSha256,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "Owner signed release sidecars changed after application authorization"
    }
    $installerHash = Get-Sha256Hex $Installer
    if (-not [string]::Equals($installerHash, $ExpectedSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "installer changed after application authorization"
    }

    Set-Phase "prepare-target"
    if ((Test-Path -LiteralPath $TargetDir) -and
        ((Get-Item -LiteralPath $TargetDir -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "target directory is a reparse point"
    }
    if (Test-Path -LiteralPath $backup) {
        throw "previous installation backup is retained for owned-file cleanup; full update deferred"
    }
    $hadUninstallRegistration = (Invoke-Reg @("query", $uninstallKey)) -eq 0
    if (-not $hadUninstallRegistration) { throw "old gogoke install registration is missing" }
    if ($hadUninstallRegistration) {
        $oldRegistration = Get-InstallRegistration
        if (-not $oldRegistration -or $oldRegistration.InstallDomain -ne "OWNER_RELEASE" -or
            [string]::IsNullOrWhiteSpace($oldRegistration.InstallInstanceId)) {
            throw "old gogoke install registration is invalid"
        }
        $oldInstallInstanceId = [string]$oldRegistration.InstallInstanceId
        Assert-InstallRegistration $oldInstallInstanceId
        if ((Invoke-Reg @("export", $uninstallKey, $registryBackup, "/y")) -ne 0) {
            throw "could not back up gogoke install registration"
        }
    }
    if (Test-Path -LiteralPath $TargetDir) {
        $owned = Test-Path -LiteralPath (Join-Path $TargetDir "gogoke.exe") -PathType Leaf
        if (-not $owned) { throw "target directory is not owned by gogoke" }
        $oldExeSha256 = Get-Sha256Hex (Join-Path $TargetDir "gogoke.exe")
        Move-Item -LiteralPath $TargetDir -Destination $backup
        $targetMoved = $true
    }

    Set-Phase "install"
    $installerArgs = @()
    if ($Silent) { $installerArgs += "/S" }
    $installerArgs += ("/D=" + $TargetDir)
    $installStarted = $true
    # NSIS takes the same sibling lock before it touches the target. Keep the
    # update-cache lock while ownership transfers, then recheck instance identity.
    $lifecycleLock.Dispose()
    $lifecycleLock = $null
    $installerProcess = Start-Process -FilePath $Installer -ArgumentList $installerArgs -Wait -PassThru
    $lifecycleLock = Acquire-LifecycleLock
    if ($installerProcess.ExitCode -ne 0) { throw "installer exited with code $($installerProcess.ExitCode)" }
    if ($targetMoved -and -not (Test-Path -LiteralPath $backup -PathType Container)) {
        throw "old gogoke installation backup changed during installer handoff"
    }
    if ($targetMoved) { Assert-PhysicalDirectory $backup "old installation backup" }
    if ($targetMoved -and -not [string]::Equals(
        (Get-Sha256Hex (Join-Path $backup "gogoke.exe")), $oldExeSha256,
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "old gogoke executable backup changed during installer handoff"
    }
    $newRegistration = Get-InstallRegistration
    if (-not $newRegistration -or [string]::IsNullOrWhiteSpace($newRegistration.InstallInstanceId) -or
        $newRegistration.InstallInstanceId -eq $oldInstallInstanceId -or
        $newRegistration.DisplayVersion -cne $ExpectedVersion) {
        throw "new gogoke installation registration is invalid"
    }
    $newInstallInstanceId = [string]$newRegistration.InstallInstanceId
    Assert-InstallRegistration $newInstallInstanceId
    if (-not [string]::Equals((Get-Sha256Hex (Join-Path $TargetDir "SHA256SUMS.windows")),
        $ExpectedManifestSha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals((Get-Sha256Hex (Join-Path $TargetDir "SHA256SUMS.windows.sig")),
            $ExpectedSignatureSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "installed Owner signed sidecars do not match the prepared set"
    }

    Set-Phase "launch"
    $newExe = Join-Path $TargetDir "gogoke.exe"
    if (-not (Test-Path -LiteralPath $newExe -PathType Leaf)) { throw "installed gogoke executable is missing" }
    if (Test-Path -LiteralPath $ReadyFile) { Remove-Item -LiteralPath $ReadyFile -Force }
    $newProcess = Start-Process -FilePath $newExe -ArgumentList @("--gogoke-update-ready=$ReadyFile") -PassThru

    Set-Phase "wait-ready"
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        Assert-InstallRegistration $newInstallInstanceId
        if (Test-Path -LiteralPath $ReadyFile -PathType Leaf) {
            try {
                $ready = [IO.File]::ReadAllText($ReadyFile) | ConvertFrom-Json -ErrorAction Stop
            } catch {
                throw "new application readiness receipt is not JSON"
            }
            if ($ready.version -cne $ExpectedVersion -or
                $ready.generationId -cne $ExpectedGenerationId) {
                throw "new application reported the wrong version or resource generation"
            }
            Write-RetainedBackupState
            Set-Phase "complete"
            Remove-Item -LiteralPath $ReadyFile -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $registryBackup) {
                Remove-Item -LiteralPath $registryBackup -Force -ErrorAction SilentlyContinue
            }
            if (Test-Path -LiteralPath $FailureLog) {
                Remove-Item -LiteralPath $FailureLog -Force -ErrorAction SilentlyContinue
            }
            exit 0
        }
        $newProcess.Refresh()
        if ($newProcess.HasExited) { throw "new application exited before reporting readiness" }
        Start-Sleep -Milliseconds 100
    }
    throw "new application did not report readiness within 30 seconds"
} catch {
    $initialError = $_.Exception.Message
    Write-Failure $initialError
    if (Test-Path -LiteralPath $RetentionNoticeFile) {
        Remove-Item -LiteralPath $RetentionNoticeFile -Force -ErrorAction SilentlyContinue
    }
    if ($newProcess -and -not $newProcess.HasExited) {
        Stop-Process -Id $newProcess.Id -Force -ErrorAction SilentlyContinue
        $null = $newProcess.WaitForExit(5000)
    }
    try {
        Set-Phase "rollback"
        if (($targetMoved -or $installStarted) -and -not $lifecycleLock) {
            throw "lifecycle lock was not held; target and backup retained for safe recovery"
        }
        $registrationDuringRollback = Get-InstallRegistration
        if ($registrationDuringRollback -and
            $registrationDuringRollback.InstallInstanceId -ne $oldInstallInstanceId -and
            $registrationDuringRollback.InstallInstanceId -ne $newInstallInstanceId) {
            throw "installation instance changed; target and backup retained for safe recovery"
        }
        if ($targetMoved -or $installStarted) {
            if (Test-Path -LiteralPath $TargetDir) {
                Assert-PhysicalDirectory $TargetDir "failed new installation"
                $failedTarget = "$TargetDir.update-failed-$([Guid]::NewGuid().ToString('N'))"
                Move-Item -LiteralPath $TargetDir -Destination $failedTarget
                Write-Failure "$initialError`r`nfailed new installation retained at $failedTarget; old installation restoration in progress"
            }
            if ($targetMoved) {
                Assert-PhysicalDirectory $backup "old installation backup"
                if (-not [string]::Equals((Get-Sha256Hex (Join-Path $backup "gogoke.exe")),
                    $oldExeSha256, [StringComparison]::OrdinalIgnoreCase)) {
                    throw "old executable backup changed during rollback"
                }
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
    if ($lifecycleLock) { $lifecycleLock.Dispose() }
    if ($lock) { $lock.Dispose() }
}
