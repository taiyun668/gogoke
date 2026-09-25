# Verify a signed, test-only candidate before an isolated cloud installation.
# This script never executes an artifact. The signer remains on trusted main.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^[0-9a-f]{40}$')][string]$ExpectedSourceCommit,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$SigningRunId,
    [Parameter(Mandatory = $true)][ValidateRange(1, 2147483647)][int]$SigningRunAttempt,
    [Parameter(Mandatory = $true)][ValidateRange(1, 9223372036854775807)][long]$SignedArtifactId
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = 'taiyun668/gogoke'
$branch = 'gpt/s1-r4-r2-execution-r1'
$sourceWorkflow = '.github/workflows/gogoke-desktop.yml'
$signingWorkflow = '.github/workflows/gogoke-candidate-sign.yml'
$maxArchiveBytes = 1073741824L
$maxExtractedBytes = 1073741824L
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))

function Read-Api([string]$Endpoint) {
    $json = & $script:ghPath api $Endpoint
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace(($json -join "`n"))) {
        throw "GitHub API read failed: $Endpoint"
    }
    return ($json -join "`n" | ConvertFrom-Json -AsHashtable -Depth 30)
}

function Assert-Run([hashtable]$Run, [long]$Id, [int]$Attempt, [string]$Sha,
    [string]$Branch, [string]$WorkflowPath, [string]$WorkflowName) {
    if ($Run.id -ne $Id -or $Run.run_attempt -ne $Attempt -or
        $Run.repository.full_name -cne $script:repo -or
        $Run.head_repository.full_name -cne $script:repo -or
        $Run.head_branch -cne $Branch -or $Run.head_sha -cne $Sha -or
        $Run.path -cne $WorkflowPath -or $Run.name -cne $WorkflowName -or
        $Run.status -cne 'completed' -or $Run.conclusion -cne 'success') {
        throw "GitHub run is not the exact successful controlled $WorkflowName run"
    }
}

function Assert-UniqueJob([long]$RunId, [int]$Attempt, [string]$JobName) {
    $jobs = Read-Api "repos/$script:repo/actions/runs/$RunId/jobs?filter=latest&per_page=100"
    if ($jobs.total_count -gt 100) { throw 'Run job inventory exceeds the bounded first page' }
    $matches = @($jobs.jobs | Where-Object {
        $_.name -ceq $JobName -and $_.run_attempt -eq $Attempt -and
        $_.status -ceq 'completed' -and $_.conclusion -ceq 'success'
    })
    if ($matches.Count -ne 1) { throw "Required successful job is missing or ambiguous: $JobName" }
}

function Assert-Artifact([hashtable]$Artifact, [long]$Id, [long]$RunId,
    [string]$HeadSha, [string]$Name) {
    if ($Artifact.id -ne $Id -or $Artifact.name -cne $Name -or
        $Artifact.workflow_run.id -ne $RunId -or
        $Artifact.workflow_run.head_sha -cne $HeadSha -or $Artifact.expired -or
        $Artifact.size_in_bytes -le 0 -or $Artifact.size_in_bytes -gt $script:maxArchiveBytes -or
        $Artifact.digest -cnotmatch '^sha256:[0-9a-f]{64}$') {
        throw 'Artifact metadata does not bind the expected run, name, size, and uploaded digest'
    }
}

function Assert-UniqueArtifact([long]$RunId, [long]$Id, [string]$Name) {
    $list = Read-Api "repos/$script:repo/actions/runs/$RunId/artifacts?per_page=100"
    if ($list.total_count -gt 100) { throw 'Artifact inventory exceeds the bounded first page' }
    $matches = @($list.artifacts | Where-Object { $_.id -eq $Id -and $_.name -ceq $Name })
    if ($matches.Count -ne 1) { throw 'Artifact ID and name are unavailable or ambiguous on the run' }
}

function Save-Artifact([hashtable]$Artifact, [string]$Target) {
    if (Test-Path -LiteralPath $Target) { throw 'Artifact archive temporary path already exists' }
    $endpoint = "https://api.github.com/repos/$script:repo/actions/artifacts/$($Artifact.id)/zip"
    Invoke-WebRequest -Uri $endpoint -Headers @{
        Authorization = "Bearer $env:GH_TOKEN"
        Accept = 'application/vnd.github+json'
        'X-GitHub-Api-Version' = '2022-11-28'
    } -MaximumRedirection 5 -OutFile $Target
    if ((Get-Item -LiteralPath $Target).Length -ne [long]$Artifact.size_in_bytes -or
        (Get-FileHash -LiteralPath $Target -Algorithm SHA256).Hash.ToLowerInvariant() -cne
            ([string]$Artifact.digest).Substring(7)) {
        throw 'Downloaded artifact archive differs from the uploaded size or SHA-256'
    }
}

function Expand-Bounded([string]$Archive, [string]$Destination,
    [string[]]$FixedNames, [string]$SetupPattern, [int]$ExpectedCount) {
    if (Test-Path -LiteralPath $Destination) { throw 'Artifact extraction path already exists' }
    New-Item -ItemType Directory -Path $Destination | Out-Null
    $zip = [IO.Compression.ZipFile]::OpenRead($Archive)
    try {
        $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        $total = 0L
        foreach ($entry in $zip.Entries) {
            $name = $entry.FullName
            if (($FixedNames -cnotcontains $name -and $name -cnotmatch $SetupPattern) -or
                -not $seen.Add($name) -or $entry.Length -lt 0 -or
                $entry.Length -gt $script:maxExtractedBytes) {
                throw 'Artifact archive has an unsafe, unexpected, or duplicate member'
            }
            $total += $entry.Length
            if ($seen.Count -gt $ExpectedCount -or $total -gt $script:maxExtractedBytes) {
                throw 'Artifact extracted file set exceeds its fixed bounds'
            }
            $source = $entry.Open()
            try {
                $target = [IO.File]::Open((Join-Path $Destination $name), [IO.FileMode]::CreateNew)
                try {
                    $buffer = [byte[]]::new(65536)
                    $copied = 0L
                    while (($read = $source.Read($buffer, 0, $buffer.Length)) -gt 0) {
                        $copied += $read
                        if ($copied -gt $entry.Length) { throw 'Artifact member exceeded its declared length' }
                        $target.Write($buffer, 0, $read)
                    }
                    if ($copied -ne $entry.Length) { throw 'Artifact member ended before its declared length' }
                } finally { $target.Dispose() }
            } finally { $source.Dispose() }
        }
        if ($seen.Count -ne $ExpectedCount) { throw 'Artifact archive has an incomplete member set' }
        foreach ($fixed in $FixedNames) {
            if (-not $seen.Contains($fixed)) { throw "Artifact archive is missing $fixed" }
        }
        $setup = @($seen | Where-Object { $_ -cmatch $SetupPattern })
        if ($setup.Count -ne 1) { throw 'Artifact archive must contain one setup member' }
        return $setup[0]
    } finally { $zip.Dispose() }
}

function Assert-FileHash([string]$Path, [string]$Expected, [string]$Name) {
    $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $Expected) { throw "$Name differs between signed and frozen artifacts" }
}

function Assert-SignedManifest([string]$Directory, [string]$SourceCommit) {
    $manifestBytes = [IO.File]::ReadAllBytes((Join-Path $Directory 'CANDIDATE-RESOURCES.windows'))
    if ($manifestBytes.Length -gt 4096 -or $manifestBytes.Length -le 0 -or
        @($manifestBytes | Where-Object { $_ -gt 127 -or $_ -eq 13 }).Count -ne 0) {
        throw 'Candidate manifest is not bounded canonical ASCII with LF endings'
    }
    $manifest = [Text.Encoding]::ASCII.GetString($manifestBytes)
    $lines = $manifest.Split("`n")
    if ($lines.Count -ne 11 -or $lines[10] -cne '') {
        throw 'Candidate manifest does not have eight headers and two hashes'
    }
    $fixed = @(
        '# gogoke-Candidate-Purpose: CI_CANDIDATE_RESOURCE',
        '# gogoke-Test-Only: true',
        '# gogoke-Repository: taiyun668/gogoke',
        "# gogoke-Source-Commit: $SourceCommit"
    )
    for ($i = 0; $i -lt $fixed.Count; $i++) {
        if ($lines[$i] -cne $fixed[$i]) { throw 'Candidate manifest fixed identity differs' }
    }
    if ($lines[4] -cnotmatch '^# gogoke-Run-Id: ([1-9][0-9]*)$' -or
        $lines[5] -cnotmatch '^# gogoke-Run-Attempt: ([1-9][0-9]*)$' -or
        $lines[6] -cnotmatch '^# gogoke-Artifact-Id: ([1-9][0-9]*)$' -or
        $lines[7] -cnotmatch '^# gogoke-Version: ((0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?)$') {
        throw 'Candidate manifest run, artifact, or version identity is malformed'
    }
    $version = $lines[7].Substring('# gogoke-Version: '.Length)
    $runId = [long]$lines[4].Substring('# gogoke-Run-Id: '.Length)
    $attempt = [int]$lines[5].Substring('# gogoke-Run-Attempt: '.Length)
    $artifactId = [long]$lines[6].Substring('# gogoke-Artifact-Id: '.Length)
    $indexHash = (Get-FileHash -LiteralPath (Join-Path $Directory 'resource-index.json') -Algorithm SHA256).Hash.ToLowerInvariant()
    $packHash = (Get-FileHash -LiteralPath (Join-Path $Directory 'gogoke-resources.windows.zip') -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($lines[8] -cne "$indexHash  resource-index.json" -or
        $lines[9] -cne "$packHash  gogoke-resources.windows.zip") {
        throw 'Candidate manifest does not bind the exact index and pack bytes'
    }
    $signatureBytes = [IO.File]::ReadAllBytes((Join-Path $Directory 'CANDIDATE-RESOURCES.windows.sig'))
    $signatureText = [Text.Encoding]::ASCII.GetString($signatureBytes)
    if ($signatureBytes.Length -ne 129 -or $signatureText -cnotmatch '^[0-9a-f]{128}\n$') {
        throw 'Candidate signature is not one canonical P-256 hexadecimal line'
    }
    $publicHex = [IO.File]::ReadAllText((Join-Path $script:repoRoot 'apps\desktop\src-tauri\gogoke-candidate-public-key.txt')).Trim()
    if ($publicHex -cnotmatch '^[0-9a-f]{128}$') { throw 'Compiled candidate public key is malformed' }
    $point = [Security.Cryptography.ECPoint]::new()
    $point.X = [Convert]::FromHexString($publicHex.Substring(0, 64))
    $point.Y = [Convert]::FromHexString($publicHex.Substring(64, 64))
    $parameters = [Security.Cryptography.ECParameters]::new()
    $parameters.Curve = [Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
    $parameters.Q = $point
    $key = [Security.Cryptography.ECDsa]::Create($parameters)
    try {
        $prefix = [Text.Encoding]::ASCII.GetBytes("GOGOKE-CI-CANDIDATE-RESOURCE-V1`0")
        $payload = [byte[]]::new($prefix.Length + $manifestBytes.Length)
        [Array]::Copy($prefix, 0, $payload, 0, $prefix.Length)
        [Array]::Copy($manifestBytes, 0, $payload, $prefix.Length, $manifestBytes.Length)
        $signature = [Convert]::FromHexString($signatureText.TrimEnd("`n"))
        if (-not $key.VerifyData($payload, $signature,
            [Security.Cryptography.HashAlgorithmName]::SHA256,
            [Security.Cryptography.DSASignatureFormat]::IeeeP1363FixedFieldConcatenation)) {
            throw 'Candidate purpose-bound P-256 signature is invalid'
        }
    } finally { $key.Dispose() }
    return [pscustomobject]@{
        Version = $version; RunId = $runId; RunAttempt = $attempt; ArtifactId = $artifactId
    }
}

if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_ENVIRONMENT -cne 'github-hosted' -or
    $env:GITHUB_SERVER_URL -cne 'https://github.com' -or $env:GITHUB_REPOSITORY -cne $repo -or
    $env:GITHUB_REF -cne "refs/heads/$branch" -or
    $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$' -or
    $env:GITHUB_EVENT_NAME -cne 'workflow_dispatch') {
    throw 'Candidate preflight requires the exact controlled branch workflow checkout'
}
if ((git -C $repoRoot rev-parse HEAD).Trim() -cne $env:GITHUB_SHA) {
    throw 'Candidate preflight code is not loaded from the exact branch workflow checkout'
}
if ([string]::IsNullOrWhiteSpace($env:GH_TOKEN) -or
    [string]::IsNullOrWhiteSpace($env:RUNNER_TEMP) -or
    [string]::IsNullOrWhiteSpace($env:GITHUB_OUTPUT)) {
    throw 'Read-only GitHub token or runner output path is unavailable'
}
$gh = Get-Command gh -CommandType Application -ErrorAction Stop
$script:ghPath = [IO.Path]::GetFullPath($gh.Source)
if (-not [IO.Path]::IsPathFullyQualified($script:ghPath)) { throw 'gh executable path is not absolute' }
$signRun = Read-Api "repos/$repo/actions/runs/$SigningRunId"
Assert-Run $signRun $SigningRunId $SigningRunAttempt $signRun.head_sha 'main' $signingWorkflow 'gogoke candidate resource signing'
if ($signRun.head_sha -cnotmatch '^[0-9a-f]{40}$') { throw 'Trusted signing main SHA is malformed' }
Assert-UniqueJob $SigningRunId $SigningRunAttempt 'Verify source run and sign candidate resource bytes'
# The source run ID is in the name, but only the signed manifest can provide it.
# Resolve the unique artifact ID first, then verify its final name against the signed manifest.
$signed = Read-Api "repos/$repo/actions/artifacts/$SignedArtifactId"
if ($signed.id -ne $SignedArtifactId -or $signed.workflow_run.id -ne $SigningRunId -or
    $signed.workflow_run.head_sha -cne $signRun.head_sha -or $signed.expired -or
    $signed.size_in_bytes -le 0 -or $signed.size_in_bytes -gt $maxArchiveBytes -or
    $signed.digest -cnotmatch '^sha256:[0-9a-f]{64}$') {
    throw 'Signed artifact is not the exact bounded output of the successful trusted run'
}
$candidateArchive = Join-Path $env:RUNNER_TEMP "gogoke-signed-candidate-$SignedArtifactId.zip"
Save-Artifact $signed $candidateArchive
$candidateDir = Join-Path $env:RUNNER_TEMP "gogoke-signed-candidate-$SignedArtifactId"
$signedSetupName = Expand-Bounded $candidateArchive $candidateDir @(
    'gogoke-package-hashes.json', 'gogoke-portable.exe', 'gogoke-native-host.exe',
    'node.exe', 'gogoke-resources.windows.zip', 'resource-index.json',
    'CANDIDATE-RESOURCES.windows', 'CANDIDATE-RESOURCES.windows.sig'
) '^gogoke-[0-9A-Za-z.+-]+-windows-x64-unsigned-setup\.exe$' 9
$manifest = Assert-SignedManifest $candidateDir $ExpectedSourceCommit
$expectedSignedName = "gogoke-windows-candidate-$ExpectedSourceCommit-$($manifest.RunId)-$SigningRunId-$SigningRunAttempt"
Assert-Artifact $signed $SignedArtifactId $SigningRunId $signRun.head_sha $expectedSignedName
Assert-UniqueArtifact $SigningRunId $SignedArtifactId $expectedSignedName
$expectedSetupName = "gogoke-$($manifest.Version)-windows-x64-unsigned-setup.exe"
if ($signedSetupName -cne $expectedSetupName) { throw 'Signed setup name differs from signed version' }

$sourceRun = Read-Api "repos/$repo/actions/runs/$($manifest.RunId)"
Assert-Run $sourceRun $manifest.RunId $manifest.RunAttempt $ExpectedSourceCommit $branch $sourceWorkflow 'gogoke desktop CI'
foreach ($name in @(
    'Browser build and tests', 'Generate locked dependency notices',
    'Clean Windows build (frozen)', 'Clean Windows build (repro)',
    'Compare independent Windows executable bytes'
)) { Assert-UniqueJob $manifest.RunId $manifest.RunAttempt $name }
$frozenName = "gogoke-windows-frozen-$ExpectedSourceCommit-$($manifest.RunId)"
$frozen = Read-Api "repos/$repo/actions/artifacts/$($manifest.ArtifactId)"
Assert-Artifact $frozen $manifest.ArtifactId $manifest.RunId $ExpectedSourceCommit $frozenName
Assert-UniqueArtifact $manifest.RunId $manifest.ArtifactId $frozenName
$frozenArchive = Join-Path $env:RUNNER_TEMP "gogoke-frozen-$($manifest.ArtifactId).zip"
Save-Artifact $frozen $frozenArchive
$frozenDir = Join-Path $env:RUNNER_TEMP "gogoke-frozen-$($manifest.ArtifactId)"
$frozenSetupName = Expand-Bounded $frozenArchive $frozenDir @(
    'frozen-build.json', 'gogoke-package-hashes.json', 'gogoke-portable.exe',
    'gogoke-installed-shell.nsis.exe', 'gogoke-native-host.exe', 'node.exe',
    'gogoke-resources.windows.zip', 'resource-index.json'
) '^gogoke-[0-9A-Za-z.+-]+-windows-x64-unsigned-setup\.exe$' 9
if ($frozenSetupName -cne $expectedSetupName) { throw 'Frozen setup name differs from signed version' }
& python (Join-Path $repoRoot 'tools\ci\gogoke_ci_frozen_artifact.py') verify `
    --directory $frozenDir --source-commit $ExpectedSourceCommit `
    --run-id $manifest.RunId --run-attempt $manifest.RunAttempt --lane frozen
if ($LASTEXITCODE -ne 0) { throw 'Frozen byte inventory verification failed' }
foreach ($name in @(
    'gogoke-package-hashes.json', 'gogoke-portable.exe', 'gogoke-native-host.exe',
    'node.exe', 'gogoke-resources.windows.zip', 'resource-index.json', $expectedSetupName
)) {
    $hash = (Get-FileHash -LiteralPath (Join-Path $frozenDir $name) -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-FileHash (Join-Path $candidateDir $name) $hash $name
}
$index = Get-Content -LiteralPath (Join-Path $candidateDir 'resource-index.json') -Raw | ConvertFrom-Json -AsHashtable
if ($index.schema -cne 'gogoke.resource-index.v1' -or
    $index.sourceCommit -cne $ExpectedSourceCommit -or
    $index.version -cne $manifest.Version) {
    throw 'Signed resource index identity differs from source and manifest'
}
$metadata = Get-Content -LiteralPath (Join-Path $frozenDir 'frozen-build.json') -Raw | ConvertFrom-Json -AsHashtable
$outputs = [ordered]@{
    artifact_directory = $candidateDir
    source_run_id = $manifest.RunId
    source_run_attempt = $manifest.RunAttempt
    source_artifact_id = $manifest.ArtifactId
    signing_run_id = $SigningRunId
    signing_run_attempt = $SigningRunAttempt
    signing_main_sha = $signRun.head_sha
    version = $manifest.Version
    setup_sha256 = $metadata.files[$expectedSetupName].sha256
    installed_shell_sha256 = $index.executables.installedShell.sha256
    native_host_sha256 = $index.executables.nativeHost.sha256
    node_sha256 = $index.executables.node.sha256
}
foreach ($pair in $outputs.GetEnumerator()) { "$($pair.Key)=$($pair.Value)" >> $env:GITHUB_OUTPUT }
Write-Output "Verified signed candidate and exact frozen source for $ExpectedSourceCommit"
