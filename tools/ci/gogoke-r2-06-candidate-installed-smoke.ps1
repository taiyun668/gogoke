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
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ExpectedSigningRunAttempt,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedSmokeRunId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ExpectedSmokeRunAttempt
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
$script:serviceEvidencePath = $null
$script:installDiagnosticPath = $null
$script:result = [ordered]@{
    schema = 'gogoke.r2-06-candidate-installed-smoke.v1'
    state = 'RUNNING'
    sourceCommit = $ExpectedSourceCommit
    sourceRunId = $ExpectedRunId
    sourceRunAttempt = $ExpectedRunAttempt
    sourceArtifactId = $ExpectedArtifactId
    signingRunId = $ExpectedSigningRunId
    signingRunAttempt = $ExpectedSigningRunAttempt
    smokeRunId = $ExpectedSmokeRunId
    smokeRunAttempt = $ExpectedSmokeRunAttempt
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

function Start-OneShot([string]$FilePath, [string[]]$Arguments, [int]$TimeoutMilliseconds, [bool]$Hidden) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $FilePath
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $Hidden
    $start.WindowStyle = if ($Hidden) { [Diagnostics.ProcessWindowStyle]::Hidden } else { [Diagnostics.ProcessWindowStyle]::Normal }
    foreach ($argument in $Arguments) { [void]$start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw "Process did not start: $FilePath" }
    if (-not $process.WaitForExit($TimeoutMilliseconds)) {
        return [pscustomobject]@{ Process = $process; TimedOut = $true; ExitCode = $null }
    }
    return [pscustomobject]@{ Process = $process; TimedOut = $false; ExitCode = $process.ExitCode }
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
    if ($env:GITHUB_REF -cne 'refs/heads/gpt/s1-r4-r2-execution-r1' -or
        $env:GITHUB_RUN_ID -cne [string]$ExpectedSmokeRunId -or
        $env:GITHUB_RUN_ATTEMPT -cne [string]$ExpectedSmokeRunAttempt) {
        throw 'Expected execution-branch smoke identity differs from the current GitHub Actions run'
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
    $script:targetRoot = [IO.Path]::GetFullPath((Join-Path $runnerTemp "gogoke-r2-06-candidate-$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt"))
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
    if ([string]::IsNullOrWhiteSpace($env:TMP) -or
        -not [IO.Path]::IsPathFullyQualified($env:TMP)) {
        throw 'Candidate TMP is unavailable for the first installer error receipt'
    }
    $script:installDiagnosticPath = Join-Path $env:TMP "gogoke-install-error-$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt.txt"
    if (Test-Path -LiteralPath $script:installDiagnosticPath) {
        throw 'First installer error receipt path is not fresh'
    }
    # Hosted Windows copied only about half the installed files at 300 seconds
    # in the measured run. Keep a single installer invocation and bound it by
    # the candidate job budget rather than treating slow copying as a failure.
    $install = Start-OneShot $setup @('/S', "/D=$script:targetRoot") 900000 $true
    if ($install.TimedOut) {
        # Diagnostic only: retain the 900-second acceptance bound and
        # observe the same installer process once more without restarting it.
        $script:result.installExceededBound = $true
        try {
            $atBound = Get-PhysicalTree $script:targetRoot
            $script:result.installedFilesAtBound = $atBound.Files.Count
            $script:result.installedDirectoriesAtBound = $atBound.Directories.Count
        } catch {
            $script:result.installInventoryAtBoundError = [string]$_.Exception.Message
        }
        $script:result.installerExitedWithinAdditionalTenMinutes = $install.Process.WaitForExit(600000)
        if ($script:result.installerExitedWithinAdditionalTenMinutes) {
            $script:result.installerExitCodeAfterBound = $install.Process.ExitCode
            try {
                $afterBound = Get-PhysicalTree $script:targetRoot
                $script:result.installedFilesAfterExit = $afterBound.Files.Count
                $script:result.installedDirectoriesAfterExit = $afterBound.Directories.Count
            } catch {
                $script:result.installInventoryAfterExitError = [string]$_.Exception.Message
            }
            $generations = Join-Path $script:targetRoot 'gogoke-service/generations'
            if (Test-Path -LiteralPath $generations -PathType Container) {
                try {
                    $generationEntries = @(Get-ChildItem -LiteralPath $generations -Force)
                    $script:result.generationEntriesAfterExit = @(
                        $generationEntries | Select-Object -First 8 Name, PSIsContainer
                    )
                    foreach ($entry in $generationEntries) {
                        if ($entry.PSIsContainer -and $entry.Name.StartsWith('.stage-', [StringComparison]::Ordinal)) {
                            $stageTree = Get-PhysicalTree $entry.FullName
                            $script:result.generationStageFilesAfterExit = $stageTree.Files.Count
                            $script:result.generationExpectedFiles = @($index.files).Count
                            $script:result.firstMissingGenerationFile = @(
                                $index.files | Where-Object {
                                    -not (Test-Path -LiteralPath (Join-Path $entry.FullName $_.path) -PathType Leaf)
                                } | Select-Object -First 1 -ExpandProperty path
                            ) | Select-Object -First 1
                            break
                        }
                    }
                } catch {
                    $script:result.generationInventoryError = [string]$_.Exception.Message
                }
            }
            if ($install.Process.ExitCode -ne 0) {
                # A diagnostic replay observes the error string after the
                # first installer process has exited. It is not first-run
                # evidence or a successful installation retry.
                $replay = [Diagnostics.Process]::new()
                try {
                    $replay.StartInfo.FileName = Join-Path $script:targetRoot 'gogoke.exe'
                    $replay.StartInfo.UseShellExecute = $false
                    $replay.StartInfo.CreateNoWindow = $true
                    $replay.StartInfo.RedirectStandardError = $true
                    [void]$replay.StartInfo.ArgumentList.Add("--gogoke-install-resources=$artifactRoot")
                    if (-not $replay.Start()) { throw 'Installed resource diagnostic did not start' }
                    if (-not $replay.WaitForExit(180000)) {
                        $replay.Kill($true)
                        if (-not $replay.WaitForExit(10000)) {
                            throw 'Installed resource diagnostic exit is unconfirmed'
                        }
                        $script:result.resourceDiagnosticTimedOut = $true
                    } else {
                        $script:result.resourceDiagnosticExitCode = $replay.ExitCode
                    }
                    $diagnosticError = $replay.StandardError.ReadToEnd().Trim()
                    if ($diagnosticError.Length -gt 512) {
                        $diagnosticError = $diagnosticError.Substring(0, 512)
                    }
                    $script:result.resourceDiagnosticStderr = $diagnosticError
                } catch {
                    $script:result.resourceDiagnosticError = [string]$_.Exception.Message
                } finally {
                    $replay.Dispose()
                }
            }
        }
        throw "NSIS install exceeded 900-second bound; diagnostic retained target: $script:targetRoot"
    }
    if ($install.ExitCode -ne 0) {
        $script:result.installerExitCodeWithinBound = $install.ExitCode
        $generations = Join-Path $script:targetRoot 'gogoke-service/generations'
        if (Test-Path -LiteralPath $generations -PathType Container) {
            try {
                $generationEntries = @(Get-ChildItem -LiteralPath $generations -Force)
                $script:result.generationEntriesAfterExit = @(
                    $generationEntries | Select-Object -First 8 Name, PSIsContainer
                )
                foreach ($entry in $generationEntries) {
                    if ($entry.PSIsContainer -and $entry.Name.StartsWith('.stage-', [StringComparison]::Ordinal)) {
                        $stageTree = Get-PhysicalTree $entry.FullName
                        $script:result.generationStageFilesAfterExit = $stageTree.Files.Count
                        $script:result.generationExpectedFiles = @($index.files).Count
                        $script:result.firstMissingGenerationFile = @(
                            $index.files | Where-Object {
                                -not (Test-Path -LiteralPath (Join-Path $entry.FullName $_.path) -PathType Leaf)
                            } | Select-Object -First 1 -ExpandProperty path
                        ) | Select-Object -First 1
                        break
                    }
                }
            } catch {
                $script:result.generationInventoryError = [string]$_.Exception.Message
            }
        }
        throw "NSIS install failed with exit code $($install.ExitCode); retain target: $script:targetRoot"
    }
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

    $script:stage = 'installed-product-service-positive-and-poison-negative'
    $script:result.stage = $script:stage
    if ($index.generationId -cnotmatch '^[0-9a-f]{64}$') {
        throw 'Signed candidate generation identity is invalid'
    }
    $nodeCommand = Join-Path $script:targetRoot 'gogoke-service/runtime/node.exe'
    $serviceTool = Join-Path $PSScriptRoot 'gogoke-package-service.mjs'
    if (-not (Test-Path -LiteralPath $serviceTool -PathType Leaf)) {
        throw 'Candidate installed service smoke tool is missing'
    }
    $script:serviceEvidencePath = Join-Path $runnerTemp "gogoke-r2-06-service-$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt.json"
    if (Test-Path -LiteralPath $script:serviceEvidencePath) {
        throw 'Candidate installed service evidence path is not fresh'
    }
    $serviceSmoke = Start-OneShot $nodeCommand @(
        $serviceTool, 'candidate-installed-service', $script:targetRoot,
        [string]$index.generationId, $ExpectedSourceCommit, $ExpectedVersion, $script:serviceEvidencePath
    ) 240000 $true
    if ($serviceSmoke.TimedOut) {
        $kill = Start-OneShot 'taskkill.exe' @('/PID', [string]$serviceSmoke.Process.Id, '/T', '/F') 15000 $true
        if ($kill.TimedOut -or $kill.ExitCode -ne 0 -or -not $serviceSmoke.Process.WaitForExit(5000)) {
            throw 'Candidate service smoke timed out and process-tree settlement is unconfirmed; retain install'
        }
        throw 'Candidate service smoke exceeded its bound; retain install'
    }
    if ($serviceSmoke.ExitCode -ne 0 -or
        -not (Test-Path -LiteralPath $script:serviceEvidencePath -PathType Leaf)) {
        throw "Candidate installed service smoke failed with exit code $($serviceSmoke.ExitCode); retain install"
    }
    $serviceEvidence = Get-Content -LiteralPath $script:serviceEvidencePath -Raw | ConvertFrom-Json
    if ($serviceEvidence.schema -cne 'gogoke.r2-06-candidate-service-smoke.v1' -or
        $serviceEvidence.state -cne 'PASS' -or
        $serviceEvidence.platform -cne 'WINDOWS_CLOUD_NOT_OWNER_WIN11' -or
        $serviceEvidence.sourceCommit -cne $ExpectedSourceCommit -or
        $serviceEvidence.generationId -cne $index.generationId -or
        $serviceEvidence.smokeRunId -cne [string]$ExpectedSmokeRunId -or
        $serviceEvidence.smokeRunAttempt -cne [string]$ExpectedSmokeRunAttempt -or
        $serviceEvidence.installedShellSha256 -cne $ExpectedInstalledShellSha256 -or
        $serviceEvidence.positive.invocation -cne 'gogoke_r2_goal_probe' -or
        $serviceEvidence.positive.state -cne 'VALIDATED_TEST_RESULT_NOT_ADOPTED' -or
        $serviceEvidence.positive.nativeController -cne 'ADMITTED' -or
        $serviceEvidence.positive.readinessSetId -cne $indexHash -or
        $serviceEvidence.positive.acceptance -cne 'TEST_FIXTURE_NOT_ADOPTED' -or
        $serviceEvidence.positive.adoption -cne $false -or
        $serviceEvidence.negative.invocation -cne 'gogoke_r2_goal_probe' -or
        $serviceEvidence.negative.rejection -cne 'GOGOKE_PRODUCT_SERVICE_FAILED:78' -or
        $serviceEvidence.negative.poisonPath -cne 'gogoke-service/generations/node_modules/@ff-labs/fff-node' -or
        $serviceEvidence.negative.poisonExecuted -cne $false -or
        $serviceEvidence.negative.poisonRemoved -cne $true -or
        $serviceEvidence.negative.readinessReceipt -cne 'ABSENT' -or
        $serviceEvidence.negative.adoption -cne $false) {
        throw 'Candidate installed service smoke evidence did not prove both product invocations'
    }
    $script:result.productService = $serviceEvidence

    $script:stage = 'uninstall-once-and-finalizer-receipt'
    $script:result.stage = $script:stage
    Assert-RegistryRegistration
    $instanceTagBytes = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($script:instanceId))
    $instanceTag = ([Convert]::ToHexString($instanceTagBytes).ToLowerInvariant()).Substring(0, 16)
    $receiptPrefix = "gogoke-uninstall-$instanceTag-"
    $beforeReceipts = @{}
    Get-ChildItem -LiteralPath $runnerTemp -Filter "$receiptPrefix*.json" -File -ErrorAction SilentlyContinue | ForEach-Object { $beforeReceipts[$_.Name] = $true }
    $script:uninstallInvoked = $true
    $uninstall = Start-OneShot $productExe @('--uninstall', '--quiet') 30000 $true
    if ($uninstall.TimedOut) { throw "Installed uninstall exceeded parent bound; retain candidate state under: $script:targetRoot" }
    if ($uninstall.ExitCode -ne 0) {
        $script:result.uninstallExitCode = $uninstall.ExitCode
        throw "Installed uninstall failed with exit code $($uninstall.ExitCode); retain candidate state under: $script:targetRoot"
    }
    $finalizerWatch = [Diagnostics.Stopwatch]::StartNew()
    # The signed candidate has about 13,000 owned files. Run 36173004213
    # observed active deletion at 125 seconds (10,338 files still present),
    # then another 841 files removed over the next 6.3 seconds.
    $finalizerDeadline = [DateTime]::UtcNow.AddSeconds(300)
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
    if ($null -eq $finalizerReceipt) { throw "No bounded DELETED finalizer receipt; preserve state under: $script:targetRoot" }
    $script:result.finalizerElapsedSeconds = [Math]::Round($finalizerWatch.Elapsed.TotalSeconds, 1)
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
        if ($tree.Files.Count -ne 0) { throw "Candidate install files remain after DELETED receipt; preserve root: $script:targetRoot" }
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
    $script:evidencePath = Join-Path $env:RUNNER_TEMP ("gogoke-r2-06-candidate-smoke-$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt.json")
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
    $script:result.productServiceEvidencePath = $script:serviceEvidencePath
    if ($script:installDiagnosticPath -and (Test-Path -LiteralPath $script:installDiagnosticPath -PathType Leaf)) {
        $diagnostic = Get-Item -LiteralPath $script:installDiagnosticPath -Force
        if (($diagnostic.Attributes -band [IO.FileAttributes]::ReparsePoint) -eq 0 -and
            $diagnostic.Length -le 256) {
            $script:result.firstInstallError = (Get-Content -LiteralPath $script:installDiagnosticPath -Raw).Trim()
        }
    }
    if ($script:stage -ceq 'install-once' -and $script:targetRoot -and
        $env:RUNNER_TEMP -and
        [IO.Path]::GetFullPath($script:targetRoot).StartsWith(
            [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\') + '\',
            [StringComparison]::OrdinalIgnoreCase) -and
        (Test-Path -LiteralPath $script:targetRoot -PathType Container)) {
        $names = @(
            'gogoke.exe', 'gogoke-native-host.exe', 'gogoke-resources.windows.zip',
            'resource-index.json', 'CANDIDATE-RESOURCES.windows',
            'CANDIDATE-RESOURCES.windows.sig', 'gogoke-install-receipt.ini',
            'gogoke-current-resource-set', 'gogoke-service', 'gogoke-resource-sets'
        )
        $script:result.installFailureInventory = @($names | ForEach-Object {
            $item = Get-Item -LiteralPath (Join-Path $script:targetRoot $_) -Force -ErrorAction SilentlyContinue
            if ($null -eq $item) {
                [ordered]@{ name = $_; state = 'ABSENT' }
            } else {
                $kind = if ($item.PSIsContainer) { 'DIRECTORY' } else { 'FILE' }
                $entry = [ordered]@{ name = $_; state = $kind }
                if (-not $item.PSIsContainer) {
                    $entry.length = $item.Length
                    $entry.sha256 = (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                }
                $entry
            }
        })
    }
    if ($env:RUNNER_TEMP -and (Test-Path -LiteralPath $env:RUNNER_TEMP -PathType Container)) {
        $script:evidencePath = Join-Path $env:RUNNER_TEMP ("gogoke-r2-06-candidate-smoke-$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt.json")
        try { $script:result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $script:evidencePath -Encoding utf8 }
        catch { $script:evidencePath = $null }
    }
    Write-Error (($script:result | ConvertTo-Json -Depth 6 -Compress) + $(if ($script:evidencePath) { " Evidence: $script:evidencePath" } else { '' }))
    exit 1
}
