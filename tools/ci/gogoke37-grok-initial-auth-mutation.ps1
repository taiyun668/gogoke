param([Parameter(Mandatory)][string]$EvidenceRoot)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$sourcePath = Join-Path $repoRoot 'apps/desktop/native-host/src/store/session_transport/grok_home_launch.rs'
$manifest = Join-Path $repoRoot 'apps/desktop/native-host/Cargo.toml'
$testName = 'store::session_transport::grok_home_launch::tests::original_inherited_auth_is_journaled_protected_and_revoked_without_process'
$original = [IO.File]::ReadAllBytes($sourcePath)
$source = [Text.Encoding]::UTF8.GetString($original)
$baselineHash = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash
$needle = '        let auth=evidence("observe-original-auth-metadata",' + "`n" +
    '            observe_grok_auth_candidate(&home.path,&home.identity))?;'
if ($source.Contains("`r`n")) { $needle = $needle.Replace("`n", "`r`n") }
if ([regex]::Matches($source, [regex]::Escape($needle)).Count -ne 1) {
    throw 'Initial production metadata observation must match exactly once.'
}
$testStart = $source.IndexOf('#[cfg(test)]')
if ($testStart -lt 0 -or $source.IndexOf($needle) -ge $testStart) {
    throw 'Mutation must belong to production before the unchanged test region.'
}
$mutated = $source.Replace($needle, $needle.Replace('observe_grok_auth_candidate(', 'observe_grok_auth('))
if ($source.Substring($testStart) -cne $mutated.Substring($mutated.IndexOf('#[cfg(test)]'))) {
    throw 'Complete test region changed during production mutation.'
}
[void][IO.Directory]::CreateDirectory($EvidenceRoot)
$records = @()
$head = ''
$restored = $false
function Invoke-OriginalAclTest([string]$Phase, [bool]$ExpectFailure) {
    $output = & cargo test --locked --manifest-path $manifest --lib $testName -- --exact --nocapture 2>&1
    $exitCode = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    $output | Tee-Object -FilePath (Join-Path $EvidenceRoot "grok-initial-auth-$Phase.log") | Out-Host
    $compiled = [regex]::IsMatch($text, 'Finished.*test.*profile')
    $executed = [regex]::IsMatch($text, '(?m)^running 1 test\s*$')
    $summary = if ($ExpectFailure) {
        'test result: FAILED\. 0 passed; 1 failed; 0 ignored;'
    } else {
        'test result: ok\. 1 passed; 0 failed; 0 ignored;'
    }
    $originalAclFailure = $text.Contains('AclWitnessMismatch')
    $valid = $compiled -and $executed -and [regex]::IsMatch($text, $summary) -and
        $(if ($ExpectFailure) { $null -ne $exitCode -and $exitCode -ne 0 -and $originalAclFailure }
          else { $exitCode -eq 0 -and -not $originalAclFailure })
    $script:records += [ordered]@{
        phase = $Phase
        command = "cargo test --locked --manifest-path apps/desktop/native-host/Cargo.toml --lib $testName -- --exact --nocapture"
        exit_code = $exitCode
        compiled_test_profile = $compiled
        exact_one_test_executed = $executed
        expected_summary = [regex]::IsMatch($text, $summary)
        original_acl_failure = $originalAclFailure
        state = $(if ($valid) { 'PASS' } else { 'FAIL' })
    }
    if (-not $valid) { throw "Grok initial-auth $Phase must compile and produce the required unchanged behavioral result." }
}
Push-Location $repoRoot
try {
    $head = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or @($head).Count -ne 1) { throw 'Exact source HEAD unavailable.' }
    Invoke-OriginalAclTest 'baseline' $false
    try {
        [IO.File]::WriteAllText($sourcePath, $mutated, [Text.UTF8Encoding]::new($false))
        Invoke-OriginalAclTest 'strict-precondition-restored' $true
    } finally {
        [IO.File]::WriteAllBytes($sourcePath, $original)
        $restored = (Get-FileHash -LiteralPath $sourcePath -Algorithm SHA256).Hash -ceq $baselineHash
        if (-not $restored) { throw 'Exact production source restoration hash mismatch.' }
    }
    Invoke-OriginalAclTest 'restored' $false
} finally {
    [ordered]@{
        schema = 'gogoke-grok-initial-auth-mutation-v1'
        head_sha = $head
        production_source_sha256 = $baselineHash.ToLowerInvariant()
        scope = 'EXACT_INITIAL_GROK_AUTH_BOUNDARY_NOT_FULL_LIBRARY_OR_OWNER_WIN11'
        mutation = 'Restore only the strict protected-DACL precondition before the existing initial grant; retain complete tests and all authority checks.'
        source_restored = $restored
        records = $records
    } | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'grok-initial-auth-mutation.json') -Encoding utf8
    Pop-Location
}
