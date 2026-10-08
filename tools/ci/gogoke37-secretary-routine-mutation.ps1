[CmdletBinding()]
param([Parameter(Mandatory)][string]$EvidenceDirectory)
$ErrorActionPreference='Stop'
$PSNativeCommandUseErrorActionPreference=$false
$source=Join-Path $PSScriptRoot '..\..\apps\desktop\native-host\src\store\session_transport\secretary_user_turn.rs'
$source=[IO.Path]::GetFullPath($source)
$original=[IO.File]::ReadAllBytes($source)
$originalHash=(Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant()
$text=[Text.Encoding]::UTF8.GetString($original)
# Use a literal production SQL locator. Keep the parameter in the query so
# removing its source comparison cannot fail only through a bind-range error.
$needle='AND source_cursor=?4'
if ([regex]::Matches($text,[regex]::Escape($needle)).Count -ne 1) { throw 'Original USER marker source locator is not unique' }
$filter='changed_user_marker_h_response_and_current_authority_never_write'
function Observe([string]$label) {
    $output=& cargo test --locked --manifest-path apps/desktop/native-host/Cargo.toml --lib $filter -- --nocapture --show-output 2>&1
    $code=$LASTEXITCODE
    $output | Tee-Object -FilePath (Join-Path $EvidenceDirectory "$label.log") | Write-Host
    $joined=$output -join "`n"
    $summary=[regex]::Matches($joined,'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored;')
    if ($summary.Count -ne 1 -or -not [regex]::IsMatch($joined,'Finished.*test.*profile')) { throw "Mutation did not compile and execute one original test: $label" }
    [ordered]@{label=$label; exitCode=$code; passed=[int]$summary[0].Groups[2].Value; failed=[int]$summary[0].Groups[3].Value; ignored=[int]$summary[0].Groups[4].Value; originalMarkerAssertion=$joined.Contains('wrong INPUT marker')}
}
$results=@()
try {
    $baseline=Observe 'marker-baseline'
    if ($baseline.exitCode -ne 0 -or $baseline.passed -ne 1 -or $baseline.failed -ne 0 -or $baseline.ignored -ne 0) { throw 'Original marker boundary baseline failed' }
    $results+=$baseline
    $mutated=$text.Replace($needle,'AND (?4 IS NOT NULL)')
    [IO.File]::WriteAllText($source,$mutated,[Text.UTF8Encoding]::new($false))
    $mutation=Observe 'marker-source-comparison-removed'
    if ($mutation.exitCode -ne 101 -or $mutation.passed -ne 0 -or $mutation.failed -ne 1 -or $mutation.ignored -ne 0 -or -not $mutation.originalMarkerAssertion) { throw 'Production source comparison mutation did not fail its original assertion' }
    $results+=$mutation
} finally {
    [IO.File]::WriteAllBytes($source,$original)
    if ((Get-FileHash -LiteralPath $source).Hash.ToLowerInvariant() -cne $originalHash) { throw 'Original USER source file restoration failed' }
}
$restored=Observe 'marker-restored'
if ($restored.exitCode -ne 0 -or $restored.passed -ne 1 -or $restored.failed -ne 0 -or $restored.ignored -ne 0) { throw 'Restored marker boundary failed' }
$results+=$restored
[ordered]@{schema='gogoke.secretary-routine-source-mutation.v1'; sourceCommit=(git rev-parse HEAD).Trim(); originalSourceSha256=$originalHash; sourceRestored=$true; results=$results; acceptance=$false} | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'marker-mutation.json') -Encoding utf8NoBOM
