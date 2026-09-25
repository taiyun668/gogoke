[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArtifactDirectory,
    [Parameter(Mandatory = $true)][string]$SourceCommit,
    [Parameter(Mandatory = $true)][long]$RunId,
    [Parameter(Mandatory = $true)][int]$RunAttempt,
    [Parameter(Mandatory = $true)][long]$ArtifactId
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_REF -cne 'refs/heads/main' -or $env:GITHUB_REPOSITORY -cne 'taiyun668/gogoke') {
    throw 'Candidate signing requires the trusted default-branch workflow.'
}
if ($SourceCommit -cnotmatch '^[0-9a-f]{40}$' -or $RunId -le 0 -or $RunAttempt -le 0 -or $ArtifactId -le 0) {
    throw 'Candidate signing source identity is malformed.'
}
$artifactRoot = (Resolve-Path -LiteralPath $ArtifactDirectory).Path
$indexPath = Join-Path $artifactRoot 'resource-index.json'
$packPath = Join-Path $artifactRoot 'gogoke-resources.windows.zip'
foreach ($path in @($indexPath, $packPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Candidate resource input missing: $path" }
}

$run = gh api "repos/taiyun668/gogoke/actions/runs/$RunId" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $run.repository.full_name -cne 'taiyun668/gogoke' -or
    $run.head_repository.full_name -cne 'taiyun668/gogoke' -or
    $run.head_branch -cne 'gpt/s1-r4-r2-execution-r1' -or
    $run.head_sha -cne $SourceCommit -or $run.run_attempt -ne $RunAttempt -or
    $run.conclusion -cne 'success' -or $run.name -cne 'gogoke desktop CI') {
    throw 'Candidate resource artifact did not come from the successful controlled source run.'
}
$artifactList = gh api "repos/taiyun668/gogoke/actions/runs/$RunId/artifacts" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Candidate source artifact inventory unavailable.' }
$expectedName = "gogoke-windows-frozen-$SourceCommit-$RunId"
$matches = @($artifactList.artifacts | Where-Object {
    $_.id -eq $ArtifactId -and $_.name -ceq $expectedName -and -not $_.expired -and $_.size_in_bytes -gt 0
})
if ($matches.Count -ne 1) { throw 'Candidate source artifact identity is ambiguous or unavailable.' }

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
python (Join-Path $repoRoot 'tools\ci\gogoke_resource_pack.py') verify --pack $packPath --index $indexPath
if ($LASTEXITCODE -ne 0) { throw 'Trusted resource pack verifier rejected candidate bytes.' }
$index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
if ($index.schema -cne 'gogoke.resource-index.v1' -or $index.sourceCommit -cne $SourceCommit -or
    $index.pack.fileName -cne 'gogoke-resources.windows.zip') {
    throw 'Candidate index identity does not match the controlled source run.'
}
$version = [string]$index.version
if ($version -cnotmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw 'Candidate version is not SemVer.'
}
$manifestPath = Join-Path $artifactRoot 'CANDIDATE-RESOURCES.windows'
if (Test-Path -LiteralPath $manifestPath) { throw 'Candidate manifest already exists; refusing overwrite.' }
$indexHash = (Get-FileHash -LiteralPath $indexPath -Algorithm SHA256).Hash.ToLowerInvariant()
$packHash = (Get-FileHash -LiteralPath $packPath -Algorithm SHA256).Hash.ToLowerInvariant()
$lines = @(
    '# gogoke-Candidate-Purpose: CI_CANDIDATE_RESOURCE'
    '# gogoke-Test-Only: true'
    '# gogoke-Repository: taiyun668/gogoke'
    "# gogoke-Source-Commit: $SourceCommit"
    "# gogoke-Run-Id: $RunId"
    "# gogoke-Run-Attempt: $RunAttempt"
    "# gogoke-Artifact-Id: $ArtifactId"
    "# gogoke-Version: $version"
    "$indexHash  resource-index.json"
    "$packHash  gogoke-resources.windows.zip"
)
$manifestBytes = [Text.Encoding]::ASCII.GetBytes(($lines -join "`n") + "`n")

function ConvertFrom-Hex([string]$text) {
    if ($text -cnotmatch '^[0-9A-Fa-f]{64}$') { throw 'Candidate key component has invalid shape.' }
    return ,[Convert]::FromHexString($text)
}
$keyLines = @($env:GOGOKE_CANDIDATE_P256_KEY -split '\r?\n' | Where-Object { $_.Trim() })
if ($keyLines.Count -ne 3) { throw 'Candidate signing secret is unavailable or malformed.' }
$parameters = [Security.Cryptography.ECParameters]::new()
$parameters.Curve = [Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
$parameters.D = ConvertFrom-Hex $keyLines[0].Trim()
$point = [Security.Cryptography.ECPoint]::new()
$point.X = ConvertFrom-Hex $keyLines[1].Trim()
$point.Y = ConvertFrom-Hex $keyLines[2].Trim()
$parameters.Q = $point
$public = ([Convert]::ToHexString($point.X) + [Convert]::ToHexString($point.Y)).ToLowerInvariant()
$expectedPublic = ([IO.File]::ReadAllText((Join-Path $repoRoot 'apps\desktop\src-tauri\gogoke-candidate-public-key.txt'))).Trim()
if ($public -cne $expectedPublic) { throw 'Candidate secret does not match the compiled public key.' }
$key = [Security.Cryptography.ECDsa]::Create($parameters)
try {
    $prefix = [Text.Encoding]::ASCII.GetBytes("GOGOKE-CI-CANDIDATE-RESOURCE-V1`0")
    $payload = New-Object byte[] ($prefix.Length + $manifestBytes.Length)
    [Array]::Copy($prefix, 0, $payload, 0, $prefix.Length)
    [Array]::Copy($manifestBytes, 0, $payload, $prefix.Length, $manifestBytes.Length)
    $signature = $key.SignData($payload, [Security.Cryptography.HashAlgorithmName]::SHA256)
    [IO.File]::WriteAllBytes($manifestPath, $manifestBytes)
    [IO.File]::WriteAllText("$manifestPath.sig", ([Convert]::ToHexString($signature)).ToLowerInvariant() + "`n", [Text.Encoding]::ASCII)
} finally {
    $key.Dispose()
    Remove-Variable keyLines, parameters, payload -ErrorAction SilentlyContinue
}
Write-Output 'Signed exact candidate-marked resource bytes.'
