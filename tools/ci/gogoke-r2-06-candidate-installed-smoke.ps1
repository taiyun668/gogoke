# Cloud-only R2-06 candidate installation smoke. Trust values come from the
# calling workflow, not frozen-build.json or other unsigned artifact metadata.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArtifactDirectory,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedSetupSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedInstalledShellSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedNativeHostSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedNodeSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$')][string]$ExpectedVersion,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSourceCommit,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedRunId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ExpectedRunAttempt,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedArtifactId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedSigningRunId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ExpectedSigningRunAttempt
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:stage = 'preflight'
$script:ownedReadyReceipt = $null
$script:ownedSentinel = $null
$script:targetRoot = $null
$script:appDataRoot = $null
$script:evidencePath = $null
$script:installInvoked = $false
$script:uninstallInvoked = $false
$script:instanceId = $null
$script:finalizerReceiptPath = $null
$script:result = [ordered]@{
    schema = 'gogoke.r2-06-candidate-installed-smoke.v1'
    state = 'RUNNING'
    sourceCommit = $ExpectedSourceCommit
    sourceRunId = $ExpectedRunId
    sourceRunAttempt = $ExpectedRunAttempt
    sourceArtifactId = $ExpectedArtifactId
    signingRunId = $ExpectedSigningRunId
    signingRunAttempt = $ExpectedSigningRunAttempt
    version = $ExpectedVersion
    setupSha256 = $ExpectedSetupSha256
    installedShellSha256 = $ExpectedInstalledShellSha256
    nativeHostSha256 = $ExpectedNativeHostSha256
    nodeSha256 = $ExpectedNodeSha256
    stage = $script:stage
}

function Assert-Hash([string]$Path, [string]$Expected, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "$Label missing: $Path" }
    $item = Get-Item -LiteralPath $Path -Force
    if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "$Label is a reparse point: $Path" }
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $Expected) { throw "$Label SHA-256 mismatch: expected $Expected, got $actual" }
}

function Assert-NoReparseAncestors([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    $current = [IO.DirectoryInfo]::new($full)
    while ($null -ne $current) {
        if (Test-Path -LiteralPath $current.FullName) {
            $item = Get-Item -LiteralPath $current.FullName -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Reparse point in smoke path: $($current.FullName)"
            }
        }
        $current = $current.Parent
    }
}

function Get-RegistryKey([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    return Get-ItemProperty -LiteralPath $Path
}

function Get-PhysicalTree([string]$Root) {
    $directories = [Collections.Generic.List[string]]::new()
    $files = [Collections.Generic.List[string]]::new()
    $pending = [Collections.Generic.Stack[string]]::new()
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        $directory = $pending.Pop()
        foreach ($entry in Get-ChildItem -LiteralPath $directory -Force -ErrorAction Stop) {
            if (($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Reparse point remains in candidate install tree: $($entry.FullName)"
            }
            if ($entry.PSIsContainer) {
                $directories.Add($entry.FullName)
                $pending.Push($entry.FullName)
            } else {
                $files.Add($entry.FullName)
            }
        }
    }
    return [pscustomobject]@{ Directories = $directories; Files = $files }
}

function Start-OneShot([string]$FilePath, [string[]]$Arguments, [int]$TimeoutMilliseconds, [bool]$Hidden, [bool]$CaptureStderr = $false) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $Hidden
    $start.RedirectStandardError = $CaptureStderr
    $start.WindowStyle = if ($Hidden) { [Diagnostics.ProcessWindowStyle]::Hidden } else { [Diagnostics.ProcessWindowStyle]::Normal }
    foreach ($argument in $Arguments) { [void]$start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw "Process did not start: $FilePath" }
    if (-not $CaptureStderr) {
        if (-not $process.WaitForExit($TimeoutMilliseconds)) {
            return [pscustomobject]@{ Process = $process; TimedOut = $true; ExitCode = $null; Stderr = '' }
        }
        return [pscustomobject]@{ Process = $process; TimedOut = $false; ExitCode = $process.ExitCode; Stderr = '' }
    }
    $stream = $process.StandardError.BaseStream
    $buffer = [byte[]]::new(4096)
    $saved = [byte[]]::new(2048)
    $savedCount = 0
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMilliseconds)
    $exitedAt = $null
    $read = $stream.ReadAsync($buffer, 0, $buffer.Length)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ($read.Wait(100)) {
            $count = $read.Result
            if ($count -eq 0) { break }
            $take = [Math]::Min($count, $saved.Length - $savedCount)
            if ($take -gt 0) {
                [Array]::Copy($buffer, 0, $saved, $savedCount, $take)
                $savedCount += $take
            }
            $read = $stream.ReadAsync($buffer, 0, $buffer.Length)
        }
        if ($process.HasExited) {
            if ($null -eq $exitedAt) { $exitedAt = [DateTime]::UtcNow }
            if (([DateTime]::UtcNow - $exitedAt).TotalMilliseconds -ge 1500) { break }
        }
    }
    $remaining = [Math]::Max(0, [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds)
    if (-not $process.WaitForExit($remaining)) {
        return [pscustomobject]@{ Process = $process; TimedOut = $true; ExitCode = $null; Stderr = '' }
    }
    $stream.Close()
    $stderr = ''
    if ($savedCount -gt 0) {
        $stderr = [Text.Encoding]::UTF8.GetString($saved, 0, $savedCount)
        $stderr = $stderr -replace '(?i)\bgh[pousr]_[A-Za-z0-9_]+\b', '[REDACTED]'
        $stderr = $stderr -replace '(?i)\bgithub_pat_[A-Za-z0-9_]+\b', '[REDACTED]'
        $stderr = $stderr -replace '(?i)(Bearer\s+)[^\s"'']+', '$1[REDACTED]'
        $published = [Text.Encoding]::UTF8.GetBytes($stderr)
        if ($published.Length -gt 2048) {
            # Leave room for one replacement character if the cut meets a UTF-8 sequence.
            $stderr = [Text.Encoding]::UTF8.GetString($published, 0, 2045)
        }
    }
    return [pscustomobject]@{ Process = $process; TimedOut = $false; ExitCode = $process.ExitCode; Stderr = $stderr }
}

function Measure-Finalizer([Diagnostics.Process]$Process, [string]$ReceiptPath,
    [string]$Root, [Diagnostics.Stopwatch]$Watch) {
    $sample = [ordered]@{
        elapsedSeconds = [Math]::Round($Watch.Elapsed.TotalSeconds, 1)
        receiptState = 'ABSENT'
        processState = 'not_found'
        exitCode = $null
        cpuSeconds = $null
        readBytes = $null
        remainingFiles = $null
    }
    if ($ReceiptPath -and (Test-Path -LiteralPath $ReceiptPath -PathType Leaf)) {
        try { $sample.receiptState = [string](([IO.File]::ReadAllText($ReceiptPath) | ConvertFrom-Json).state) }
        catch [IO.IOException] { $sample.receiptState = 'LOCKED' }
        catch { $sample.receiptState = 'INVALID' }
    }
    if ($null -ne $Process) {
        try {
            $Process.Refresh()
            if ($Process.HasExited) {
                $sample.processState = 'exited'
                $sample.exitCode = $Process.ExitCode
            } else {
                $sample.processState = 'running'
                $sample.cpuSeconds = [Math]::Round($Process.TotalProcessorTime.TotalSeconds, 2)
                try {
                    $live = Get-CimInstance Win32_Process -Filter "ProcessId = $($Process.Id)" -ErrorAction Stop
                    if ($null -ne $live -and $null -ne $live.ReadTransferCount) {
                        $sample.readBytes = [long]$live.ReadTransferCount
                    }
                } catch { }
            }
        } catch { $sample.processState = 'unavailable' }
    }
    if (Test-Path -LiteralPath $Root -PathType Container) {
        try { $sample.remainingFiles = (Get-PhysicalTree $Root).Files.Count }
        catch { $sample.remainingFiles = 'unavailable' }
    } else { $sample.remainingFiles = 0 }
    return $sample
}

function Assert-RegistryRegistration {
    $uninstallPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate'
    $registration = Get-RegistryKey $uninstallPath
    if ($null -eq $registration) { throw 'Candidate HKCU uninstall registration missing' }
    if ([string]$registration.InstallLocation -cne $script:targetRoot -or
        [string]$registration.DisplayVersion -cne $ExpectedVersion -or
        [string]$registration.InstallDomain -cne 'CI_CANDIDATE_RESOURCE' -or
        [string]$registration.InstallInstanceId -cnotmatch '^.{16,}$' -or
        [string]$registration.UninstallString -cne ('"' + (Join-Path $script:targetRoot 'gogoke.exe') + '" --uninstall') -or
        [string]$registration.QuietUninstallString -cne ('"' + (Join-Path $script:targetRoot 'gogoke.exe') + '" --uninstall --quiet')) {
        throw 'Candidate HKCU registration fields differ from the exact installed instance'
    }
    if (Test-Path -LiteralPath 'HKCU:\Software\gogoke\gogoke-candidate') {
        throw 'Candidate installer wrote an unused product-root registry key'
    }
    $script:instanceId = [string]$registration.InstallInstanceId
}

try {
    $script:stage = 'cloud-identity'
    $script:result.stage = $script:stage
    if (-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or
        $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
        $env:GITHUB_SERVER_URL -cne 'https://github.com' -or
        $env:GITHUB_REPOSITORY -cne 'taiyun668/gogoke') {
        throw 'Candidate installed smoke requires GitHub-hosted Windows Actions'
    }
    if ($env:GITHUB_RUN_ID -cne [string]$ExpectedSigningRunId -or
        $env:GITHUB_RUN_ATTEMPT -cne [string]$ExpectedSigningRunAttempt) {
        throw 'Expected signing run identity differs from the current GitHub Actions run'
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        if ($identity.IsSystem -or
            $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Candidate installed product smoke must run in a Medium process'
        }
    } finally { $identity.Dispose() }
    foreach ($secretName in @('GH_TOKEN', 'GITHUB_TOKEN', 'GOGOKE_CANDIDATE_P256_KEY',
                              'ACTIONS_RUNTIME_TOKEN', 'ACTIONS_ID_TOKEN_REQUEST_TOKEN')) {
        if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($secretName))) {
            throw "Credential environment variable is visible to candidate product: $secretName"
        }
    }
    if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or
        -not (Test-Path -LiteralPath $env:RUNNER_TEMP -PathType Container)) {
        throw 'RUNNER_TEMP is unavailable'
    }

    $script:stage = 'artifact-preflight'
    $script:result.stage = $script:stage
    $artifactRoot = (Resolve-Path -LiteralPath $ArtifactDirectory).Path
    $setupName = "gogoke-$ExpectedVersion-windows-x64-unsigned-setup.exe"
    $setup = Join-Path $artifactRoot $setupName
    $required = @(
        $setup,
        (Join-Path $artifactRoot 'gogoke-resources.windows.zip'),
        (Join-Path $artifactRoot 'resource-index.json'),
        (Join-Path $artifactRoot 'CANDIDATE-RESOURCES.windows'),
        (Join-Path $artifactRoot 'CANDIDATE-RESOURCES.windows.sig')
    )
    foreach ($path in $required) {
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Candidate sibling file missing: $path" }
        $artifactFile = Get-Item -LiteralPath $path -Force
        if (($artifactFile.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Candidate sibling is a reparse point: $path" }
    }
    foreach ($formalName in @('SHA256SUMS.windows', 'SHA256SUMS.windows.sig')) {
        if (Test-Path -LiteralPath (Join-Path $artifactRoot $formalName)) {
            throw "Candidate artifact contains a formal release sidecar: $formalName"
        }
    }
    Assert-Hash $setup $ExpectedSetupSha256 'Setup artifact'
    $manifestText = [IO.File]::ReadAllText((Join-Path $artifactRoot 'CANDIDATE-RESOURCES.windows'), [Text.Encoding]::ASCII)
    foreach ($line in @(
        "# gogoke-Candidate-Purpose: CI_CANDIDATE_RESOURCE",
        '# gogoke-Test-Only: true',
        '# gogoke-Repository: taiyun668/gogoke',
        "# gogoke-Source-Commit: $ExpectedSourceCommit",
        "# gogoke-Run-Id: $ExpectedRunId",
        "# gogoke-Run-Attempt: $ExpectedRunAttempt",
        "# gogoke-Artifact-Id: $ExpectedArtifactId",
        "# gogoke-Version: $ExpectedVersion"
    )) {
        if (-not ($manifestText -split "`n" | Where-Object { $_.TrimEnd("`r") -ceq $line })) {
            throw "Signed candidate identity does not contain expected field: $line"
        }
    }
    $index = Get-Content -LiteralPath (Join-Path $artifactRoot 'resource-index.json') -Raw | ConvertFrom-Json
    if ($index.schema -cne 'gogoke.resource-index.v1' -or
        $index.version -cne $ExpectedVersion -or
        $index.sourceCommit -cne $ExpectedSourceCommit) {
        throw 'Resource index identity disagrees with trusted expected inputs'
    }

    $script:stage = 'fresh-target-and-registration'
    $script:result.stage = $script:stage
    $runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
    $script:targetRoot = [IO.Path]::GetFullPath((Join-Path $runnerTemp "gogoke-r2-06-candidate-$ExpectedSigningRunId-$ExpectedSigningRunAttempt"))
    if (-not $script:targetRoot.StartsWith($runnerTemp + '\', [StringComparison]::OrdinalIgnoreCase) -or
        (Test-Path -LiteralPath $script:targetRoot)) {
        throw 'Candidate install target is not a fresh child of RUNNER_TEMP'
    }
    Assert-NoReparseAncestors $runnerTemp
    foreach ($key in @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate',
        'HKCU:\Software\gogoke\gogoke-candidate'
    )) {
        if (Test-Path -LiteralPath $key) { throw "Candidate registry key already exists: $key" }
    }
    $appDataBase = [Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)
    $script:appDataRoot = Join-Path $appDataBase 'app.gogoke.desktop.candidate'
    if (Test-Path -LiteralPath $script:appDataRoot) {
        throw 'Candidate AppData root already exists; preserving preexisting user data'
    }
    Assert-NoReparseAncestors $appDataBase

    $script:stage = 'install-once'
    $script:result.stage = $script:stage
    $script:installInvoked = $true
    $install = Start-OneShot $setup @('/S', "/D=$script:targetRoot") 300000 $true
    if ($install.TimedOut) { throw "NSIS install exceeded bound; retain target: $script:targetRoot" }
    if ($install.ExitCode -ne 0) { throw "NSIS install failed with exit code $($install.ExitCode); retain target: $script:targetRoot" }
    Assert-RegistryRegistration
    $uninstaller = Join-Path $script:targetRoot 'uninstall.exe'
    if (Test-Path -LiteralPath $uninstaller) { throw 'Candidate install unexpectedly created uninstall.exe' }

    $script:stage = 'installed-byte-identity'
    $script:result.stage = $script:stage
    Assert-Hash (Join-Path $script:targetRoot 'gogoke.exe') $ExpectedInstalledShellSha256 'Installed shell'
    Assert-Hash (Join-Path $script:targetRoot 'gogoke-native-host.exe') $ExpectedNativeHostSha256 'Installed native host'
    Assert-Hash (Join-Path $script:targetRoot 'gogoke-service\runtime\node.exe') $ExpectedNodeSha256 'Installed Node runtime'

    $script:stage = 'sentinel-and-product-readiness'
    $script:result.stage = $script:stage
    New-Item -ItemType Directory -Path $script:appDataRoot | Out-Null
    $sentinelName = "r2-06-candidate-$ExpectedRunId-$ExpectedRunAttempt.sentinel"
    $script:ownedSentinel = Join-Path $script:appDataRoot $sentinelName
    $sentinelBytes = [Text.Encoding]::UTF8.GetBytes("gogoke-r2-06-candidate-appdata-sentinel`n$ExpectedSourceCommit`n$ExpectedRunId`n$ExpectedRunAttempt`n")
    [IO.File]::WriteAllBytes($script:ownedSentinel, $sentinelBytes)
    $readyName = 'gogoke-update-r2-06-' + [Guid]::NewGuid().ToString('N') + '.ready'
    # Rust's Windows temp_dir uses GetTempPath2W, which preserves the raw TMP
    # spelling. .NET GetTempPath expands 8.3 aliases on hosted Windows runners.
    $rawTemp = [string]$env:TMP
    if ([string]::IsNullOrWhiteSpace($rawTemp) -or
        -not [IO.Path]::IsPathFullyQualified($rawTemp) -or
        -not (Test-Path -LiteralPath $rawTemp -PathType Container)) {
        throw 'Candidate TMP is not an existing absolute directory'
    }
    Assert-NoReparseAncestors $rawTemp
    $script:ownedReadyReceipt = [IO.Path]::Combine($rawTemp, $readyName)
    if (Test-Path -LiteralPath $script:ownedReadyReceipt) { throw 'Readiness receipt path is not fresh' }
    $productExe = Join-Path $script:targetRoot 'gogoke.exe'
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $productExe
    $start.UseShellExecute = $false
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Normal
    [void]$start.ArgumentList.Add("--gogoke-update-ready=$script:ownedReadyReceipt")
    $product = [Diagnostics.Process]::new()
    $product.StartInfo = $start
    if (-not $product.Start()) { throw 'Installed Gogoke did not start' }
    $readyDeadline = [DateTime]::UtcNow.AddSeconds(45)
    while ([DateTime]::UtcNow -lt $readyDeadline) {
        if (Test-Path -LiteralPath $script:ownedReadyReceipt -PathType Leaf) { break }
        if ($product.HasExited) { throw "Installed product exited before readiness: $($product.ExitCode)" }
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $script:ownedReadyReceipt -PathType Leaf)) {
        throw "Installed product produced no version-bound real-bootstrap readiness receipt: $script:ownedReadyReceipt"
    }
    $ready = Get-Content -LiteralPath $script:ownedReadyReceipt -Raw | ConvertFrom-Json
    $indexHash = (Get-FileHash -LiteralPath (Join-Path $script:targetRoot 'resource-index.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($ready.version -cne $ExpectedVersion -or
        $ready.generationId -cne $index.generationId -or
        $ready.setId -cne $indexHash) {
        throw 'Installed product readiness receipt identity mismatch'
    }
    if (-not ($manifestText -split "`n" | Where-Object { $_.TrimEnd("`r") -ceq "$indexHash  resource-index.json" })) {
        throw 'Installed resource index is not the exact resource index bound by the signed candidate sidecar'
    }
    $script:result.readiness = [ordered]@{ version = $ready.version; generationId = $ready.generationId; setId = $ready.setId }
    [void]$product.CloseMainWindow()
    if (-not $product.WaitForExit(15000)) {
        throw "Installed UI did not exit after close request; preserve candidate root: $script:targetRoot"
    }

    $script:stage = 'uninstall-once-and-finalizer-receipt'
    $script:result.stage = $script:stage
    Assert-RegistryRegistration
    $instanceTagBytes = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($script:instanceId))
    $instanceTag = ([Convert]::ToHexString($instanceTagBytes).ToLowerInvariant()).Substring(0, 16)
    $receiptPrefix = "gogoke-uninstall-$instanceTag-"
    $beforeReceipts = @{}
    Get-ChildItem -LiteralPath $runnerTemp -Filter "$receiptPrefix*.json" -File -ErrorAction SilentlyContinue | ForEach-Object { $beforeReceipts[$_.Name] = $true }
    $script:uninstallInvoked = $true
    $uninstall = Start-OneShot $productExe @('--uninstall', '--quiet') 30000 $true $true
    if ($uninstall.TimedOut) { throw "Installed uninstall exceeded parent bound; retain candidate state under: $script:targetRoot" }
    if ($uninstall.ExitCode -ne 0) {
        $script:result.uninstallExitCode = $uninstall.ExitCode
        $script:result.uninstallStderr = $uninstall.Stderr
        throw "Installed uninstall failed with exit code $($uninstall.ExitCode); stderr: $($uninstall.Stderr); retain candidate state under: $script:targetRoot"
    }
    $finalizerWatch = [Diagnostics.Stopwatch]::StartNew()
    $finalizerDeadline = [DateTime]::UtcNow.AddSeconds(125)
    $finalizerReceipt = $null
    while ([DateTime]::UtcNow -lt $finalizerDeadline) {
        $newReceipts = @(Get-ChildItem -LiteralPath $runnerTemp -Filter "$receiptPrefix*.json" -File -ErrorAction SilentlyContinue | Where-Object { -not $beforeReceipts.ContainsKey($_.Name) })
        if ($newReceipts.Count -gt 1) { throw 'More than one new finalizer receipt appeared for the candidate instance' }
        if ($newReceipts.Count -eq 1) {
            $script:finalizerReceiptPath = $newReceipts[0].FullName
            try {
                # The finalizer reserves this file with FileShare.None before
                # publishing PENDING and only releases it after DELETED/FAILED.
                $candidateReceipt = [IO.File]::ReadAllText($newReceipts[0].FullName) | ConvertFrom-Json
            } catch [IO.IOException] {
                Start-Sleep -Milliseconds 250
                continue
            }
            if ($candidateReceipt.state -ceq 'FAILED') { throw "Uninstall finalizer failed: $([string]$candidateReceipt.detail). Receipt: $($newReceipts[0].FullName)" }
            if ($candidateReceipt.state -ceq 'DELETED' -and [DateTime]::UtcNow -lt $finalizerDeadline) {
                $finalizerReceipt = $newReceipts[0]
                break
            }
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $finalizerReceipt) {
        $finalizerProcess = $null
        $script:result.finalizerChildLookup = 'unavailable'
        try {
            $children = @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($uninstall.Process.Id)" -ErrorAction Stop |
                Where-Object { $_.Name -ieq 'powershell.exe' })
            if ($children.Count -eq 1) {
                $finalizerProcess = [Diagnostics.Process]::GetProcessById([int]$children[0].ProcessId)
                $script:result.finalizerChildLookup = 'found'
                $script:result.finalizerPid = $finalizerProcess.Id
            } elseif ($children.Count -eq 0) {
                $script:result.finalizerChildLookup = 'none'
            } else {
                $script:result.finalizerChildLookup = 'ambiguous'
            }
        } catch { }
        $first = Measure-Finalizer $finalizerProcess $script:finalizerReceiptPath $script:targetRoot $finalizerWatch
        Start-Sleep -Seconds 5
        $second = Measure-Finalizer $finalizerProcess $script:finalizerReceiptPath $script:targetRoot $finalizerWatch
        $script:result.finalizerDiagnostics = @($first, $second)
        throw "No bounded DELETED finalizer receipt; preserve state under: $script:targetRoot"
    }
    $receiptRecord = Get-Content -LiteralPath $finalizerReceipt.FullName -Raw | ConvertFrom-Json
    if ($receiptRecord.schema -cne 'gogoke.uninstall-result.v1' -or
        $receiptRecord.state -cne 'DELETED' -or
        $receiptRecord.domain -cne 'CI_CANDIDATE_RESOURCE') {
        throw "Finalizer receipt identity mismatch: $($finalizerReceipt.FullName)"
    }

    $script:stage = 'post-uninstall-preservation-check'
    $script:result.stage = $script:stage
    foreach ($key in @(
        'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate',
        'HKCU:\Software\gogoke\gogoke-candidate'
    )) {
        if (Test-Path -LiteralPath $key) { throw "Candidate registry key remains after DELETED receipt: $key" }
    }
    if (-not (Test-Path -LiteralPath $script:ownedSentinel -PathType Leaf) -or
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($script:ownedSentinel)) -cne [Convert]::ToBase64String($sentinelBytes)) {
        throw "Candidate AppData sentinel changed or disappeared: $script:ownedSentinel"
    }
    if (Test-Path -LiteralPath $script:targetRoot) {
        $tree = Get-PhysicalTree $script:targetRoot
        if ($tree.Files.Count -ne 0) { throw "Owned candidate install files remain; preserve root: $script:targetRoot" }
        foreach ($directory in @($tree.Directories | Sort-Object Length -Descending)) {
            Remove-Item -LiteralPath $directory -Force
        }
        Remove-Item -LiteralPath $script:targetRoot -Force
    }
    $script:finalizerReceiptPath = $finalizerReceipt.FullName
    $script:result.finalizerReceipt = $finalizerReceipt.FullName
    $script:result.sentinelSha256 = (Get-FileHash -LiteralPath $script:ownedSentinel -Algorithm SHA256).Hash.ToLowerInvariant()
    # Remove only the unique receipt and sentinel this invocation proved it created.
    Remove-Item -LiteralPath $script:ownedReadyReceipt -Force
    Remove-Item -LiteralPath $script:ownedSentinel -Force
    $script:result.state = 'PASS'
    $script:result.stage = 'complete'
    $script:evidencePath = Join-Path $env:RUNNER_TEMP ("gogoke-r2-06-candidate-smoke-$ExpectedSigningRunId-$ExpectedSigningRunAttempt.json")
    $script:result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $script:evidencePath -Encoding utf8
    Write-Output ($script:result | ConvertTo-Json -Depth 6 -Compress)
} catch {
    $message = [string]$_.Exception.Message
    if ($message.Length -gt 320) { $message = $message.Substring(0, 320) }
    $script:result.state = 'FAIL'
    $script:result.stage = $script:stage
    $script:result.error = $message
    $script:result.installInvoked = $script:installInvoked
    $script:result.uninstallInvoked = $script:uninstallInvoked
    $script:result.retainedCandidateRoot = if ($script:installInvoked) { $script:targetRoot } else { $null }
    $script:result.retainedAppDataSentinel = if ($script:ownedSentinel -and (Test-Path -LiteralPath $script:ownedSentinel)) { $script:ownedSentinel } else { $null }
    $script:result.readinessReceiptPath = if ($script:ownedReadyReceipt) { $script:ownedReadyReceipt } else { $null }
    $script:result.finalizerReceiptPath = $script:finalizerReceiptPath
    if ($env:RUNNER_TEMP -and (Test-Path -LiteralPath $env:RUNNER_TEMP -PathType Container)) {
        $script:evidencePath = Join-Path $env:RUNNER_TEMP ("gogoke-r2-06-candidate-smoke-$ExpectedSigningRunId-$ExpectedSigningRunAttempt.json")
        try { $script:result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $script:evidencePath -Encoding utf8 }
        catch { $script:evidencePath = $null }
    }
    Write-Error (($script:result | ConvertTo-Json -Depth 6 -Compress) + $(if ($script:evidencePath) { " Evidence: $script:evidencePath" } else { '' }))
    exit 1
}
