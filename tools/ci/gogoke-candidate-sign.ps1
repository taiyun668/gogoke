[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArtifactDirectory,
    [Parameter(Mandatory = $true)][string]$SourceCommit,
    [Parameter(Mandatory = $true)][long]$RunId,
    [Parameter(Mandatory = $true)][int]$RunAttempt,
    [Parameter(Mandatory = $true)][long]$ArtifactId
)

$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:GITHUB_REF -cne 'refs/heads/main' -or
    $env:GITHUB_REPOSITORY -cne 'taiyun668/gogoke' -or
    $env:GITHUB_SHA -cnotmatch '^[0-9a-f]{40}$') {
    throw 'Candidate signing requires the trusted default-branch workflow.'
}
if ($SourceCommit -cnotmatch '^[0-9a-f]{40}$' -or $RunId -le 0 -or $RunAttempt -le 0 -or $ArtifactId -le 0) {
    throw 'Candidate signing source identity is malformed.'
}
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
if ((git -C $repoRoot rev-parse HEAD).Trim() -cne $env:GITHUB_SHA) {
    throw 'Candidate signer is not the exact trusted main checkout.'
}
$authorizationMain = 'b75250c3b9987b9b09a0fa068f986e2c482a1044'
$manifestBlob = '9d540cbac609b5ba21cb12e1640b375bcc980736'
$manifestPath = 'docs/design/gogoke-37-plan-v1/MANIFEST.json'
$receiptPath = 'artifacts/gogoke-37/intake/PUBLIC_AUTHORIZATION_RECEIPT.json'
if ((git -C $repoRoot rev-parse "HEAD:$manifestPath").Trim() -cne $manifestBlob) {
    throw 'Trusted signer main has a different design 37 manifest.'
}
$receipt = Get-Content -LiteralPath (Join-Path $repoRoot $receiptPath) -Raw | ConvertFrom-Json
if ($receipt.schema -cne 'gogoke.37.public-authorization.v1' -or
    $receipt.repository -cne 'taiyun668/gogoke' -or
    $receipt.plan.public_plan_manifest_blob -cne $manifestBlob) {
    throw 'Trusted signer main authorization receipt does not bind the approved plan.'
}
$receiptBlob = '671cb15b811048c883e6f2f2671ba8a15fc4b55c'
if ((git -C $repoRoot rev-parse "HEAD:$receiptPath").Trim() -cne $receiptBlob) {
    throw 'Trusted signer main public authorization receipt blob changed.'
}
foreach ($binding in @(@($manifestPath, $manifestBlob), @($receiptPath, $receiptBlob))) {
    $current = gh api "repos/taiyun668/gogoke/contents/$($binding[0])?ref=main" | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0 -or $current.type -cne 'file' -or $current.sha -cne $binding[1]) {
        throw 'Current main authorization changed before candidate signing.'
    }
}
$comparison = gh api "repos/taiyun668/gogoke/compare/$authorizationMain...$SourceCommit" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $comparison.status -cnotin @('ahead', 'identical') -or
    $comparison.merge_base_commit.sha -cne $authorizationMain) {
    throw 'Candidate source is not descended from the authorized main commit.'
}
$sourceManifest = gh api "repos/taiyun668/gogoke/contents/$manifestPath`?ref=$SourceCommit" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $sourceManifest.type -cne 'file' -or $sourceManifest.sha -cne $manifestBlob) {
    throw 'Candidate source has a different design 37 manifest.'
}
$sourceReceipt = gh api "repos/taiyun668/gogoke/contents/$receiptPath`?ref=$SourceCommit" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $sourceReceipt.type -cne 'file' -or $sourceReceipt.sha -cne $receiptBlob) {
    throw 'Candidate source has a different public authorization receipt.'
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
    $run.head_branch -cne 'codex/gogoke-37-l0' -or
    $run.head_sha -cne $SourceCommit -or $run.run_attempt -ne $RunAttempt -or
    $run.event -cne 'workflow_dispatch' -or $run.status -cne 'completed' -or
    $run.conclusion -cne 'success' -or $run.name -cne 'gogoke desktop CI' -or
    $run.path -cne '.github/workflows/gogoke-desktop.yml') {
    throw 'Candidate resource artifact did not come from the successful controlled source run.'
}
$artifactList = gh api "repos/taiyun668/gogoke/actions/runs/$RunId/artifacts" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $artifactList.total_count -gt 30) {
    throw 'Candidate source artifact inventory unavailable or unbounded.'
}
$expectedName = "gogoke-windows-frozen-$SourceCommit-$RunId"
$matches = @($artifactList.artifacts | Where-Object {
    $_.id -eq $ArtifactId -and $_.name -ceq $expectedName -and -not $_.expired -and $_.size_in_bytes -gt 0
})
if ($matches.Count -ne 1) { throw 'Candidate source artifact identity is ambiguous or unavailable.' }

python (Join-Path $repoRoot 'tools\ci\gogoke_resource_pack.py') verify --pack $packPath --index $indexPath
if ($LASTEXITCODE -ne 0) { throw 'Trusted resource pack verifier rejected candidate bytes.' }
$index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
if ($index.schema -cne 'gogoke.resource-index.v1' -or $index.sourceCommit -cne $SourceCommit -or
    $index.pack.fileName -cne 'gogoke-resources.windows.zip') {
    throw 'Candidate index identity does not match the controlled source run.'
}
$version = [string]$index.version
# gogoke_resource_pack.py verify already checks the complete SemVer shape,
# including a prerelease and build suffix on the same version.
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
