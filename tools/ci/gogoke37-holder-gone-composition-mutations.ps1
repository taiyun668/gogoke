[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$EvidenceRoot)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
if ($env:GITHUB_ACTIONS -cne 'true' -or $env:RUNNER_OS -cne 'Windows') {
    throw 'Native composition mutations are cloud Windows CI only.'
}
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$path = Join-Path $repoRoot 'apps/desktop/native-host/src/store/product_database/v37_holder_disappearance.rs'
$testPath = Join-Path $repoRoot 'apps/desktop/native-host/src/store/product_database/v37_holder_disappearance_tests.rs'
$manifest = Join-Path $repoRoot 'apps/desktop/native-host/Cargo.toml'
$testName = 'store::product_database::v37_holder_disappearance_tests::actual_two_disappeared_holders_recover_in_one_call_replay_without_acl_effect_and_admit_cold_source'
$original = [IO.File]::ReadAllBytes($path)
$source = [Text.Encoding]::UTF8.GetString($original)
$sourceHash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
$testHash = (Get-FileHash -LiteralPath $testPath -Algorithm SHA256).Hash
$cases = @(
    @{
        axis = 'stale-final-facts'
        expectedReason = 'holder frozen claim receipt changed'
        needle = '                self.gone_validate_capture(&self.gone_original(&fresh_profile)?,'
        replacement = '                self.gone_validate_capture(originals.get(&record.input.binding_id).ok_or_else(||refused("holder completed original absent"))?,'
    },
    @{
        axis = 'nested-owner-transaction'
        expectedReason = 'cannot start a transaction within a transaction'
        needle = '                    self.gone_scope_in_current_transaction(instance_id,&allowed,incoming)?;'
        replacement = '                    self.gone_scope(instance_id,&allowed,incoming)?;'
    }
)
New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null
$records = @()
function Invoke-Composition([string]$Phase, [bool]$ExpectFailure, [string]$ExpectedReason = '') {
    $output = & cargo test --locked --manifest-path $manifest $testName --lib -- --exact --nocapture 2>&1
    $code = $LASTEXITCODE
    $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
    $output | Tee-Object -FilePath (Join-Path $EvidenceRoot "holder-gone-composition-$Phase.log") | Out-Host
    $compiled = [regex]::IsMatch($text, 'Finished.*test.*profile')
    $executed = [regex]::IsMatch($text, '(?m)^running 1 test\s*$')
    $summary = if ($ExpectFailure) { 'test result: FAILED\. 0 passed; 1 failed; 0 ignored;' }
               else { 'test result: ok\. 1 passed; 0 failed; 0 ignored;' }
    $firstCall = $text.Contains('FIRST composed recovery must revoke both grants and release both H claims')
    $causeMatches = $ExpectFailure -and -not [string]::IsNullOrEmpty($ExpectedReason) -and
        $firstCall -and $text.Contains($ExpectedReason)
    $valid = $compiled -and $executed -and [regex]::IsMatch($text, $summary) -and
        $(if ($ExpectFailure) { $null -ne $code -and $code -ne 0 -and $causeMatches } else { $code -eq 0 })
    $script:records += [ordered]@{ phase=$Phase; exit_code=$code; compiled=$compiled;
        first_recovery_assertion=$firstCall; expected_reason=$ExpectedReason; cause_matches=$causeMatches;
        exact_one_test=$executed; state=$(if($valid){'PASS'}else{'FAIL'}); original_log="holder-gone-composition-$Phase.log" }
    if (-not $valid) { throw "Composition $Phase must compile and execute the unchanged exact behavioral result." }
}
Push-Location $repoRoot
try {
    $head = & git rev-parse HEAD
    if ($LASTEXITCODE -ne 0 -or @($head).Count -ne 1) { throw 'Exact source HEAD unavailable.' }
    Invoke-Composition 'baseline' $false
    foreach ($case in $cases) {
        if ([regex]::Matches($source, [regex]::Escape($case.needle)).Count -ne 1) {
            throw "Exact production locator must match once: $($case.axis)"
        }
        try {
            [IO.File]::WriteAllText($path, $source.Replace($case.needle, $case.replacement), [Text.UTF8Encoding]::new($false))
            if ((Get-FileHash -LiteralPath $testPath -Algorithm SHA256).Hash -cne $testHash) { throw 'Composition test changed.' }
            Invoke-Composition "mutation-$($case.axis)" $true $case.expectedReason
        } finally {
            [IO.File]::WriteAllBytes($path, $original)
            if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -cne $sourceHash -or
                (Get-FileHash -LiteralPath $testPath -Algorithm SHA256).Hash -cne $testHash) {
                throw 'Exact production and unchanged test restoration mismatch.'
            }
        }
        Invoke-Composition "restored-$($case.axis)" $false
    }
} finally {
    [ordered]@{ schema='gogoke.holder-gone-composition-mutations.v1'; head_sha=$head;
        production_source_sha256=$sourceHash.ToLowerInvariant(); test_sha256=$testHash.ToLowerInvariant();
        meaning='COMPOSED_NATIVE_BEHAVIOR_NOT_OWNER_WIN11_OR_ACCEPTANCE'; records=$records } |
        ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'holder-gone-composition-mutations.json') -Encoding utf8
    Pop-Location
}
