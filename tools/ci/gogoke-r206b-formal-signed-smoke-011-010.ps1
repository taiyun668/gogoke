# Formal-signature smoke on exact frozen full 0.1.11 bytes. The previously frozen
# resources 0.1.10 signature is independently verified but no resource update is applied.
# The Owner private key is never an input. Signatures arrive as a temporary CI secret.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateSet('Preflight', 'Exercise')][string]$Mode,
    [Parameter(Mandatory)][string]$WorkRoot,
    [string]$SourceCommit,
    [long]$SourceRunId,
    [int]$SourceRunAttempt,
    [long]$SourceArtifactId
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or $env:GITHUB_ACTIONS -cne 'true' -or
    $env:GITHUB_REPOSITORY -cne 'taiyun668/gogoke' -or
    $env:GITHUB_REF -cne 'refs/heads/gpt/s1-r4-r2-execution-r1') {
    throw 'Formal signed smoke is restricted to the exact cloud execution branch.'
}
$runnerTemp = [IO.Path]::GetFullPath($env:RUNNER_TEMP).TrimEnd('\')
$work = [IO.Path]::GetFullPath($WorkRoot)
if (-not $work.StartsWith($runnerTemp + '\', [StringComparison]::OrdinalIgnoreCase) -or
    [IO.Path]::GetFileName($work) -cne "gogoke-r206b-formal-$env:GITHUB_RUN_ID") {
    throw 'Formal smoke work root is outside its exact runner temporary namespace.'
}
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$fullSource = 'c1fc362773f64576c23d17128d9cd86046b5c6b3'
$resourcesSource = '32a148e4c187544d06bbf007072388d310c29622'
$fullManifestHash = '82c3b13246aa79ba0e021fa85dece73a172f6704509f76b15403dde989348bf0'
$resourcesManifestHash = 'a979c118efdf12d30d8f1fe2467c0649dc70603b8c20ce2a802331a896a4a60d'
$fullSignatureHash = '526a8a77128716fe95416af572a7bbaf1335a691bf987553acdb4739b21ea7fa'
$resourcesSignatureHash = 'bcaa37bb5f185ace052efbcff62f64eb3f5298a67cb5fee7471d9ab5e149e8e1'
$setupHash = 'f3a5b894abfc9e4351aa41ac79666288e0c6d836b7cd6aa99e1b6e1294966817'
$installedShellHash = '3384deda28240834358aee6b6b0bdf14a15fe197de78498209a80674989492b8'
$nativeHostHash = '9a28408837efd5c05e0c21c5645a3a82d9a7d2a2839f3e2a56ae756e114ad559'
$nodeHash = 'e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50'

function Hash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Require-Hash([string]$Path, [string]$Expected) {
    $name = [IO.Path]::GetFileName($Path)
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Formal smoke asset missing: $name" }
    if ((Hash $Path) -cne $Expected) { throw "Formal smoke asset hash differs: $name" }
}
function Read-Api([string]$Endpoint) {
    $json = & gh api $Endpoint
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace(($json -join "`n"))) {
        throw 'GitHub API identity read failed.'
    }
    return ($json -join "`n" | ConvertFrom-Json -Depth 25)
}
function Download-ExactArtifact([long]$Id, [long]$RunId, [string]$Commit,
    [string]$Name, [string]$Digest, [string]$Destination) {
    $run = Read-Api "repos/taiyun668/gogoke/actions/runs/$RunId"
    if ($run.id -ne $RunId -or $run.head_sha -cne $Commit -or
        $run.repository.full_name -cne 'taiyun668/gogoke' -or
        $run.path -cne '.github/workflows/gogoke-desktop.yml' -or
        $run.name -cne 'gogoke desktop CI' -or $run.run_attempt -ne 1 -or
        $run.status -cne 'completed' -or $run.conclusion -cne 'success') {
        throw 'Frozen source run identity is not successful and exact.'
    }
    $artifact = Read-Api "repos/taiyun668/gogoke/actions/artifacts/$Id"
    if ($artifact.id -ne $Id -or $artifact.name -cne $Name -or
        $artifact.workflow_run.id -ne $RunId -or
        $artifact.workflow_run.head_sha -cne $Commit -or $artifact.expired -or
        $artifact.digest -cne "sha256:$Digest" -or
        $artifact.size_in_bytes -le 0 -or $artifact.size_in_bytes -gt 1073741824) {
        throw 'Frozen artifact identity or uploaded digest differs.'
    }
    if (Test-Path -LiteralPath $Destination) { throw 'Frozen extraction destination already exists.' }
    New-Item -ItemType Directory -Path $Destination | Out-Null
    $archivePath = Join-Path $work "$Id.zip"
    Invoke-WebRequest -Uri "https://api.github.com/repos/taiyun668/gogoke/actions/artifacts/$Id/zip" `
        -Headers @{ Authorization = "Bearer $env:GH_TOKEN"; Accept = 'application/vnd.github+json' } `
        -MaximumRedirection 5 -OutFile $archivePath
    if ((Get-Item -LiteralPath $archivePath).Length -ne $artifact.size_in_bytes -or
        (Hash $archivePath) -cne $Digest) { throw 'Downloaded artifact differs from GitHub digest.' }
    Add-Type -AssemblyName System.IO.Compression
    $archive = [IO.Compression.ZipFile]::OpenRead($archivePath)
    try {
        $names = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $name = $entry.FullName
            if ([string]::IsNullOrEmpty($name) -or $name -in @('.', '..') -or
                $name.Contains('/') -or $name.Contains('\') -or -not $names.Add($name) -or
                $entry.Length -gt 1073741824) {
                throw 'Frozen archive has a non-flat or duplicate entry.'
            }
            $outputPath = Join-Path $Destination $name
            $input = $entry.Open()
            try {
                $output = [IO.File]::Open($outputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
                try { $input.CopyTo($output) } finally { $output.Dispose() }
            } finally { $input.Dispose() }
            if ((Get-Item -LiteralPath $outputPath).Length -ne $entry.Length) {
                throw 'Extracted asset length differs from artifact entry.'
            }
        }
    } finally { $archive.Dispose() }
}
function Verify-Manifest([string]$ManifestPath, [string]$SignatureHex, [string]$ExpectedHash,
    [string]$ExpectedSignatureHash, [string]$Version, [string]$Kind, [string]$FrozenDir,
    [string]$PortableDir, [string]$ReleaseDir, [Security.Cryptography.ECDsa]$Verifier) {
    Require-Hash $ManifestPath $ExpectedHash
    if ($SignatureHex -cnotmatch '^[0-9a-f]{128}$') { throw 'Formal signature has invalid shape.' }
    $bytes = [IO.File]::ReadAllBytes($ManifestPath)
    if (-not $Verifier.VerifyData($bytes, [Convert]::FromHexString($SignatureHex),
            [Security.Cryptography.HashAlgorithmName]::SHA256)) {
        throw 'Formal signature does not verify against compiled Owner public key.'
    }
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    if ($text.Contains("`r") -or -not $text.EndsWith("`n", [StringComparison]::Ordinal) -or
        $text.EndsWith("`n`n", [StringComparison]::Ordinal)) {
        throw 'Formal manifest is not exact ASCII LF.'
    }
    $lines = @($text.Substring(0, $text.Length - 1).Split("`n"))
    if ($lines.Count -ne $(if ($Kind -ceq 'full') { 6 } else { 4 }) -or
        $lines[0] -cne "# gogoke-Version: $Version" -or
        $lines[1] -cne "# gogoke-Release-Type: $Kind") {
        throw 'Formal manifest headers or entry count differ.'
    }
    $expectedNames = @('resource-index.json', 'gogoke-resources.windows.zip')
    if ($Kind -ceq 'full') {
        $expectedNames += "gogoke-$Version-windows-x64-unsigned-setup.exe"
        $expectedNames += "gogoke-$Version-windows-x64-unsigned-portable.zip"
    }
    New-Item -ItemType Directory -Path $ReleaseDir | Out-Null
    for ($i = 0; $i -lt $expectedNames.Count; $i++) {
        $entry = [regex]::Match($lines[$i + 2], '^([0-9a-f]{64})  ([A-Za-z0-9._+-]+)$')
        if (-not $entry.Success -or $entry.Groups[2].Value -cne $expectedNames[$i]) {
            throw 'Formal asset line differs.'
        }
        $assetName = $entry.Groups[2].Value
        $assetHash = $entry.Groups[1].Value
        $path = Join-Path $FrozenDir $assetName
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { $path = Join-Path $PortableDir $assetName }
        Require-Hash $path $assetHash
        Copy-Item -LiteralPath $path -Destination (Join-Path $ReleaseDir $assetName)
    }
    Copy-Item -LiteralPath $ManifestPath -Destination (Join-Path $ReleaseDir 'SHA256SUMS.windows')
    [IO.File]::WriteAllText((Join-Path $ReleaseDir 'SHA256SUMS.windows.sig'),
        "$SignatureHex`n", [Text.Encoding]::ASCII)
    Require-Hash (Join-Path $ReleaseDir 'SHA256SUMS.windows.sig') $ExpectedSignatureHash
    Require-Hash (Join-Path $ReleaseDir 'SHA256SUMS.windows') $ExpectedHash
    & python (Join-Path $repoRoot 'tools\ci\gogoke_resource_pack.py') verify `
        --pack (Join-Path $ReleaseDir 'gogoke-resources.windows.zip') `
        --index (Join-Path $ReleaseDir 'resource-index.json')
    if ($LASTEXITCODE -ne 0) { throw 'Signed resource pack/index verification failed.' }
}

if ($Mode -ceq 'Preflight') {
    if ($SourceCommit -cne $fullSource -or $SourceRunId -ne 36438821931 -or
        $SourceRunAttempt -ne 1 -or $SourceArtifactId -ne 10979163584 -or
        [string]::IsNullOrWhiteSpace($env:GH_TOKEN) -or
        [string]::IsNullOrWhiteSpace($env:GOGOKE_R206B_FORMAL_SIGNATURES) -or
        (Test-Path -LiteralPath $work)) {
        throw 'Formal smoke preflight identity or fresh root differs.'
    }
    New-Item -ItemType Directory -Path $work | Out-Null
    $fullFrozen = Join-Path $work 'full-frozen'
    $fullPortable = Join-Path $work 'full-portable'
    $resourcesFrozen = Join-Path $work 'resources-frozen'
    Download-ExactArtifact 10979163584 36438821931 $fullSource `
        "gogoke-windows-frozen-$fullSource-36438821931" `
        '630eedf6965de50a951a3413e74cf0d4f7e11f462375ed46ca3f69ef66fa7c17' $fullFrozen
    Download-ExactArtifact 10979418352 36438821931 $fullSource `
        "gogoke-windows-portable-$fullSource-36438821931" `
        '080b348ed5d6ad04b6bbeec3edc84eb8e191496c407f399389de3aa4356e1907' $fullPortable
    Download-ExactArtifact 10971640782 36421999351 $resourcesSource `
        "gogoke-windows-frozen-$resourcesSource-36421999351" `
        '54a766725f2b8c6f5d2ffb4c6d1e452bc97a76ca2a4b8ee7ed9e8cbeb3349881' $resourcesFrozen
    & python (Join-Path $repoRoot 'tools\ci\gogoke_ci_frozen_artifact.py') verify `
        --directory $fullFrozen --source-commit $fullSource --run-id 36438821931 --run-attempt 1 --lane frozen
    if ($LASTEXITCODE -ne 0) { throw 'Full frozen inventory verification failed.' }
    & python (Join-Path $repoRoot 'tools\ci\gogoke_ci_frozen_artifact.py') verify `
        --directory $resourcesFrozen --source-commit $resourcesSource --run-id 36421999351 --run-attempt 1 --lane frozen
    if ($LASTEXITCODE -ne 0) { throw 'Resources frozen inventory verification failed.' }
    $signatures = $env:GOGOKE_R206B_FORMAL_SIGNATURES | ConvertFrom-Json
    $pubText = [IO.File]::ReadAllText((Join-Path $repoRoot 'apps\desktop\src-tauri\gogoke-release-public-key.txt'),
        [Text.Encoding]::ASCII).Trim()
    if ($pubText -cnotmatch '^[0-9a-fA-F]{128}$') { throw 'Compiled Owner public key shape differs.' }
    $point = [Security.Cryptography.ECPoint]::new()
    $point.X = [Convert]::FromHexString($pubText.Substring(0, 64))
    $point.Y = [Convert]::FromHexString($pubText.Substring(64, 64))
    $parameters = [Security.Cryptography.ECParameters]::new()
    $parameters.Curve = [Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
    $parameters.Q = $point
    $verifier = [Security.Cryptography.ECDsa]::Create($parameters)
    $inputRoot = Join-Path $repoRoot 'artifacts\s1-r4\release-inputs\R2-06b'
    Verify-Manifest (Join-Path $inputRoot 'full-0.1.11\SHA256SUMS.windows.input.md') `
        ([string]$signatures.full) $fullManifestHash $fullSignatureHash '0.1.11' 'full' `
        $fullFrozen $fullPortable (Join-Path $work 'full-release') $verifier
    Verify-Manifest (Join-Path $inputRoot 'resources-0.1.10\SHA256SUMS.windows.input.md') `
        ([string]$signatures.resources) $resourcesManifestHash $resourcesSignatureHash '0.1.10' 'resources' `
        $resourcesFrozen $fullPortable (Join-Path $work 'resources-release') $verifier
    Write-Output 'PREFLIGHT_PASS exact frozen assets and both Owner-public-key formal signatures'
    exit 0
}

if (-not (Test-Path -LiteralPath (Join-Path $work 'full-release') -PathType Container) -or
    -not (Test-Path -LiteralPath (Join-Path $work 'resources-release') -PathType Container)) {
    throw 'Formal smoke did not receive both exact preflight release sets.'
}
if ([string]::IsNullOrWhiteSpace($env:GH_TOKEN)) {
    throw 'Formal product cloud GitHub read credential unavailable.'
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
try {
    if ([Security.Principal.WindowsPrincipal]::new($identity).IsInRole(
            [Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Formal installed smoke must run under a Medium non-administrator token.'
    }
} finally { $identity.Dispose() }
$resultPath = Join-Path $work 'formal-smoke-result.json'
if (Test-Path -LiteralPath $resultPath) { throw 'Formal smoke result already exists.' }
$result = [ordered]@{
    schema = 'gogoke.r2-06b.formal-signed-installed-smoke.v1'
    state = 'RUNNING'
    sourceCommit = $fullSource
    fullSourceRunId = 36438821931
    resourcesSourceRunId = 36421999351
    fullManifestSha256 = $fullManifestHash
    resourcesManifestSha256 = $resourcesManifestHash
    fullSignatureSha256 = $fullSignatureHash
    resourcesSignatureSha256 = $resourcesSignatureHash
    formalDynamicResourceUpdate = 'NOT_RUN_NO_PUBLIC_RELEASE'
}
try {
    $fullDir = Join-Path $work 'full-release'
    $setup = Join-Path $fullDir 'gogoke-0.1.11-windows-x64-unsigned-setup.exe'
    Require-Hash $setup $setupHash
    Require-Hash (Join-Path $fullDir 'SHA256SUMS.windows') $fullManifestHash
    Require-Hash (Join-Path $fullDir 'SHA256SUMS.windows.sig') $fullSignatureHash
    $install = Join-Path $work 'formal-install'
    if (Test-Path -LiteralPath $install) { throw 'Formal install target already exists.' }
    $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke'
    if (Test-Path -LiteralPath $key) { throw 'Cloud runner already has formal registration.' }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $setup
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    foreach ($argument in @('/S', "/D=$install")) { [void]$start.ArgumentList.Add($argument) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw 'Formal setup did not start.' }
    if (-not $process.WaitForExit(900000)) { throw 'Formal setup exceeded its 15-minute bound.' }
    if ($process.ExitCode -ne 0) { throw "Formal setup returned $($process.ExitCode)." }
    $registration = Get-ItemProperty -LiteralPath $key -ErrorAction Stop
    if ($registration.InstallLocation -cne $install -or
        $registration.InstallDomain -cne 'OWNER_RELEASE' -or
        $registration.DisplayVersion -cne '0.1.11' -or
        [string]::IsNullOrWhiteSpace([string]$registration.InstallInstanceId)) {
        throw 'Formal installed registration identity differs.'
    }
    if (Test-Path -LiteralPath (Join-Path $install 'uninstall.exe')) {
        throw 'Formal installation unexpectedly created uninstall.exe.'
    }
    Require-Hash (Join-Path $install 'gogoke.exe') $installedShellHash
    Require-Hash (Join-Path $install 'gogoke-native-host.exe') $nativeHostHash
    Require-Hash (Join-Path $install 'gogoke-service\runtime\node.exe') $nodeHash
    Require-Hash (Join-Path $install 'SHA256SUMS.windows') $fullManifestHash
    Require-Hash (Join-Path $install 'SHA256SUMS.windows.sig') $fullSignatureHash
    $result.installedShellSha256 = $installedShellHash
    $result.nativeHostSha256 = $nativeHostHash
    $result.nodeSha256 = $nodeHash
    $result.installExitCode = 0
    $result.registrationDomain = 'OWNER_RELEASE'
    $indexPath = Join-Path $install 'resource-index.json'
    $indexHash = Hash $indexPath
    $index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
    if ($index.schema -cne 'gogoke.resource-index.v1' -or $index.sourceCommit -cne $fullSource -or
        $index.version -cne '0.1.11' -or [string]::IsNullOrWhiteSpace([string]$index.generationId)) {
        throw 'Formal installed resource index identity differs.'
    }
    $evidencePath = Join-Path $env:RUNNER_TEMP "gogoke-r2-06-service-$env:GITHUB_RUN_ID-$env:GITHUB_RUN_ATTEMPT.json"
    if (Test-Path -LiteralPath $evidencePath) { throw 'Formal service evidence path exists.' }
    $data = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)) 'app.gogoke.desktop'
    & (Join-Path $install 'gogoke-service\runtime\node.exe') `
        (Join-Path $repoRoot 'tools\ci\gogoke-package-service.mjs') `
        formal-installed-service $install ([string]$index.generationId) $fullSource '0.1.11' `
        $evidencePath (Join-Path $data 'product-authority')
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $evidencePath -PathType Leaf)) {
        throw 'Actual formal installed generation-aware product smoke failed.'
    }
    $product = Get-Content -LiteralPath $evidencePath -Raw | ConvertFrom-Json
    if ($product.schema -cne 'gogoke.r2-06b.formal-service-smoke.v1' -or
        $product.state -cne 'PASS' -or $product.platform -cne 'WINDOWS_CLOUD_NOT_OWNER_WIN11' -or
        $product.sourceCommit -cne $fullSource -or $product.generationId -cne $index.generationId -or
        $product.smokeRunId -cne [string]$env:GITHUB_RUN_ID -or
        $product.smokeRunAttempt -cne [string]$env:GITHUB_RUN_ATTEMPT -or
        $product.installedShellSha256 -cne $installedShellHash -or
        $product.positive.state -cne 'VALIDATED_TEST_RESULT_NOT_ADOPTED' -or
        $product.positive.readinessSetId -cne $indexHash -or
        $product.positive.adoption -cne $false -or
        -not ([string]$product.negative.rejection).StartsWith('GOGOKE_PRODUCT_SERVICE_FAILED:78:STDERR_TAIL:GOGOKE_MODULE_NOT_LISTED', [StringComparison]::Ordinal) -or
        $product.negative.poisonExecuted -cne $false -or
        $product.negative.poisonRemoved -cne $true -or
        $product.negative.adoption -cne $false -or $product.release -cne $false) {
        throw 'Formal installed product smoke evidence differs.'
    }
    $result.productState = 'PASS_TEST_ONLY'
    $result.adoption = $false
    $result.release = $false
    $result.productServiceEvidenceSha256 = Hash $evidencePath
    $readyPath = [IO.Path]::Combine([string]$env:TMP,
        ('gogoke-update-r2-06-' + [Guid]::NewGuid().ToString('N') + '.ready'))
    if (Test-Path -LiteralPath $readyPath) { throw 'Formal readiness path exists.' }
    $launch = [Diagnostics.ProcessStartInfo]::new()
    $launch.FileName = Join-Path $install 'gogoke.exe'
    $launch.UseShellExecute = $false
    $launch.WindowStyle = [Diagnostics.ProcessWindowStyle]::Normal
    [void]$launch.ArgumentList.Add("--gogoke-update-ready=$readyPath")
    $app = [Diagnostics.Process]::new()
    $app.StartInfo = $launch
    if (-not $app.Start()) { throw 'Formal installed product did not launch.' }
    $readyDeadline = [DateTime]::UtcNow.AddSeconds(45)
    while ([DateTime]::UtcNow -lt $readyDeadline) {
        if (Test-Path -LiteralPath $readyPath -PathType Leaf) { break }
        if ($app.HasExited) { throw 'Formal installed product exited before readiness.' }
        Start-Sleep -Milliseconds 250
    }
    if (-not (Test-Path -LiteralPath $readyPath -PathType Leaf)) {
        throw 'Formal installed product did not publish bootstrap readiness.'
    }
    $ready = Get-Content -LiteralPath $readyPath -Raw | ConvertFrom-Json
    if ($ready.version -cne '0.1.11' -or $ready.generationId -cne $index.generationId -or
        $ready.setId -cne $indexHash) { throw 'Formal installed bootstrap readiness identity differs.' }
    $result.bootstrapReadiness = [ordered]@{ version = $ready.version; generationId = $ready.generationId; setId = $ready.setId }
    [void]$app.CloseMainWindow()
    if (-not $app.WaitForExit(15000)) { throw 'Formal installed product did not exit after close request.' }
    $dataBefore = @{}
    if (Test-Path -LiteralPath $data -PathType Container) {
        foreach ($item in @(Get-ChildItem -LiteralPath $data -Recurse -File -Force)) {
            $dataBefore[$item.FullName.Substring($data.Length + 1)] = Hash $item.FullName
        }
    }
    $instanceHash = [Security.Cryptography.SHA256]::HashData(
        [Text.Encoding]::UTF8.GetBytes([string]$registration.InstallInstanceId))
    $tag = ([Convert]::ToHexString($instanceHash).ToLowerInvariant()).Substring(0, 16)
    $existing = @(Get-ChildItem -LiteralPath $work -Filter "gogoke-uninstall-$tag-*.json" -File -ErrorAction SilentlyContinue |
        Select-Object -ExpandProperty Name)
    $uninstall = [Diagnostics.ProcessStartInfo]::new()
    $uninstall.FileName = Join-Path $install 'gogoke.exe'
    $uninstall.UseShellExecute = $false
    $uninstall.CreateNoWindow = $true
    foreach ($argument in @('--uninstall', '--quiet')) { [void]$uninstall.ArgumentList.Add($argument) }
    $parent = [Diagnostics.Process]::new()
    $parent.StartInfo = $uninstall
    if (-not $parent.Start()) { throw 'Formal uninstaller did not start.' }
    if (-not $parent.WaitForExit(30000) -or $parent.ExitCode -ne 0) {
        throw 'Formal uninstaller parent did not exit cleanly.'
    }
    $deadline = [DateTime]::UtcNow.AddMinutes(6)
    $finalizer = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        $found = @(Get-ChildItem -LiteralPath $work -Filter "gogoke-uninstall-$tag-*.json" -File -ErrorAction SilentlyContinue |
            Where-Object { $existing -cnotcontains $_.Name })
        if ($found.Count -gt 1) { throw 'Formal finalizer receipt is ambiguous.' }
        if ($found.Count -eq 1) {
            try { $candidate = Get-Content -LiteralPath $found[0].FullName -Raw | ConvertFrom-Json }
            catch { Start-Sleep -Milliseconds 250; continue }
            if ($candidate.state -ceq 'FAILED') { throw 'Formal finalizer reported failure.' }
            if ($candidate.state -ceq 'DELETED') { $finalizer = $candidate; break }
        }
        Start-Sleep -Milliseconds 250
    }
    if ($null -eq $finalizer -or $finalizer.domain -cne 'OWNER_RELEASE' -or
        (Test-Path -LiteralPath $key)) { throw 'Formal uninstall did not settle.' }
    $dataAfter = @{}
    if (Test-Path -LiteralPath $data -PathType Container) {
        foreach ($item in @(Get-ChildItem -LiteralPath $data -Recurse -File -Force)) {
            $dataAfter[$item.FullName.Substring($data.Length + 1)] = Hash $item.FullName
        }
    }
    if ($dataBefore.Count -ne $dataAfter.Count) { throw 'Formal data file count changed during uninstall.' }
    foreach ($name in $dataBefore.Keys) {
        if (-not $dataAfter.ContainsKey($name) -or $dataBefore[$name] -cne $dataAfter[$name]) {
            throw 'Formal data bytes changed during uninstall.'
        }
    }
    $result.uninstallState = 'DELETED'
    $result.dataPreservedFileCount = $dataAfter.Count
    $result.state = 'PASS'
} catch {
    $result.state = 'FAIL'
    $result.error = [string]$_.Exception.Message
} finally {
    [IO.File]::WriteAllText($resultPath, ($result | ConvertTo-Json -Depth 8) + "`n",
        [Text.UTF8Encoding]::new($false))
}
if ($result.state -cne 'PASS') { throw 'Formal signed installed smoke failed; see bounded receipt.' }
Write-Output 'FORMAL_SIGNED_SMOKE_PASS exact frozen full install, product and uninstall; resources signature verified'
