[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$Version
)

$ErrorActionPreference = "Stop"
$repo = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$installerPath = (Resolve-Path -LiteralPath $Installer).Path
$root = Join-Path $env:TEMP ("gogoke-update-smoke-" + [Guid]::NewGuid().ToString("N"))
$target = Join-Path $root "gogoke"
$coordinator = Join-Path $repo "apps\desktop\src-tauri\update\gogoke-update-coordinator.ps1"
$failure = Join-Path $root "failure.log"
$lock = Join-Path $root "apply.lock"
$ready = Join-Path $env:TEMP ("gogoke-update-" + [Guid]::NewGuid().ToString("N") + ".ready")
$installerHash = (Get-FileHash -LiteralPath $installerPath -Algorithm SHA256).Hash
$powershell = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"

function Start-ShortParent {
    Start-Process -FilePath $powershell -ArgumentList @(
        "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden",
        "-Command", "Start-Sleep -Milliseconds 400"
    ) -WindowStyle Hidden -PassThru
}

function Invoke-Coordinator([string]$expected, [switch]$NoRestart) {
    $parent = Start-ShortParent
    $parts = @(
        "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", $coordinator,
        "-Installer", $installerPath, "-ParentPid", [string]$parent.Id,
        "-CurrentExe", (Join-Path $target "gogoke.exe"),
        "-TargetDir", $target, "-ReadyFile", $ready,
        "-ExpectedVersion", $expected, "-ExpectedSha256", $installerHash,
        "-LockFile", $lock,
        "-FailureLog", $failure, "-Silent"
    )
    if ($NoRestart) { $parts += "-NoRestartOnRollback" }
    & $powershell @parts | ForEach-Object { Write-Host $_ }
    $exitCode = $LASTEXITCODE
    [pscustomobject]@{ ExitCode = $exitCode }
}

function Stop-TargetProcess {
    $targetExe = Join-Path $target "gogoke.exe"
    Get-CimInstance Win32_Process -Filter "Name='gogoke.exe'" -ErrorAction SilentlyContinue |
        Where-Object { $_.ExecutablePath -eq $targetExe } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
}

try {
    New-Item -ItemType Directory -Force -Path $root | Out-Null
    $success = Invoke-Coordinator $Version
    if ($success.ExitCode -ne 0) { throw "successful update smoke exited $($success.ExitCode)" }
    $targetExe = Join-Path $target "gogoke.exe"
    if (-not (Test-Path -LiteralPath $targetExe -PathType Leaf)) { throw "updated executable is missing" }
    if (Test-Path -LiteralPath $ready) { throw "coordinator did not consume the readiness receipt" }
    Stop-TargetProcess

    $marker = Join-Path $target "rollback-marker.txt"
    [IO.File]::WriteAllText($marker, "old-version-owned-file", [Text.Encoding]::ASCII)
    $rollback = Invoke-Coordinator "9.9.9" -NoRestart
    if ($rollback.ExitCode -eq 0) { throw "fault-injected update unexpectedly succeeded" }
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) { throw "rollback did not restore the old target" }
    if (([IO.File]::ReadAllText($marker)).Trim() -ne "old-version-owned-file") { throw "rollback marker changed" }
    if (-not (Test-Path -LiteralPath $failure -PathType Leaf)) { throw "rollback did not retain a failure log" }
    if (Test-Path -LiteralPath "$target.update-backup") { throw "rollback backup was not returned to the target" }
    Write-Output "PASS gogoke update handoff, readiness, clean replacement, and rollback"
} catch {
    if (Test-Path -LiteralPath $failure -PathType Leaf) {
        [Console]::Error.WriteLine([IO.File]::ReadAllText($failure))
    }
    throw
} finally {
    Stop-TargetProcess
    $uninstaller = Join-Path $target "uninstall.exe"
    if (Test-Path -LiteralPath $uninstaller -PathType Leaf) {
        Write-Output "gogoke update smoke phase=cleanup-uninstall"
        $cleanup = Start-Process -FilePath $uninstaller -ArgumentList "/S" -Wait -PassThru
        if ($cleanup.ExitCode -ne 0) {
            [Console]::Error.WriteLine("gogoke update smoke uninstaller exited $($cleanup.ExitCode)")
        }
    }
    if (Test-Path -LiteralPath $ready) { Remove-Item -LiteralPath $ready -Force }
    $resolved = [IO.Path]::GetFullPath($root)
    $tempPrefix = [IO.Path]::GetFullPath($env:TEMP).TrimEnd('\') + '\'
    if ($resolved.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $resolved)) {
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}

# The rollback probe must return non-zero, but that expected child status must
# not become the status of a smoke run that reached every PASS assertion.
$global:LASTEXITCODE = 0
