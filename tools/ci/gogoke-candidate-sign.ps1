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
$receiptBlob = '373de0bd083201c0e5aefa5634a769ccd7f42000'
$receiptPath = 'artifacts/gogoke-37/intake/PUBLIC_AUTHORIZATION_RECEIPT.json'
$planPath = 'docs/design/gogoke-37-plan-v1/PLAN.json'
$verifierPath = Join-Path $repoRoot 'docs\design\gogoke-37-plan-v1\verify_plan.py'
if ((git -C $repoRoot rev-parse "HEAD:$receiptPath").Trim() -cne $receiptBlob) {
    throw 'Trusted signer main public authorization receipt blob changed.'
}
$receipt = Get-Content -LiteralPath (Join-Path $repoRoot $receiptPath) -Raw | ConvertFrom-Json
if ($receipt.schema -cne 'gogoke.37.public-authorization.v2' -or
    $receipt.repository -cne 'taiyun668/gogoke' -or
    $receipt.plan.scope_digest -cnotmatch '^[0-9a-f]{64}$') {
    throw 'Trusted signer main authorization receipt is malformed.'
}
$mainDigest = & python $verifierPath --scope-digest (Join-Path $repoRoot $planPath)
if ($LASTEXITCODE -ne 0 -or @($mainDigest).Count -ne 1 -or $mainDigest -cne $receipt.plan.scope_digest) {
    throw 'Trusted signer main plan scope differs from the Owner authorization.'
}
$currentMain = gh api 'repos/taiyun668/gogoke/commits/main' | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $currentMain.sha -cne $env:GITHUB_SHA) {
    throw 'Trusted signer checkout is not current main.'
}
$introducing = $null
foreach ($commit in @(git -C $repoRoot log --first-parent --format=%H HEAD -- $receiptPath)) {
    if ((git -C $repoRoot rev-parse "${commit}:$receiptPath" 2>$null).Trim() -ceq $receiptBlob) {
        $introducing = $commit
        break
    }
}
if ($introducing -cnotmatch '^[0-9a-f]{40}$') { throw 'Receipt introduction is absent from main first-parent history.' }
$parents = @((git -C $repoRoot rev-list --parents -n 1 $introducing).Trim() -split ' ')
if ($parents.Count -ne 3 -or $parents[0] -cne $introducing -or
    (git -C $repoRoot rev-parse "$($parents[1]):$receiptPath" 2>$null).Trim() -ceq $receiptBlob) {
    throw 'Receipt was not introduced by a main merge commit.'
}
$mergedPulls = @(gh api "repos/taiyun668/gogoke/commits/$introducing/pulls" | ConvertFrom-Json)
if ($LASTEXITCODE -ne 0 -or $mergedPulls.Count -ne 1 -or
    $mergedPulls[0].merge_commit_sha -cne $introducing -or $mergedPulls[0].number -le 0) {
    throw 'Receipt merge has no unique associated pull request.'
}
$mergedPull = gh api "repos/taiyun668/gogoke/pulls/$($mergedPulls[0].number)" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or -not $mergedPull.merged -or
    $mergedPull.base.ref -cne 'main' -or $mergedPull.base.repo.full_name -cne 'taiyun668/gogoke' -or
    $mergedPull.merge_commit_sha -cne $introducing -or
    $mergedPull.merged_by.login -cne 'taiyun668' -or -not $mergedPull.merged_at) {
    throw 'Receipt merge is not the Owner-merged main pull request.'
}
$comparison = gh api "repos/taiyun668/gogoke/compare/$introducing...$SourceCommit" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $comparison.status -cnotin @('ahead', 'identical') -or
    $comparison.merge_base_commit.sha -cne $introducing) {
    throw 'Candidate source is not descended from the receipt-introducing main merge.'
}
$sourceReceipt = gh api "repos/taiyun668/gogoke/contents/$receiptPath`?ref=$SourceCommit" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $sourceReceipt.type -cne 'file' -or $sourceReceipt.sha -cne $receiptBlob) {
    throw 'Candidate source has a different public authorization receipt.'
}
$sourcePlan = gh api "repos/taiyun668/gogoke/contents/$planPath`?ref=$SourceCommit" | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $sourcePlan.type -cne 'file' -or $sourcePlan.encoding -cne 'base64' -or
    $sourcePlan.size -le 0 -or $sourcePlan.size -gt 1048576) {
    throw 'Candidate plan data is unavailable or unbounded.'
}
$planData = Join-Path $env:RUNNER_TEMP "gogoke-candidate-plan-$SourceCommit.json"
if (Test-Path -LiteralPath $planData) { throw 'Candidate plan temporary path already exists.' }
try {
    [IO.File]::WriteAllBytes($planData, [Convert]::FromBase64String($sourcePlan.content))
    $sourceDigest = & python $verifierPath --scope-digest $planData
    if ($LASTEXITCODE -ne 0 -or @($sourceDigest).Count -ne 1 -or $sourceDigest -cne $receipt.plan.scope_digest) {
        throw 'Candidate plan scope differs from the Owner authorization.'
    }
} finally { Remove-Item -LiteralPath $planData -Force -ErrorAction SilentlyContinue }
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
