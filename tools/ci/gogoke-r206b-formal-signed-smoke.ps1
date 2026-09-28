# Formal-signature smoke on the exact frozen R2-06b bytes. Cloud Windows only.
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
$fullSource = '1ffda964807d257c0bc9fddfec03023642395b9c'
$resourcesSource = '309d7f4dad924173e95e52971029e4bf92baa9ac'
$fullManifestHash = '33d8edb752d1ea09e9e087e5cf21e505093e5d85743419e17f90ce275802c85e'
$resourcesManifestHash = '8ff4d4803a949306bc46d85890db740d3c71966e52de3f6dc9de46fb6f180641'
$fullSignatureHash = '3c8978b0cf7e2d568253d5e02ca0594a5cbc82f0dd8cf954f35335c96f5f0444'
$resourcesSignatureHash = 'bf15f250a0eb617c5c36277b39a9d9ce4e498e8750d575148b332e9e76a5fa84'
$setupHash = 'bdf9b6e000a8874dd7672fda8d5d83543b82f93032d654e6ab393427208c1e2c'
$installedShellHash = '0bfe0785f0764522ecb853903885fbea0ec8d082f37e21fa81ad1dd523f74085'
$nativeHostHash = '842221cb902906e19b439123a638c50f80432edebed0788b75182e7a42c9d267'
$nodeHash = 'e3be0545990c90995d7bf3a7af5d64af1f2e0fc1bbd9b79c27f7abc1e9676e50'

function Hash([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Require-Hash([string]$Path, [string]$Expected) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf) -or (Hash $Path) -cne $Expected) {
        throw 'Exact formal smoke asset hash differs.'
    }
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
function Verify-Manifest([string]$Input, [string]$SignatureHex, [string]$ExpectedHash,
    [string]$ExpectedSignatureHash, [string]$Version, [string]$Kind, [string]$FrozenDir,
    [string]$PortableDir, [string]$ReleaseDir, [Security.Cryptography.ECDsa]$Verifier) {
    Require-Hash $Input $ExpectedHash
    if ($SignatureHex -cnotmatch '^[0-9a-f]{128}$') { throw 'Formal signature has invalid shape.' }
    $bytes = [IO.File]::ReadAllBytes($Input)
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
        if ($lines[$i + 2] -cnotmatch '^([0-9a-f]{64})  ([A-Za-z0-9._+-]+)$' -or
            $Matches[2] -cne $expectedNames[$i]) { throw 'Formal asset line differs.' }
        $path = Join-Path $FrozenDir $Matches[2]
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { $path = Join-Path $PortableDir $Matches[2] }
        Require-Hash $path $Matches[1]
        Copy-Item -LiteralPath $path -Destination (Join-Path $ReleaseDir $Matches[2])
    }
    Copy-Item -LiteralPath $Input -Destination (Join-Path $ReleaseDir 'SHA256SUMS.windows')
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
    if ($SourceCommit -cne $fullSource -or $SourceRunId -ne 36359270160 -or
        $SourceRunAttempt -ne 1 -or $SourceArtifactId -ne 10945084785 -or
        [string]::IsNullOrWhiteSpace($env:GH_TOKEN) -or
        [string]::IsNullOrWhiteSpace($env:GOGOKE_R206B_FORMAL_SIGNATURES) -or
        (Test-Path -LiteralPath $work)) {
        throw 'Formal smoke preflight identity or fresh root differs.'
    }
    New-Item -ItemType Directory -Path $work | Out-Null
    $fullFrozen = Join-Path $work 'full-frozen'
    $fullPortable = Join-Path $work 'full-portable'
    $resourcesFrozen = Join-Path $work 'resources-frozen'
    Download-ExactArtifact 10945084785 36359270160 $fullSource `
        "gogoke-windows-frozen-$fullSource-36359270160" `
        'ce7024880031a2e749c503105aa9561b78df6f7f0d5f5e76d0598c2f82b9d783' $fullFrozen
    Download-ExactArtifact 10945673352 36359270160 $fullSource `
        "gogoke-windows-portable-$fullSource-36359270160" `
        '675582db8e6f7a92c1e5e073906f6608efe68f1b80a63053398e218faed7f79a' $fullPortable
    Download-ExactArtifact 10947590712 36363049008 $resourcesSource `
        "gogoke-windows-frozen-$resourcesSource-36363049008" `
        'f3740cba607db1cc523ed5b4e7e2fd484a2b4c8e233394e31ced87d32dd62eba' $resourcesFrozen
    & python (Join-Path $repoRoot 'tools\ci\gogoke_ci_frozen_artifact.py') verify `
        --directory $fullFrozen --source-commit $fullSource --run-id 36359270160 --run-attempt 1 --lane frozen
    if ($LASTEXITCODE -ne 0) { throw 'Full frozen inventory verification failed.' }
    & python (Join-Path $repoRoot 'tools\ci\gogoke_ci_frozen_artifact.py') verify `
        --directory $resourcesFrozen --source-commit $resourcesSource --run-id 36363049008 --run-attempt 1 --lane frozen
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
    Verify-Manifest (Join-Path $inputRoot 'full-0.1.3\SHA256SUMS.windows.input.md') `
        ([string]$signatures.full) $fullManifestHash $fullSignatureHash '0.1.3' 'full' `
        $fullFrozen $fullPortable (Join-Path $work 'full-release') $verifier
    Verify-Manifest (Join-Path $inputRoot 'resources-0.1.4\SHA256SUMS.windows.input.md') `
        ([string]$signatures.resources) $resourcesManifestHash $resourcesSignatureHash '0.1.4' 'resources' `
        $resourcesFrozen $fullPortable (Join-Path $work 'resources-release') $verifier
    Write-Output 'PREFLIGHT_PASS exact frozen assets and both Owner-public-key formal signatures'
    exit 0
}

if (-not (Test-Path -LiteralPath (Join-Path $work 'full-release') -PathType Container) -or
    -not (Test-Path -LiteralPath (Join-Path $work 'resources-release') -PathType Container)) {
    throw 'Formal smoke did not receive both exact preflight release sets.'
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
    fullSourceRunId = 36359270160
    resourcesSourceRunId = 36363049008
    fullManifestSha256 = $fullManifestHash
    resourcesManifestSha256 = $resourcesManifestHash
    fullSignatureSha256 = $fullSignatureHash
    resourcesSignatureSha256 = $resourcesSignatureHash
    formalDynamicResourceUpdate = 'NOT_RUN_NO_PUBLIC_RELEASE'
}
try {
    $fullDir = Join-Path $work 'full-release'
    $setup = Join-Path $fullDir 'gogoke-0.1.3-windows-x64-unsigned-setup.exe'
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
        $registration.DisplayVersion -cne '0.1.3' -or
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
    $receipt = Join-Path $env:RUNNER_TEMP ('gogoke-r2-smoke-receipt-' + [Guid]::NewGuid().ToString('N') + '.json')
    if (Test-Path -LiteralPath $receipt) { throw 'Product smoke receipt path exists.' }
    $env:GOGOKE_R2_SMOKE_RECEIPT = $receipt
    try {
        & node (Join-Path $repoRoot 'tools\ci\gogoke-package-service.mjs') smoke $install
        if ($LASTEXITCODE -ne 0) { throw 'Actual formal installed product smoke failed.' }
    } finally { Remove-Item Env:GOGOKE_R2_SMOKE_RECEIPT -ErrorAction SilentlyContinue }
    $product = Get-Content -LiteralPath $receipt -Raw | ConvertFrom-Json
    if ($product.schema -cne 'gogoke.r2-04.installed-smoke-receipt.v1' -or
        $product.state -cne 'PASS' -or $product.installedSmoke.adoption -ne $false -or
        $product.installedSmoke.release -ne $false) {
        throw 'Formal product smoke result is not exact PASS test-only.'
    }
    $result.productState = 'PASS_TEST_ONLY'
    $result.adoption = $false
    $result.release = $false
    $data = Join-Path ([Environment]::GetFolderPath([Environment+SpecialFolder]::ApplicationData)) 'app.gogoke.desktop'
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
