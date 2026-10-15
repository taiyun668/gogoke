param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('f-kernel-source', 'h-generation-source', 'native-boot-cutoff', 'kernel-replay-boot')]
    [string]$Axis,
    [Parameter(Mandatory = $true)]
    [string]$EvidenceRoot
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false

function Get-Sha256Bytes([byte[]]$Bytes) {
    $sha = [System.Security.Cryptography.SHA256]::Create()
    try { return ([BitConverter]::ToString($sha.ComputeHash($Bytes))).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Count-Ordinal([string]$Text, [string]$Needle) {
    if ([string]::IsNullOrEmpty($Needle)) { return 0 }
    $count = 0; $cursor = 0
    while (($found = $Text.IndexOf($Needle, $cursor, [StringComparison]::Ordinal)) -ge 0) {
        $count++; $cursor = $found + $Needle.Length
    }
    return $count
}

function Write-Json([string]$Path, $Value) {
    $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath $Path -Encoding utf8NoBOM
}

function Invoke-LoggedNative([string]$Label, [string]$Executable, [string[]]$Arguments) {
    $log = Join-Path $EvidenceRoot "$Label.log"
    $output = & $Executable @Arguments 2>&1
    $code = $LASTEXITCODE
    $output | Set-Content -LiteralPath $log -Encoding utf8NoBOM
    $output | Write-Host
    return [ordered]@{
        label = $Label
        command = @($Executable) + $Arguments
        exitCode = $code
        log = $log
        logSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($log))
        output = ($output -join [Environment]::NewLine)
    }
}

function Assert-Check([string]$Phase, [string]$Target) {
    $cargoArgs = @('check', '--locked', '--manifest-path', $script:manifest)
    if ($Target -ceq 'lib') { $cargoArgs += '--lib' }
    elseif ($Target -ceq 'bin') { $cargoArgs += @('--bin', 'gogoke-native-host') }
    else { throw "Unknown cargo check target: $Target" }
    $row = Invoke-LoggedNative "$Axis-$Phase-check-$Target" $script:cargo $cargoArgs
    if ($row.exitCode -ne 0 -or $row.output -match '(?m)^error\[E\d+\]|could not compile') {
        throw "$Phase cargo check $Target failed or reported a compiler error"
    }
    $row.output = ''
    return $row
}

function Invoke-ExactBehaviorTest([string]$Phase, [bool]$ExpectFailure, [string]$ExpectedFailureText) {
    $cargoArgs = @('test', '--locked', '--manifest-path', $script:manifest, '--lib', $script:testFilter,
              '--', '--exact', '--nocapture')
    $row = Invoke-LoggedNative "$Axis-$Phase-test" $script:cargo $cargoArgs
    $text = [string]$row.output
    $matches = [regex]::Matches($text,
        'test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out')
    $running = [regex]::Matches($text, '(?m)^running 1 test\s*$')
    $compileError = [regex]::IsMatch($text, 'could not compile|error\[E\d+\]')
    $row.summaryCount = $matches.Count
    $row.runningOneTestCount = $running.Count
    $row.compileError = $compileError
    $row.passed = if ($matches.Count -eq 1) { [int]$matches[0].Groups[2].Value } else { -1 }
    $row.failed = if ($matches.Count -eq 1) { [int]$matches[0].Groups[3].Value } else { -1 }
    $row.ignored = if ($matches.Count -eq 1) { [int]$matches[0].Groups[4].Value } else { -1 }
    $row.failureTextFound = if ($ExpectFailure) { $text.Contains($ExpectedFailureText) } else { $false }
    if ($running.Count -ne 1 -or $matches.Count -ne 1 -or $compileError) {
        throw "$Phase exact test did not produce one compiled test summary"
    }
    if ($ExpectFailure) {
        if ($row.exitCode -ne 101 -or $matches[0].Groups[1].Value -cne 'FAILED' -or
            $row.passed -ne 0 -or $row.failed -ne 1 -or $row.ignored -ne 0 -or
            -not $row.failureTextFound) {
            throw "$Phase mutation test must be 101 / 0 pass / 1 fail / 0 ignored and contain the exact failure text"
        }
    } elseif ($row.exitCode -ne 0 -or $matches[0].Groups[1].Value -cne 'ok' -or
        $row.passed -ne 1 -or $row.failed -ne 0 -or $row.ignored -ne 0) {
        throw "$Phase unchanged exact test must be 0 / 1 pass / 0 fail / 0 ignored"
    }
    $row.Remove('output')
    return $row
}

function Invoke-CheckPair([string]$Phase) {
    $lib = Assert-Check $Phase 'lib'
    $bin = Assert-Check $Phase 'bin'
    return @($lib, $bin)
}

$recipes = @{
    'f-kernel-source' = @{
        label = 'F KERNEL_BOOT_CUTOFF source arm'
        source = 'apps/desktop/native-host/src/store/instance/temporary.rs'
        testFile = 'apps/desktop/native-host/src/store/instance/temporary.rs'
        testMarkerPrefix = '#[cfg(test)]'
        needle = '("KERNEL_BOOT_CUTOFF", String::new(), Some((fact, source, pairs)))'
        # Keep the typed Some tuple so this behavioral refusal does not erase
        # the only concrete Option payload used by later kernel rechecks.
        replacement = '("KERNEL_BOOT_CUTOFF", { return Err(TemporaryHomeError::StopFactUnavailable); String::new() }, Some((fact, source, pairs)))'
        filter = 'store::instance::temporary::tests::synthetic_preboot_identity_checks_only_kernel_close_transaction_semantics'
        expectedFailureText = 'StopFactUnavailable'
    }
    'h-generation-source' = @{
        label = 'H original current-generation claim binding'
        source = 'apps/desktop/native-host/src/store/session_transport/temporary_home_close.rs'
        testFile = 'apps/desktop/native-host/src/store/session_transport/temporary_home_close.rs'
        testMarkerPrefix = '#[cfg(all(test, windows))]'
        needle = 'a[3] != generation ||'
        replacement = ''
        filter = 'store::session_transport::temporary_home_close::tests::original_home_source_is_qualified_before_any_os_absence_query'
        expectedFailureText = 'another generation cannot donate the old execution'
    }
    'native-boot-cutoff' = @{
        label = 'Native boot cutoff check'
        source = 'apps/desktop/native-host/src/process/session/legacy_holders_gone.rs'
        testFile = 'apps/desktop/native-host/src/store/product_database/v37_session_tests.rs'
        testMarkerPrefix = '#[cfg(test)]'
        needle = 'if boot_start <= creation_time_100ns {'
        replacement = 'if false {'
        filter = 'store::product_database::v37_session::tests::temporary_home_close_user_bridge_consumes_actual_native_claude_stop_without_model_call'
        expectedFailureText = 'the qualified original source must reach the real kernel cutoff'
    }
    'kernel-replay-boot' = @{
        label = 'F kernel replay sealed boot identity'
        source = 'apps/desktop/native-host/src/store/instance/temporary.rs'
        testFile = 'apps/desktop/native-host/src/store/instance/temporary.rs'
        testMarkerPrefix = '#[cfg(test)]'
        needle = 'Ok(sealed_boot)'
        replacement = 'Ok(current_boot)'
        filter = 'store::instance::temporary::tests::synthetic_preboot_identity_checks_only_kernel_close_transaction_semantics'
        expectedFailureText = 'a later boot must reuse the original sealed cutoff'
    }
}

$record = [ordered]@{
    schema = 'gogoke.pr94.kernel-home-mutation.v1'
    state = 'NOT_RUN'
    acceptance = $false
    installedV07 = 'NOT_RUN'
    actualPrebootProof = 'NOT_RUN'
    axis = $Axis
    sourceCommit = $env:GITHUB_SHA
    sourcePath = $null
    sourceOriginalSha256 = $null
    sourceChangedSha256 = 'NOT_RUN'
    sourceRestoredSha256 = 'NOT_RUN'
    sourceRestoredExact = $false
    testPath = $null
    testOriginalSha256 = $null
    testAfterMutationSha256 = 'NOT_RUN'
    testRestoredSha256 = 'NOT_RUN'
    inlineTestRegionOriginalSha256 = 'NOT_RUN'
    inlineTestRegionAfterMutationSha256 = 'NOT_RUN'
    inlineTestRegionRestoredSha256 = 'NOT_RUN'
    testBytesUnchangedDuringMutation = $false
    moduleMarker = $null
    productionNeedleCountBeforeRealTests = 0
    node = $null
    actualGitBackend = $null
    originalChecks = @()
    baselineTest = $null
    mutationChecks = @()
    mutationTest = $null
    restoredChecks = @()
    restoredTest = $null
    error = ''
}
$originalBytes = $null
$sourcePath = $null
$testPath = $null
$originalSourceHash = $null
$originalTestHash = $null
$originalTestRegionHash = $null
$sourceText = $null
$testRegionStart = -1
$sourceHadBom = $false
$git = $env:GOGOKE_CONTROLLED_GIT_PATH
$cargo = (Get-Command cargo.exe -CommandType Application | Select-Object -First 1 -ExpandProperty Source)
$manifest = 'apps/desktop/native-host/Cargo.toml'
$testFilter = $null
$expectedFailureText = $null

try {
    if ($env:GITHUB_ACTIONS -cne 'true') { throw 'This executor is CI-only; never mutate a local checkout.' }
    if ([string]::IsNullOrWhiteSpace($env:GITHUB_SHA) -or [string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ID) -or
        [string]::IsNullOrWhiteSpace($env:GITHUB_RUN_ATTEMPT)) { throw 'GitHub source/run identity is incomplete.' }
    if (-not (Test-Path -LiteralPath $EvidenceRoot -PathType Container)) { throw 'Pre-created evidence directory is missing.' }
    $root = (Get-Location).Path
    $checkedOut = (& $git rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or $checkedOut -cne $env:GITHUB_SHA) { throw 'Checkout differs from the exact workflow source SHA.' }
    if ((& $git status --porcelain --untracked-files=no).Count -ne 0) { throw 'Tracked checkout is not initially clean.' }

    $recipe = $recipes[$Axis]
    $sourcePath = Join-Path $root $recipe.source
    $testPath = Join-Path $root $recipe.testFile
    $testFilter = $recipe.filter
    $expectedFailureText = $recipe.expectedFailureText
    $record.sourcePath = $recipe.source
    $record.testPath = $recipe.testFile
    $record.moduleMarker = $recipe.testMarkerPrefix + "`nmod tests {"

    if (-not (Test-Path -LiteralPath $sourcePath -PathType Leaf) -or
        -not (Test-Path -LiteralPath $testPath -PathType Leaf)) { throw 'Production source or original test source is missing.' }
    $originalBytes = [IO.File]::ReadAllBytes($sourcePath)
    $testBytes = [IO.File]::ReadAllBytes($testPath)
    $sourceHadBom = $originalBytes.Length -ge 3 -and $originalBytes[0] -eq 0xEF -and $originalBytes[1] -eq 0xBB -and $originalBytes[2] -eq 0xBF
    $utf8 = [Text.UTF8Encoding]::new($false, $true)
    $offset = if ($sourceHadBom) { 3 } else { 0 }
    $sourceText = $utf8.GetString($originalBytes, $offset, $originalBytes.Length - $offset)
    $originalSourceHash = Get-Sha256Bytes $originalBytes
    $originalTestHash = Get-Sha256Bytes $testBytes
    $marker = $recipe.testMarkerPrefix + $(if ($sourceText.Contains("`r`n")) { "`r`n" } else { "`n" }) + 'mod tests {'
    $testRegionStart = $sourceText.IndexOf($marker, [StringComparison]::Ordinal)
    if ($testRegionStart -lt 0 -or (Count-Ordinal $sourceText $marker) -ne 1) {
        throw 'Exact real `mod tests {` boundary is missing or ambiguous.'
    }
    $needleCount = Count-Ordinal $sourceText $recipe.needle
    $needleStart = $sourceText.IndexOf($recipe.needle, [StringComparison]::Ordinal)
    $record.productionNeedleCountBeforeRealTests = $needleCount
    if ($needleCount -ne 1 -or $needleStart -lt 0 -or $needleStart -ge $testRegionStart) {
        throw 'Production mutation needle must occur exactly once before the real test module boundary.'
    }
    $originalTestRegionHash = Get-Sha256Bytes ($utf8.GetBytes($sourceText.Substring($testRegionStart)))
    $record.inlineTestRegionOriginalSha256 = $originalTestRegionHash
    if ($testPath -ne $sourcePath -and $testPath -notmatch 'v37_session_tests\.rs$') {
        throw 'Separate original test source path differs from the authorized native USER fixture.'
    }

    $nodePath = $env:GOGOKE_CONTROLLED_NODE_PATH
    if (-not (Test-Path -LiteralPath $nodePath -PathType Leaf)) { throw 'Pinned Node path is unavailable.' }
    $nodeOutput = & $nodePath --version 2>&1
    if ($LASTEXITCODE -ne 0 -or ($nodeOutput -join '') -cne 'v24.13.1') { throw 'Controlled Node version mismatch.' }
    $gitOutput = & $git --version 2>&1
    if ($LASTEXITCODE -ne 0) { throw 'Actual Git backend version query failed.' }
    $gitBackendHash = (Get-FileHash -LiteralPath $git -Algorithm SHA256).Hash.ToLowerInvariant()
    $nodeHash = (Get-FileHash -LiteralPath $nodePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $record.node = [ordered]@{path=$nodePath;version=($nodeOutput -join '');sha256=$nodeHash}
    $record.actualGitBackend = [ordered]@{path=$git;version=($gitOutput -join '');sha256=$gitBackendHash}
    $record.sourceOriginalSha256 = $originalSourceHash
    $record.testOriginalSha256 = $originalTestHash
    [IO.File]::WriteAllBytes((Join-Path $EvidenceRoot 'original-production-source.bin'), $originalBytes)
    if ($testPath -ne $sourcePath) { [IO.File]::WriteAllBytes((Join-Path $EvidenceRoot 'original-test-source.bin'), $testBytes) }

    $record.originalChecks = Invoke-CheckPair 'original'
    $record.baselineTest = Invoke-ExactBehaviorTest 'baseline' $false ''
    $lineEnding = if ($sourceText.Contains("`r`n")) { "`r`n" } else { "`n" }
    $replacement = $recipe.replacement
    $mutatedText = $sourceText.Replace($recipe.needle, $replacement)
    $mutatedMarker = $recipe.testMarkerPrefix + $lineEnding + 'mod tests {'
    if ((Count-Ordinal $mutatedText $mutatedMarker) -ne 1) {
        throw 'Real test module boundary changed while constructing the mutation.'
    }
    $mutatedModuleStart = $mutatedText.IndexOf($mutatedMarker, [StringComparison]::Ordinal)
    $mutatedTestRegionHash = Get-Sha256Bytes ($utf8.GetBytes($mutatedText.Substring($mutatedModuleStart)))
    $record.inlineTestRegionAfterMutationSha256 = $mutatedTestRegionHash
    if ($mutatedTestRegionHash -cne $originalTestRegionHash) { throw 'Prepared production mutation changes the original inline test bytes.' }

    try {
        $mutatedBytes = $utf8.GetBytes($mutatedText)
        if ($sourceHadBom) { $mutatedBytes = [byte[]](@(0xEF,0xBB,0xBF) + $mutatedBytes) }
        [IO.File]::WriteAllBytes($sourcePath, $mutatedBytes)
        $record.sourceChangedSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($sourcePath))
        if ($record.sourceChangedSha256 -ceq $originalSourceHash) { throw 'Production mutation changed no bytes.' }
        $record.testAfterMutationSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($testPath))
        if ($testPath -ne $sourcePath -and $record.testAfterMutationSha256 -cne $originalTestHash) {
            throw 'Separate original test file bytes changed during production mutation.'
        }
        $afterMutationText = $utf8.GetString([IO.File]::ReadAllBytes($sourcePath), $(if ($sourceHadBom) { 3 } else { 0 }), ([IO.File]::ReadAllBytes($sourcePath).Length - $(if ($sourceHadBom) { 3 } else { 0 })))
        $afterMutationModule = $afterMutationText.IndexOf($marker, [StringComparison]::Ordinal)
        if ($afterMutationModule -lt 0 -or
            (Get-Sha256Bytes ($utf8.GetBytes($afterMutationText.Substring($afterMutationModule)))) -cne $originalTestRegionHash) {
            throw 'Inline fixture/test region changed during production mutation.'
        }
        $record.mutationChecks = Invoke-CheckPair 'mutation'
        $record.mutationTest = Invoke-ExactBehaviorTest 'mutation' $true $expectedFailureText
    } finally {
        [IO.File]::WriteAllBytes($sourcePath, $originalBytes)
        $record.sourceRestoredSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($sourcePath))
        $record.sourceRestoredExact = $record.sourceRestoredSha256 -ceq $originalSourceHash
        $record.testRestoredSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($testPath))
        if ($testPath -eq $sourcePath) {
            $restoredText = $utf8.GetString([IO.File]::ReadAllBytes($sourcePath), $offset, ([IO.File]::ReadAllBytes($sourcePath).Length - $offset))
            $restoredMarkerIndex = $restoredText.IndexOf($marker, [StringComparison]::Ordinal)
            $record.testBytesUnchangedDuringMutation = $restoredMarkerIndex -ge 0 -and
                (Get-Sha256Bytes ($utf8.GetBytes($restoredText.Substring($restoredMarkerIndex)))) -ceq $originalTestRegionHash
        } else {
            $record.testBytesUnchangedDuringMutation = $record.testRestoredSha256 -ceq $originalTestHash
        }
        if ($record.sourceRestoredExact) {
            try {
                $record.restoredChecks = Invoke-CheckPair 'restored'
                $record.restoredTest = Invoke-ExactBehaviorTest 'restored' $false ''
            } catch { $record.error = "Restored validation failed: $($_.Exception.Message)" }
        }
    }

    if (-not $record.sourceRestoredExact -or -not $record.testBytesUnchangedDuringMutation -or
        $record.originalChecks.Count -ne 2 -or $record.mutationChecks.Count -ne 2 -or $record.restoredChecks.Count -ne 2 -or
        $record.baselineTest.passed -ne 1 -or $record.mutationTest.failed -ne 1 -or $record.restoredTest.passed -ne 1) {
        throw 'One or more original/mutation/restoration evidence checks did not pass.'
    }
    if ((& $git status --porcelain --untracked-files=no).Count -ne 0) { throw 'Tracked source tree is dirty after exact restoration.' }
    if ((Get-FileHash -LiteralPath $git -Algorithm SHA256).Hash.ToLowerInvariant() -cne $gitBackendHash -or
        (Get-FileHash -LiteralPath $nodePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne $nodeHash) {
        throw 'Controlled Node or Git backend bytes changed during the mutation.'
    }
    $record.state = 'PASS_CI_ONLY_PRODUCTION_MUTATION_AND_EXACT_RESTORATION'
} catch {
    $record.error = $_.Exception.Message
    if ($record.sourceRestoredExact) { $record.state = 'FAIL_MUTATION_EVIDENCE_RESTORED' }
    elseif ($originalBytes -ne $null) { $record.state = 'FAIL_RESTORE_NOT_CONFIRMED' }
    else { $record.state = 'FAIL_ORIGINAL_OR_MUTATION_NOT_RUN' }
} finally {
    if ($originalBytes -ne $null -and $sourcePath -and (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        try {
            if ((Get-Sha256Bytes ([IO.File]::ReadAllBytes($sourcePath))) -cne $originalSourceHash) {
                [IO.File]::WriteAllBytes($sourcePath, $originalBytes)
            }
            $record.sourceRestoredSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($sourcePath))
            $record.sourceRestoredExact = $record.sourceRestoredSha256 -ceq $originalSourceHash
            if ($record.sourceRestoredExact -and $testPath -eq $sourcePath -and $sourceText) {
                $finalBytes = [IO.File]::ReadAllBytes($sourcePath)
                $finalOffset = if ($sourceHadBom) { 3 } else { 0 }
                $finalText = $utf8.GetString($finalBytes, $finalOffset, $finalBytes.Length - $finalOffset)
                $finalMarker = $recipe.testMarkerPrefix + $(if ($finalText.Contains("`r`n")) { "`r`n" } else { "`n" }) + 'mod tests {'
                $finalMarkerIndex = $finalText.IndexOf($finalMarker, [StringComparison]::Ordinal)
                $record.testBytesUnchangedDuringMutation = $finalMarkerIndex -ge 0 -and
                    (Get-Sha256Bytes ($utf8.GetBytes($finalText.Substring($finalMarkerIndex)))) -ceq $originalTestRegionHash
            $record.inlineTestRegionRestoredSha256 = if ($finalMarkerIndex -ge 0) {
                Get-Sha256Bytes ($utf8.GetBytes($finalText.Substring($finalMarkerIndex)))
            } else { 'MISSING' }
                $record.testRestoredSha256 = Get-Sha256Bytes $finalBytes
            } elseif ($testPath -and $testPath -ne $sourcePath -and (Test-Path -LiteralPath $testPath -PathType Leaf)) {
                $record.testRestoredSha256 = Get-Sha256Bytes ([IO.File]::ReadAllBytes($testPath))
                if ($record.testAfterMutationSha256 -eq 'NOT_RUN') {
                    $record.testAfterMutationSha256 = $record.testRestoredSha256
                }
                $record.testBytesUnchangedDuringMutation =
                    $record.testRestoredSha256 -ceq $originalTestHash -and
                    $record.testAfterMutationSha256 -ceq $originalTestHash
            }
        } catch {
            $record.error = "Final source/test restoration audit failed: $($_.Exception.Message)"
            $record.sourceRestoredExact = $false
        }
    }
    if ($record.state -cne 'PASS_CI_ONLY_PRODUCTION_MUTATION_AND_EXACT_RESTORATION') {
        if ($record.sourceRestoredExact) {
            $record.state = if ($record.sourceChangedSha256 -cne 'NOT_RUN') {
                'FAIL_MUTATION_EVIDENCE_RESTORED'
            } else { 'FAIL_ORIGINAL_OR_MUTATION_NOT_RUN' }
        } elseif ($originalBytes -ne $null) {
            $record.state = 'FAIL_RESTORE_NOT_CONFIRMED'
        }
    }
    $record.acceptance = $false
    $record.installedV07 = 'NOT_RUN'
    $record.actualPrebootProof = 'NOT_RUN'
    $record.finalSourceSha256 = if ($sourcePath -and (Test-Path -LiteralPath $sourcePath -PathType Leaf)) {
        Get-Sha256Bytes ([IO.File]::ReadAllBytes($sourcePath))
    } else { 'NOT_READ' }
    $record.finalTestSha256 = if ($testPath -and (Test-Path -LiteralPath $testPath -PathType Leaf)) {
        Get-Sha256Bytes ([IO.File]::ReadAllBytes($testPath))
    } else { 'NOT_READ' }
    Write-Json (Join-Path $EvidenceRoot 'frozen-original-result.json') $record
}

if ($record.state -cne 'PASS_CI_ONLY_PRODUCTION_MUTATION_AND_EXACT_RESTORATION') {
    throw "PR94 mutation axis failed: $Axis state=$($record.state) $($record.error)"
}
