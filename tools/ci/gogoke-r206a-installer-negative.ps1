# Cloud-only negative axes for the separately instrumented R2-06a NSIS setup.
# The signed candidate has already passed gogoke-candidate-installed-preflight.ps1.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$SignedArtifactDirectory,
    [Parameter(Mandatory = $true)][string]$InstrumentArtifactDirectory,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSourceCommit,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedRunId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ExpectedRunAttempt,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedSourceArtifactId,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedProductionSetupSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{64}$')][string]$ExpectedInstalledShellSha256,
    [Parameter(Mandatory = $true)][ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$')][string]$ExpectedVersion,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$ExpectedSmokeRunId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$ExpectedSmokeRunAttempt
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:stage = 'environment'
$script:axis = $null
$script:workRoot = $null
$script:processes = [Collections.Generic.List[Diagnostics.Process]]::new()
$script:registration = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate'
$script:productRootKey = 'HKCU:\Software\gogoke\gogoke-candidate'
$script:receipt = [ordered]@{
    schema = 'gogoke.r2-06a.installer-negative.v1'
    state = 'FAIL'
    stage = $script:stage
    sourceCommit = $ExpectedSourceCommit
    sourceRunId = $ExpectedRunId
    sourceRunAttempt = $ExpectedRunAttempt
    sourceArtifactId = $ExpectedSourceArtifactId
    smokeRunId = $ExpectedSmokeRunId
    smokeRunAttempt = $ExpectedSmokeRunAttempt
    productionSetupSha256 = $ExpectedProductionSetupSha256
    leafInsertion = [ordered]@{ state = 'FAIL' }
    sameDomainDifferentParents = [ordered]@{ state = 'FAIL' }
    uninstallInstallInterleave = [ordered]@{ state = 'FAIL' }
    barrierArrivalsSeconds = [ordered]@{}
    barrierDiagnosticsAt90Seconds = [ordered]@{}
}

function Assert-PlainFile([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Expected a plain file: $Path"
    }
    return $item
}
function Assert-PlainDirectory([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if (-not $item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
        throw "Expected a plain directory: $Path"
    }
    return $item
}
function Get-Sha([string]$Path) {
    [void](Assert-PlainFile $Path)
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Assert-BelowRunner([string]$Path) {
    $full = [IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($script:runnerRoot + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path is not a child of RUNNER_TEMP: $full"
    }
    return $full
}
function New-FreshDirectory([string]$Path) {
    $full = Assert-BelowRunner $Path
    if (Test-Path -LiteralPath $full) { throw "Test directory is not fresh: $full" }
    [void](New-Item -ItemType Directory -Path $full -ErrorAction Stop)
    [void](Assert-PlainDirectory $full)
    return $full
}
function Assert-NoCandidateRegistration {
    if (Test-Path -LiteralPath $script:registration) { throw 'Candidate uninstall registration already exists or remains' }
    if (Test-Path -LiteralPath $script:productRootKey) { throw 'Candidate product-root registry key exists' }
}
function Start-Installer([string]$Setup, [string]$Target) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Setup
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.Environment['TEMP'] = $script:runnerRoot
    $start.Environment['TMP'] = $script:runnerRoot
    [void]$start.ArgumentList.Add('/S')
    [void]$start.ArgumentList.Add("/D=$Target")
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Instrumented installer did not start' }
    $script:processes.Add($process)
    return $process
}
function Wait-Exit([Diagnostics.Process]$Process, [int]$Milliseconds, [string]$Label) {
    if (-not $Process.WaitForExit($Milliseconds)) { throw "$Label exceeded its process bound; runner teardown owns the retained process" }
    return $Process.ExitCode
}
function Wait-Barrier([Diagnostics.Process]$Process, [string]$Label) {
    # Keep the former 90-second limit as a diagnostic. The existing full
    # setup bound is 15 minutes; a slow signed-set preflight is not a failed
    # ownership barrier until that same bound expires.
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $deadline = [DateTime]::UtcNow.AddMinutes(15)
    $observedOldMark = $false
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $script:readyPath -PathType Leaf) {
            [void](Assert-PlainFile $script:readyPath)
            if ($Process.HasExited) { throw "$Label exited at or immediately after the barrier" }
            $script:receipt.barrierArrivalsSeconds[$Label] = [Math]::Round($watch.Elapsed.TotalSeconds, 1)
            return
        }
        if ($Process.HasExited) { throw "$Label exited before the post-preflight barrier: $($Process.ExitCode)" }
        if (-not $observedOldMark -and $watch.Elapsed.TotalSeconds -ge 90) {
            $observedOldMark = $true
            $stages = @($script:stagePaths.Keys | Where-Object {
                Test-Path -LiteralPath $script:stagePaths[$_] -PathType Leaf
            })
            $children = try {
                @(Get-CimInstance Win32_Process -Filter "ParentProcessId = $($Process.Id)" -ErrorAction Stop |
                    ForEach-Object { [string]$_.Name })
            } catch { @('UNAVAILABLE') }
            $script:receipt.barrierDiagnosticsAt90Seconds[$Label] = [ordered]@{
                observedStages = $stages; childProcessNames = $children
                processCpuSeconds = try { [Math]::Round((Get-Process -Id $Process.Id -ErrorAction Stop).CPU, 1) } catch { $null }
            }
        }
        Start-Sleep -Milliseconds 100
    }
    $stages = @($script:stagePaths.Keys | Where-Object {
        Test-Path -LiteralPath $script:stagePaths[$_] -PathType Leaf
    })
    throw "$Label did not reach the post-preflight barrier within the existing 15-minute setup bound; observed stages: $($stages -join ',')"
}
function Clear-TestMarkers {
    foreach ($path in @($script:readyPath, $script:goPath) + @($script:stagePaths.Values)) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
}
function Wait-RegistrationLockReleased {
    $localData = [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    $path = Join-Path $localData 'gogoke-registration-CI_CANDIDATE_RESOURCE.lock'
    [void](Assert-PlainFile $path)
    $deadline = [DateTime]::UtcNow.AddSeconds(300)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $stream = [IO.File]::Open($path, [IO.FileMode]::Open, [IO.FileAccess]::ReadWrite,
                [IO.FileShare]::None)
            try { [void](Assert-PlainFile $path) } finally { $stream.Dispose() }
            return
        } catch [IO.IOException] {
            if (($_.Exception.HResult -band 0xFFFF) -ne 32) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
    throw 'Candidate registration-domain lock was not released after bounded uninstall'
}
function New-Go {
    if (Test-Path -LiteralPath $script:goPath) { throw 'Barrier release marker already exists' }
    [IO.File]::WriteAllBytes($script:goPath, [Text.Encoding]::ASCII.GetBytes('go'))
}
function Assert-RegisteredAt([string]$Target) {
    $key = Get-ItemProperty -LiteralPath $script:registration -ErrorAction Stop
    $exe = Join-Path $Target 'gogoke.exe'
    if ([string]$key.InstallLocation -cne $Target -or
        [string]$key.InstallDomain -cne 'CI_CANDIDATE_RESOURCE' -or
        [string]$key.DisplayVersion -cne $ExpectedVersion -or
        [string]$key.InstallInstanceId -cnotmatch '^.{16,}$' -or
        [string]$key.UninstallString -cne ('"' + $exe + '" --uninstall') -or
        [string]$key.QuietUninstallString -cne ('"' + $exe + '" --uninstall --quiet')) {
        throw 'Candidate registration differs from exact installer A target'
    }
    if (Test-Path -LiteralPath $script:productRootKey) { throw 'Candidate product-root registry key exists' }
    return [string]$key.InstallInstanceId
}
function Start-Uninstall([string]$Exe) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Exe
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    $start.Environment['TEMP'] = $script:runnerRoot
    $start.Environment['TMP'] = $script:runnerRoot
    [void]$start.ArgumentList.Add('--uninstall')
    [void]$start.ArgumentList.Add('--quiet')
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Installed candidate uninstaller did not start' }
    $script:processes.Add($process)
    return $process
}

try {
    if (-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or
        $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
        $env:GITHUB_SERVER_URL -cne 'https://github.com' -or
        $env:GITHUB_REPOSITORY -cne 'taiyun668/gogoke' -or
        $env:GITHUB_REF -cne 'refs/heads/gpt/s1-r4-r2-execution-r1' -or
        $env:GITHUB_RUN_ID -cne [string]$ExpectedSmokeRunId -or
        $env:GITHUB_RUN_ATTEMPT -cne [string]$ExpectedSmokeRunAttempt) {
        throw 'Negative installer test requires the exact GitHub-hosted Windows smoke run'
    }
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        if ($identity.IsSystem -or $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Negative installer test requires a Medium process'
        }
    } finally { $identity.Dispose() }
    foreach ($name in @('GH_TOKEN', 'GITHUB_TOKEN', 'GOGOKE_CANDIDATE_P256_KEY',
                       'ACTIONS_RUNTIME_TOKEN', 'ACTIONS_ID_TOKEN_REQUEST_TOKEN')) {
        if (-not [string]::IsNullOrEmpty([Environment]::GetEnvironmentVariable($name))) {
            throw "Credential environment variable is visible to installer test: $name"
        }
    }
    if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or
        -not [IO.Path]::IsPathFullyQualified($env:RUNNER_TEMP)) { throw 'RUNNER_TEMP unavailable' }
    $script:runnerRoot = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
    [void](Assert-PlainDirectory $script:runnerRoot)
    # Child installers receive RUNNER_TEMP as TEMP/TMP. Their compiled NSIS
    # $TEMP barrier therefore lands under the verified runner temporary root.
    $script:readyPath = Join-Path $script:runnerRoot 'gogoke-r206a-test-barrier.ready'
    $script:goPath = Join-Path $script:runnerRoot 'gogoke-r206a-test-barrier.go'
    $script:stagePaths = [ordered]@{}
    foreach ($name in @('lock', 'lock-busy', 'preflight-start', 'preflight-end', 'install-start')) {
        $script:stagePaths[$name] = Join-Path $script:runnerRoot "gogoke-r206a-test-barrier.stage-$name"
    }
    foreach ($path in @($script:readyPath, $script:goPath) + @($script:stagePaths.Values)) {
        if (Test-Path -LiteralPath $path) { throw "Barrier marker is not fresh: $path" }
    }
    Assert-NoCandidateRegistration
    Wait-RegistrationLockReleased

    $script:stage = 'artifact-identity'
    $signedRoot = (Resolve-Path -LiteralPath $SignedArtifactDirectory).Path
    $instrumentRoot = (Resolve-Path -LiteralPath $InstrumentArtifactDirectory).Path
    [void](Assert-PlainDirectory $signedRoot)
    [void](Assert-PlainDirectory $instrumentRoot)
    $productionSetup = Join-Path $signedRoot "gogoke-$ExpectedVersion-windows-x64-unsigned-setup.exe"
    if ((Get-Sha $productionSetup) -cne $ExpectedProductionSetupSha256) { throw 'Signed production setup SHA-256 differs from trusted preflight output' }
    $rawProductionShellHash = Get-Sha (Join-Path $signedRoot 'gogoke-portable.exe')
    $testSetup = Join-Path $instrumentRoot 'gogoke-r206a-test-setup.exe'
    $testHash = Get-Sha $testSetup
    if ($testHash -ceq $ExpectedProductionSetupSha256) { throw 'Instrumented setup is byte-identical to production setup' }
    $instrumentPath = Join-Path $instrumentRoot 'instrument.json'
    $instrumentFile = Assert-PlainFile $instrumentPath
    if ($instrumentFile.Length -gt 4096 -or $instrumentFile.Length -le 0) { throw 'Instrument metadata size is invalid' }
    $instrument = Get-Content -LiteralPath $instrumentPath -Raw | ConvertFrom-Json -AsHashtable
    if ($instrument.Count -ne 7 -or
        $instrument.schema -cne 'gogoke.r2-06a.nsis-negative-instrument.v1' -or
        $instrument.sourceCommit -cne $ExpectedSourceCommit -or
        [string]$instrument.runId -cne [string]$ExpectedRunId -or
        [string]$instrument.runAttempt -cne [string]$ExpectedRunAttempt -or
        $instrument.setupSha256 -cne $testHash -or
        $instrument.productionShellSha256 -cne $rawProductionShellHash -or
        $instrument.barrier -cne 'CI_ONLY') {
        throw 'Instrument metadata does not bind the exact source and separate CI-only setup'
    }
    $sidecars = @('gogoke-resources.windows.zip', 'resource-index.json',
                  'CANDIDATE-RESOURCES.windows', 'CANDIDATE-RESOURCES.windows.sig')
    foreach ($name in $sidecars) { [void](Assert-PlainFile (Join-Path $signedRoot $name)) }
    $manifestBytes = [IO.File]::ReadAllBytes((Join-Path $signedRoot 'CANDIDATE-RESOURCES.windows'))
    if ($manifestBytes.Length -gt 4096 -or $manifestBytes.Length -le 0) { throw 'Signed manifest size is invalid' }
    $manifestLines = [Text.Encoding]::ASCII.GetString($manifestBytes).Split("`n")
    $expectedHeaders = @('# gogoke-Candidate-Purpose: CI_CANDIDATE_RESOURCE', '# gogoke-Test-Only: true',
        '# gogoke-Repository: taiyun668/gogoke', "# gogoke-Source-Commit: $ExpectedSourceCommit",
        "# gogoke-Run-Id: $ExpectedRunId", "# gogoke-Run-Attempt: $ExpectedRunAttempt",
        "# gogoke-Artifact-Id: $ExpectedSourceArtifactId", "# gogoke-Version: $ExpectedVersion")
    if ($manifestLines.Count -ne 11 -or $manifestLines[10] -cne '') { throw 'Signed manifest line count differs' }
    for ($i = 0; $i -lt 8; $i++) {
        if ($manifestLines[$i] -cne $expectedHeaders[$i]) { throw 'Signed candidate manifest identity differs' }
    }
    if ($manifestLines[8] -cne "$(Get-Sha (Join-Path $signedRoot 'resource-index.json'))  resource-index.json" -or
        $manifestLines[9] -cne "$(Get-Sha (Join-Path $signedRoot 'gogoke-resources.windows.zip'))  gogoke-resources.windows.zip") {
        throw 'Signed manifest does not bind copied resource bytes'
    }
    $index = Get-Content -LiteralPath (Join-Path $signedRoot 'resource-index.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($index.schema -cne 'gogoke.resource-index.v1' -or
        $index.sourceCommit -cne $ExpectedSourceCommit -or $index.version -cne $ExpectedVersion -or
        $index.executables.installedShell.sha256 -cne $ExpectedInstalledShellSha256) {
        throw 'Signed resource index does not match trusted source and shell identity'
    }
    $script:receipt.instrumentedSetupSha256 = $testHash

    $script:stage = 'fresh-test-paths'
    $runTag = "$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt"
    $script:workRoot = New-FreshDirectory (Join-Path $script:runnerRoot "gogoke-r206a-negative-$runTag")
    $setupRoot = New-FreshDirectory (Join-Path $script:workRoot 'setup')
    foreach ($name in $sidecars) {
        Copy-Item -LiteralPath (Join-Path $signedRoot $name) -Destination (Join-Path $setupRoot $name) -ErrorAction Stop
        if ((Get-Sha (Join-Path $setupRoot $name)) -cne (Get-Sha (Join-Path $signedRoot $name))) {
            throw "Copied candidate sidecar differs: $name"
        }
    }
    $script:stagedSetup = Join-Path $setupRoot 'gogoke-r206a-test-setup.exe'
    Copy-Item -LiteralPath $testSetup -Destination $script:stagedSetup -ErrorAction Stop
    if ((Get-Sha $script:stagedSetup) -cne $testHash) { throw 'Staged instrumented setup differs' }
    foreach ($name in @('SHA256SUMS.windows', 'SHA256SUMS.windows.sig')) {
        if (Test-Path -LiteralPath (Join-Path $setupRoot $name)) { throw 'Formal sidecar leaked into candidate staging' }
    }

    $script:stage = 'leaf-insertion'
    $script:axis = 'leafInsertion'
    $leafParent = New-FreshDirectory (Join-Path $script:workRoot 'leaf-parent')
    $leafTarget = Join-Path $leafParent 'install'
    $leafProcess = Start-Installer $script:stagedSetup $leafTarget
    Wait-Barrier $leafProcess 'Leaf insertion installer'
    $sentinel = Join-Path $leafTarget 'gogoke.exe'
    [void](Assert-BelowRunner $sentinel)
    if (Test-Path -LiteralPath $sentinel) { throw 'First publish target was already present before insertion' }
    $sentinelBytes = [Text.Encoding]::UTF8.GetBytes("R2-06a leaf ownership sentinel $runTag`n")
    $created = [IO.File]::Open($sentinel, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try { $created.Write($sentinelBytes, 0, $sentinelBytes.Length) } finally { $created.Dispose() }
    New-Go
    $leafExit = Wait-Exit $leafProcess 60000 'Leaf insertion installer'
    if ($leafExit -eq 0) { throw 'Leaf insertion installer unexpectedly succeeded' }
    if (-not (Test-Path -LiteralPath $sentinel -PathType Leaf) -or
        [Convert]::ToBase64String([IO.File]::ReadAllBytes($sentinel)) -cne
            [Convert]::ToBase64String($sentinelBytes)) {
        throw 'Leaf insertion changed or removed the exact sentinel bytes'
    }
    Assert-NoCandidateRegistration
    $script:receipt.leafInsertion = [ordered]@{
        state = 'PASS'; installerExitCode = $leafExit; sentinelSha256 = (Get-Sha $sentinel)
        sentinelPath = $sentinel; registration = 'ABSENT'
    }
    # These markers are ours; leave the failed install tree for runner teardown.
    Clear-TestMarkers

    $script:stage = 'same-domain-different-parents'
    $script:axis = 'sameDomainDifferentParents'
    $parentA = New-FreshDirectory (Join-Path $script:workRoot 'parent-a')
    $parentB = New-FreshDirectory (Join-Path $script:workRoot 'parent-b')
    $targetA = Join-Path $parentA 'install'
    $targetB = Join-Path $parentB 'install'
    $processA = Start-Installer $script:stagedSetup $targetA
    Wait-Barrier $processA 'Installer A'
    # A has already written its markers. Remove only those exact test files so
    # any new stage or .ready marker can be attributed to B while A is paused.
    Remove-Item -LiteralPath $script:readyPath -Force
    foreach ($path in $script:stagePaths.Values) {
        if (Test-Path -LiteralPath $path) { Remove-Item -LiteralPath $path -Force }
    }
    $processB = Start-Installer $script:stagedSetup $targetB
    $exitB = Wait-Exit $processB 30000 'Installer B'
    if ($exitB -eq 0) { throw 'Installer B unexpectedly succeeded while A held the domain lock' }
    [void](Assert-PlainFile $script:stagePaths['lock-busy'])
    if (Test-Path -LiteralPath $script:readyPath) { throw 'Installer B reached the post-preflight barrier' }
    if (Test-Path -LiteralPath $targetB) { throw 'Installer B touched its separate target tree' }
    if ($processA.HasExited) { throw 'Installer A exited before the domain-lock observation finished' }
    Assert-NoCandidateRegistration
    New-Go
    $exitA = Wait-Exit $processA 900000 'Installer A'
    if ($exitA -ne 0) { throw "Installer A failed after release: $exitA" }
    $instanceId = Assert-RegisteredAt $targetA
    if ((Get-Sha (Join-Path $targetA 'gogoke.exe')) -cne $ExpectedInstalledShellSha256) {
        throw 'Installer A did not publish the exact signed installed shell'
    }
    $script:receipt.sameDomainDifferentParents = [ordered]@{
        state = 'FAIL'; installerAExitCode = $exitA; installerBExitCode = $exitB
        installerBBarrier = 'ABSENT'; installerBTarget = 'ABSENT'; registrationTarget = $targetA
        installInstanceId = $instanceId
    }
    $script:stage = 'uninstall-A'
    $uninstall = Start-Uninstall (Join-Path $targetA 'gogoke.exe')
    $uninstallExit = Wait-Exit $uninstall 30000 'Installer A product uninstall'
    if ($uninstallExit -ne 0) { throw "Installed A uninstall failed: $uninstallExit" }
    $deadline = [DateTime]::UtcNow.AddSeconds(300)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not (Test-Path -LiteralPath $script:registration)) { break }
        Start-Sleep -Milliseconds 250
    }
    Assert-NoCandidateRegistration
    Wait-RegistrationLockReleased
    $script:receipt.sameDomainDifferentParents.uninstallExitCode = $uninstallExit
    $script:receipt.sameDomainDifferentParents.postUninstallRegistration = 'ABSENT'
    $script:receipt.sameDomainDifferentParents.state = 'PASS'

    $script:stage = 'uninstall-install-interleave'
    $script:axis = 'uninstallInstallInterleave'
    # A CI-only in-memory copy of the production finalizer pauses between its
    # last Assert-Root and DeleteSubKey. Its lock code is unchanged; the pause
    # cannot enter the production shell. B must fail before its own barrier.
    Clear-TestMarkers
    $fixtureParent = New-FreshDirectory (Join-Path $script:workRoot 'uninstall-parent-a')
    $nextParent = New-FreshDirectory (Join-Path $script:workRoot 'uninstall-parent-b')
    $fixtureHelper = (Resolve-Path -LiteralPath './tools/ci/test_gogoke_uninstall_finalizer_cloud.py').Path
    $payloadPath = & python -B $fixtureHelper --prepare-interleave $fixtureParent
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $payloadPath -PathType Leaf)) {
        throw 'Cloud finalizer A fixture could not be prepared'
    }
    $payload = Get-Content -LiteralPath $payloadPath -Raw | ConvertFrom-Json -AsHashtable
    $deleteReady = Join-Path $script:workRoot 'uninstall-a-before-delete.ready'
    $deleteGo = Join-Path $script:workRoot 'uninstall-a-before-delete.go'
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = (Get-Command python -ErrorAction Stop).Source
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.WindowStyle = [Diagnostics.ProcessWindowStyle]::Hidden
    foreach ($argument in @('-B', $fixtureHelper, '--parent-delete-barrier', $payloadPath, $deleteReady, $deleteGo)) {
        [void]$start.ArgumentList.Add($argument)
    }
    $fixtureProcess = [Diagnostics.Process]::new()
    $fixtureProcess.StartInfo = $start
    if (-not $fixtureProcess.Start()) { throw 'Cloud finalizer A fixture did not start' }
    $script:processes.Add($fixtureProcess)
    $fixtureExit = Wait-Exit $fixtureProcess 30000 'Cloud finalizer A parent handoff'
    if ($fixtureExit -ne 0) { throw "Cloud finalizer A parent failed: $fixtureExit" }
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $deleteReadyObserved = $false
    while (-not $deleteReadyObserved) {
        if (Test-Path -LiteralPath $deleteReady -PathType Leaf) {
            try {
                [void](Assert-PlainFile $deleteReady)
                $markerBytes = [IO.File]::ReadAllBytes($deleteReady)
                if ([Text.Encoding]::ASCII.GetString($markerBytes) -cne 'before-delete') {
                    throw 'Cloud finalizer A pre-delete marker differs'
                }
                $deleteReadyObserved = $true
                break
            } catch [IO.IOException] {
                if (($_.Exception.HResult -band 0xFFFF) -ne 32) { throw }
            }
        }
        if ([DateTime]::UtcNow -ge $deadline) { throw 'Cloud finalizer A did not reach the pre-delete barrier' }
        Start-Sleep -Milliseconds 50
    }
    $fixtureRegistration = Get-ItemProperty -LiteralPath $script:registration -ErrorAction Stop
    if ([string]$fixtureRegistration.InstallInstanceId -cne [string]$payload.instance -or
        [string]$fixtureRegistration.InstallLocation -cne [string]$payload.root) {
        throw 'Cloud finalizer A registration changed before B'
    }
    $nextTarget = Join-Path $nextParent 'install'
    $blockedB = Start-Installer $script:stagedSetup $nextTarget
    $blockedExit = Wait-Exit $blockedB 30000 'Installer B during A uninstall'
    [void](Assert-PlainFile $script:stagePaths['lock-busy'])
    if ($blockedExit -eq 0 -or (Test-Path -LiteralPath $script:readyPath) -or
        (Test-Path -LiteralPath $nextTarget)) {
        throw 'Installer B crossed the domain lock held by finalizer A'
    }
    $fixtureRegistration = Get-ItemProperty -LiteralPath $script:registration -ErrorAction Stop
    if ([string]$fixtureRegistration.InstallInstanceId -cne [string]$payload.instance) {
        throw 'Installer B changed fixture A registration while A held the domain lock'
    }
    [IO.File]::WriteAllBytes($deleteGo, [Text.Encoding]::ASCII.GetBytes('go'))
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    $finalizerReceipt = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path -LiteralPath $payload.receipt -PathType Leaf) {
            try {
                $observed = Get-Content -LiteralPath $payload.receipt -Raw | ConvertFrom-Json -AsHashtable
                if ($observed.state -cin @('DELETED', 'FAILED')) { $finalizerReceipt = $observed; break }
            } catch { }
        }
        Start-Sleep -Milliseconds 100
    }
    if ($null -eq $finalizerReceipt -or $finalizerReceipt.state -cne 'DELETED' -or
        (Test-Path -LiteralPath $script:registration) -or
        (Test-Path -LiteralPath (Join-Path $payload.root 'gogoke.exe'))) {
        throw 'Cloud finalizer A did not complete its exact deletion before B resumed'
    }
    Wait-RegistrationLockReleased
    Clear-TestMarkers
    $resumedB = Start-Installer $script:stagedSetup $nextTarget
    Wait-Barrier $resumedB 'Installer B after A uninstall'
    New-Go
    $resumedExit = Wait-Exit $resumedB 900000 'Installer B after A uninstall'
    if ($resumedExit -ne 0) { throw "Installer B failed after A released the domain lock: $resumedExit" }
    $nextInstance = Assert-RegisteredAt $nextTarget
    if ((Get-Sha (Join-Path $nextTarget 'gogoke.exe')) -cne $ExpectedInstalledShellSha256) {
        throw 'Installer B did not publish the exact signed installed shell'
    }
    $script:receipt.uninstallInstallInterleave = [ordered]@{
        state = 'FAIL'; finalizerAReceipt = 'DELETED'; blockedBExitCode = $blockedExit
        blockedBBarrier = 'ABSENT'; blockedBTarget = 'ABSENT'
        resumedBExitCode = $resumedExit; resumedBInstanceId = $nextInstance
        registrationTarget = $nextTarget
    }
    $uninstallB = Start-Uninstall (Join-Path $nextTarget 'gogoke.exe')
    $uninstallBExit = Wait-Exit $uninstallB 30000 'Installer B product uninstall'
    if ($uninstallBExit -ne 0) { throw "Installed B uninstall failed: $uninstallBExit" }
    $deadline = [DateTime]::UtcNow.AddSeconds(300)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (-not (Test-Path -LiteralPath $script:registration)) { break }
        Start-Sleep -Milliseconds 250
    }
    Assert-NoCandidateRegistration
    Wait-RegistrationLockReleased
    $script:receipt.uninstallInstallInterleave.postUninstallRegistration = 'ABSENT'
    $script:receipt.uninstallInstallInterleave.state = 'PASS'
    $script:receipt.state = 'PASS'
    $script:stage = 'complete'
} catch {
    $message = [string]$_.Exception.Message
    if ($message.Length -gt 320) { $message = $message.Substring(0, 320) }
    $script:receipt.error = $message
    $script:receipt.failedAxis = $script:axis
} finally {
    $script:receipt.stage = $script:stage
    if ($script:processes.Count -gt 0) {
        $script:receipt.processes = @($script:processes | ForEach-Object {
            [ordered]@{ id = $_.Id; exited = $_.HasExited }
        })
    }
    if ($script:workRoot) { $script:receipt.retainedTestRoot = $script:workRoot }
    if ($env:RUNNER_TEMP -and (Test-Path -LiteralPath $env:RUNNER_TEMP -PathType Container)) {
        try {
            $receiptPath = Join-Path $env:RUNNER_TEMP "gogoke-r206a-installer-negative-$ExpectedSmokeRunId-$ExpectedSmokeRunAttempt.json"
            if (-not (Test-Path -LiteralPath $receiptPath)) {
                $script:receipt | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $receiptPath -Encoding utf8
            }
            Write-Output "Receipt: $receiptPath"
        } catch { Write-Warning "Could not write bounded negative-axis receipt: $($_.Exception.Message)" }
    }
    foreach ($process in $script:processes) { $process.Dispose() }
}
if ($script:receipt.state -cne 'PASS') { Write-Error ($script:receipt | ConvertTo-Json -Depth 8 -Compress); exit 1 }
Write-Output ($script:receipt | ConvertTo-Json -Depth 8 -Compress)
