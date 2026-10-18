[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$EvidenceRoot
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows') {
    throw 'Native Job mutation is cloud Windows CI only; no local native compile or test.'
}

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$sourcePath = Join-Path $repoRoot 'apps/desktop/native-host/src/process/windows.rs'
$manifest = Join-Path $repoRoot 'apps/desktop/native-host/Cargo.toml'
$testName = 'process::windows::tests::holder_gone_requires_job_close_to_remove_live_descendant_before_root_recovery'
$failureMarker = 'HOLDER_GONE_KILL_ON_CLOSE_DESCENDANT_SURVIVED'
$needle = '    limits.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;'
$original = [IO.File]::ReadAllBytes($sourcePath)
$source = [Text.Encoding]::UTF8.GetString($original)
$baselineHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash
if ([regex]::Matches($source, [regex]::Escape($needle)).Count -ne 1) {
    throw 'Exact production KILL_ON_JOB_CLOSE assignment must match once.'
}
$testStart = $source.IndexOf('#[cfg(test)]' + "`n" + 'mod tests {')
if ($testStart -lt 0) {
    $testStart = $source.IndexOf('#[cfg(test)]' + "`r`n" + 'mod tests {')
}
if ($testStart -lt 0 -or $source.IndexOf($needle) -ge $testStart) {
    throw 'Mutation locator must belong to production code before the unchanged tests.'
}
$mutated = $source.Replace($needle, '    limits.basic_limit_information.limit_flags = 0;')
$offset = $mutated.IndexOf('#[cfg(test)]' + "`n" + 'mod tests {')
if ($offset -lt 0) { $offset = $mutated.IndexOf('#[cfg(test)]' + "`r`n" + 'mod tests {') }
if ($offset -lt 0 -or $source.Substring($testStart) -cne $mutated.Substring($offset)) {
    throw 'The complete test region must remain unchanged by mutation.'
}

New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null
$records = @()
function Invoke-ExactJobTest([string]$Phase, [bool]$ExpectFailure) {
    $output = & cargo test --locked --manifest-path $manifest $testName --lib -- --exact --nocapture 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    $output | Tee-Object -FilePath (Join-Path $EvidenceRoot "holder-gone-job-$Phase.log") | Out-Host
    $compiled = [regex]::IsMatch($text, 'Finished.*test.*profile')
    $executed = [regex]::IsMatch($text, '(?m)^running 1 test\s*$')
    $expectedSummary = if ($ExpectFailure) {
        'test result: FAILED\. 0 passed; 1 failed; 0 ignored;'
    } else {
        'test result: ok\. 1 passed; 0 failed; 0 ignored;'
    }
    $summaryMatches = [regex]::IsMatch($text, $expectedSummary)
    $behaviorMarker = $text.Contains($failureMarker)
    $valid = $compiled -and $executed -and $summaryMatches -and
        $(if ($ExpectFailure) { $null -ne $exitCode -and $exitCode -ne 0 -and $behaviorMarker }
          else { $exitCode -eq 0 -and -not $behaviorMarker })
    $record = [ordered]@{
        phase = $Phase
        command = "cargo test --locked --manifest-path apps/desktop/native-host/Cargo.toml $testName --lib -- --exact --nocapture"
        exit_code = $exitCode
        compiled_test_profile = $compiled
        exact_one_test_executed = $executed
        expected_summary = $summaryMatches
        descendant_survived_marker = $behaviorMarker
        state = $(if ($valid) { 'PASS' } else { 'FAIL' })
    }
    $script:records += $record
    if (-not $valid) {
        throw "Job mutation $Phase must compile and produce its required exact behavioral result."
    }
}

Push-Location $repoRoot
try {
    $head = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or @($head).Count -ne 1) { throw 'Exact source HEAD unavailable.' }
    Invoke-ExactJobTest 'baseline' $false
    try {
        [IO.File]::WriteAllText($sourcePath, $mutated, [Text.UTF8Encoding]::new($false))
        Invoke-ExactJobTest 'kill-on-close-removed' $true
    } finally {
        [IO.File]::WriteAllBytes($sourcePath, $original)
        if ((Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash -cne $baselineHash) {
            throw 'Exact production source restoration hash mismatch.'
        }
    }
    Invoke-ExactJobTest 'restored' $false
} finally {
    [ordered]@{
        schema = 'gogoke-holder-gone-job-mutation-v1'
        head_sha = $head
        production_source_sha256 = $baselineHash.ToLowerInvariant()
        scope = 'EXACT_JOB_CLOSE_BEHAVIOR_ONLY_NOT_FULL_LIBRARY_OR_OWNER_WIN11'
        mutation = 'Remove only production JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE assignment; keep complete test region.'
        records = $records
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'holder-gone-job-mutation.json') -Encoding utf8
    Pop-Location
}
