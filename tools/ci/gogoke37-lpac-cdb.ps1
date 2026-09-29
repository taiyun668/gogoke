param([Parameter(Mandatory = $true)][string]$EvidenceRoot)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$privateRoot = Join-Path $env:RUNNER_TEMP 'gogoke37-lpac-cdb-private'
New-Item -ItemType Directory -Path $privateRoot, $EvidenceRoot -Force | Out-Null

$build = & cargo test --locked --manifest-path apps/desktop/native-host/Cargo.toml --lib --no-run --message-format=json 2>&1
if ($LASTEXITCODE -ne 0) { throw 'Exact native test image build failed' }
$images = @($build | ForEach-Object {
    try { $_ | ConvertFrom-Json -ErrorAction Stop } catch { $null }
} | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'gogoke_native_host' -and
    $_.target.kind -contains 'lib' -and $_.profile.test -eq $true -and $_.executable
} | ForEach-Object { $_.executable } | Select-Object -Unique)
if ($images.Count -ne 1) { throw "Expected one native test image; found $($images.Count)" }
$testImage = $images[0]
$testHash = (Get-FileHash -LiteralPath $testImage -Algorithm SHA256).Hash.ToLowerInvariant()

$cdb = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/Debuggers/x64/cdb.exe'
if (-not (Test-Path -LiteralPath $cdb -PathType Leaf)) { throw 'Microsoft CDB unavailable on runner' }
$signature = Get-AuthenticodeSignature -LiteralPath $cdb
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft') {
    throw 'Microsoft CDB signature invalid'
}
$cdbHash = (Get-FileHash -LiteralPath $cdb -Algorithm SHA256).Hash.ToLowerInvariant()
$cdbVersion = (Get-Item -LiteralPath $cdb).VersionInfo.FileVersion

$gatePointer = Join-Path $privateRoot 'gate-path.txt'
$commands = Join-Path $privateRoot 'breakpoints.txt'
@'
.childdbg 0
bu KERNELBASE!CreateProcessInternalW "r $t2 = @$ra; .printf \"LPAC_PARENT_ENTER tid=%x return=%p\\n\", @$tid, @$ra; gc"
bu ntdll!NtCreateUserProcess ".printf \"LPAC_NT_ENTER tid=%x\\n\", @$tid; k 32"
bl
.echo LPAC_BREAKPOINTS_READY
g
'@ | Set-Content -LiteralPath $commands -Encoding ascii
$debugLog = Join-Path $privateRoot 'cdb-raw.log'
$testOut = Join-Path $privateRoot 'test-out.log'
$testErr = Join-Path $privateRoot 'test-err.log'
$oldGate = $env:GOGOKE_LPAC_CDB_DIR
$env:GOGOKE_LPAC_CDB_DIR = $gatePointer
try {
    $test = Start-Process -FilePath $testImage -ArgumentList @(
        '--exact', 'process::windows::tests::real_app_container_descendant_reaches_native_seat_pipe', '--nocapture'
    ) -WindowStyle Hidden -RedirectStandardOutput $testOut -RedirectStandardError $testErr -PassThru
} finally {
    $env:GOGOKE_LPAC_CDB_DIR = $oldGate
}
$debugger = $null
$phase = 'WAIT_PARENT'
$instrumentationError = $null
$maxLogBytes = 16 * 1024 * 1024

function Wait-CdbMarker([string]$marker, [int]$seconds) {
    $deadline = (Get-Date).AddSeconds($seconds)
    while ((Get-Date) -le $deadline) {
        if (Test-Path -LiteralPath $debugLog -PathType Leaf) {
            if ((Get-Item -LiteralPath $debugLog).Length -gt $maxLogBytes) {
                throw 'CDB private log exceeded 16 MiB calibration bound'
            }
            if (@(Get-Content -LiteralPath $debugLog -Tail 200 |
                    Where-Object { $_ -eq $marker -or $_ -match "^$([regex]::Escape($marker)) tid=[0-9a-f]+`$" }).Count -gt 0) { return }
        }
        if ($debugger -and $debugger.HasExited) { throw "CDB exited before $marker" }
        if ($test.HasExited) { throw "Exact LPAC test exited before $marker" }
        Start-Sleep -Milliseconds 100
    }
    throw "CDB timed out before $marker"
}

function Send-Cdb([string[]]$lines) {
    if (-not $debugger -or $debugger.HasExited) { throw 'CDB stdin is unavailable' }
    foreach ($line in $lines) { $debugger.StandardInput.WriteLine($line) }
    $debugger.StandardInput.Flush()
}

try {
    $deadline = (Get-Date).AddSeconds(45)
    while (-not (Test-Path -LiteralPath $gatePointer -PathType Leaf)) {
        if ($test.HasExited) { throw 'Exact LPAC test exited before ACL-granted directory publication' }
        if ((Get-Date) -gt $deadline) { throw 'Exact LPAC directory publication timed out' }
        Start-Sleep -Milliseconds 100
    }
    $gate = (Get-Content -LiteralPath $gatePointer -Raw).Trim()
    while (-not (Test-Path -LiteralPath (Join-Path $gate 'parent.pid') -PathType Leaf)) {
        if ($test.HasExited) { throw 'Exact LPAC test exited before parent PID publication' }
        if ((Get-Date) -gt $deadline) { throw 'Exact LPAC parent PID publication timed out' }
        Start-Sleep -Milliseconds 100
    }
    $parentPid = [int](Get-Content -LiteralPath (Join-Path $gate 'parent.pid') -Raw)
    $start = [System.Diagnostics.ProcessStartInfo]::new($cdb)
    foreach ($argument in @('-p', "$parentPid", '-G', '-cf', $commands, '-logo', $debugLog)) {
        [void]$start.ArgumentList.Add($argument)
    }
    $start.UseShellExecute = $false
    $start.RedirectStandardInput = $true
    $start.CreateNoWindow = $true
    $start.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
    $debugger = [System.Diagnostics.Process]::Start($start)
    if (-not $debugger) { throw 'CDB did not start' }
    $phase = 'WAIT_BREAKPOINTS'
    Wait-CdbMarker 'LPAC_BREAKPOINTS_READY' 45
    New-Item -ItemType File -Path (Join-Path $gate 'release') -Force | Out-Null
    $phase = 'WAIT_NT_ENTRY'
    Wait-CdbMarker 'LPAC_NT_ENTER' 45
    $entryLog = @(Get-Content -LiteralPath $debugLog)
    $ntEntryIndex = -1
    $ntThread = $null
    for ($i = 0; $i -lt $entryLog.Count; $i++) {
        if ($entryLog[$i] -match '^LPAC_NT_ENTER tid=([0-9a-f]+)$') {
            $ntEntryIndex = $i
            $ntThread = $Matches[1].ToLowerInvariant()
            break
        }
    }
    if ($ntEntryIndex -lt 0) { throw 'CDB did not expose the NtCreateUserProcess thread' }
    $parentReturn = $null
    for ($i = 0; $i -lt $ntEntryIndex; $i++) {
        if ($entryLog[$i] -match '^LPAC_PARENT_ENTER tid=([0-9a-f]+) return=([0-9a-f`]+)$' -and
            $Matches[1].ToLowerInvariant() -eq $ntThread) {
            $parentReturn = $Matches[2]
        }
    }
    if (-not $parentReturn -or ($parentReturn -replace '[`0]', '') -eq '') {
        throw 'No preceding CreateProcessInternalW entry on the NtCreateUserProcess thread'
    }
    $parentOnNtStack = @($entryLog | Select-Object -Skip ($ntEntryIndex + 1) |
        Where-Object { $_ -match '(?i)KERNELBASE!CreateProcessInternalW(?:\+|\b)' }).Count -gt 0
    if (-not $parentOnNtStack) { throw 'NtCreateUserProcess stack did not contain CreateProcessInternalW' }
    $phase = 'WAIT_NT_RETURN'
    Send-Cdb @(
        "r `$t2 = $parentReturn", 'r $t0 = @rcx', 'r $t1 = @rdx', 'r $t3 = @$ra',
        '.printf "LPAC_NT_CAPTURE tid=%x parent=%p return=%p process_slot=%p thread_slot=%p\n", @$tid, @$t2, @$t3, @$t0, @$t1',
        '~.bp /1 @$t3 ".printf \"LPAC_NT_RETURN tid=%x\\n\", @$tid"', '.echo LPAC_NT_ARMED', 'g'
    )
    Wait-CdbMarker 'LPAC_NT_RETURN' 30
    $phase = 'WAIT_NT_SNAPSHOT'
    Send-Cdb @(
        '.printf "LPAC_NT_STATUS %08x\n", @eax',
        '.printf "LPAC_PROCESS_HANDLE %p\n", poi(@$t0)',
        '.printf "LPAC_THREAD_HANDLE %p\n", poi(@$t1)',
        '.echo LPAC_NT_SNAPSHOT_DONE'
    )
    Wait-CdbMarker 'LPAC_NT_SNAPSHOT_DONE' 15
    # Get-Content strips CRLF from each line. Regex '$' against -Raw text
    # sees the remaining CR and falsely reports a different thread.
    $snapshot = @(Get-Content -LiteralPath $debugLog)
    $capture = @($snapshot | Where-Object {
        $_ -match '^LPAC_NT_CAPTURE tid=([0-9a-f]+) parent=([0-9a-f`]+) return=([0-9a-f`]+)'
    })
    if ($capture.Count -ne 1 -or
        $capture[0] -notmatch '^LPAC_NT_CAPTURE tid=([0-9a-f]+) parent=([0-9a-f`]+) return=([0-9a-f`]+)') {
        throw 'CDB did not expose both saved return addresses'
    }
    if ($Matches[1].ToLowerInvariant() -ne $ntThread -or $Matches[2] -ne $parentReturn -or
        ($Matches[2] -replace '[`0]', '') -eq '' -or ($Matches[3] -replace '[`0]', '') -eq '') {
        throw 'CDB saved a zero return address'
    }
    if (@($snapshot | Where-Object { $_ -eq "LPAC_NT_RETURN tid=$ntThread" }).Count -ne 1) {
        throw 'NtCreateUserProcess return occurred on another thread'
    }
    if (@($snapshot | Where-Object { $_ -match '^LPAC_NT_STATUS [0-9a-f]{8}$' }).Count -ne 1 -or
        @($snapshot | Where-Object { $_ -match '^LPAC_PROCESS_HANDLE [0-9a-f`]+$' }).Count -ne 1 -or
        @($snapshot | Where-Object { $_ -match '^LPAC_THREAD_HANDLE [0-9a-f`]+$' }).Count -ne 1) {
        throw 'CDB did not expose status and both output handles'
    }
    $phase = 'WAIT_TRACE_ENDPOINT'
    Send-Cdb @(
        'bc *', '.echo LPAC_TRACE_BEGIN', 'wt -oa -or @$t2',
        '.printf "LPAC_TRACE_ENDPOINT tid=%x\n", @$tid',
        '.if (@$ip == @$t2) { .echo LPAC_TRACE_ENDPOINT_MATCH } .else { .echo LPAC_TRACE_ENDPOINT_MISMATCH }',
        '.printf "LPAC_PARENT_STATUS %08x\n", @eax',
        '.echo LPAC_FINAL_ERROR_BEGIN', '!gle', '.echo LPAC_FINAL_ERROR_END',
        '.echo LPAC_TRACE_DONE'
    )
    Wait-CdbMarker 'LPAC_TRACE_DONE' 90
    $phase = 'WAIT_TEST'
    Send-Cdb @('g')
    if (-not $test.WaitForExit(150000)) { throw 'Exact LPAC diagnostic test timed out' }
    $phase = 'FINISHED'
} catch {
    $instrumentationError = $_.Exception.Message
} finally {
    if (-not $test.HasExited) {
        try { $test.Kill(); [void]$test.WaitForExit(5000) }
        catch { $instrumentationError = "${instrumentationError}; exact test cleanup: $($_.Exception.Message)" }
    }
    if ($debugger -and -not $debugger.HasExited) {
        try { $debugger.Kill(); [void]$debugger.WaitForExit(5000) }
        catch { $instrumentationError = "${instrumentationError}; CDB cleanup: $($_.Exception.Message)" }
    }
}

$raw = if (Test-Path -LiteralPath $debugLog -PathType Leaf) {
    Get-Content -LiteralPath $debugLog
} else { @() }
$markers = @($raw | Where-Object { $_ -match '^LPAC_[A-Z_]+$' } | Select-Object -First 40)
$ntStatus = @($raw | ForEach-Object {
    if ($_ -match '^LPAC_NT_STATUS ([0-9a-f]{8})$') { $Matches[1] }
} | Select-Object -First 1)
$handleStates = [ordered]@{}
foreach ($kind in @('PROCESS', 'THREAD')) {
    $value = @($raw | ForEach-Object {
        if ($_ -match "^LPAC_${kind}_HANDLE ([0-9a-f``]+)$") { $Matches[1] }
    } | Select-Object -First 1)
    $handleStates[$kind.ToLowerInvariant()] = if ($value.Count -eq 0) { 'UNOBSERVED' }
        elseif (($value[0] -replace '[`0]', '') -eq '') { 'ZERO' }
        else { 'NONZERO' }
}
$orderedRows = [System.Collections.Generic.List[object]]::new()
$inTrace = $false
$inFinalError = $false
$finalErrorHex = $null
$parentStatusHex = $null
$traceRowsTruncated = $false
$traceCallCount = 0
foreach ($line in $raw) {
    if ($line -eq 'LPAC_TRACE_BEGIN') { $inTrace = $true; continue }
    if ($line -eq 'LPAC_TRACE_DONE') { break }
    if (-not $inTrace) { continue }
    if ($line -match '^LPAC_PARENT_STATUS ([0-9a-f]{8})$') {
        $parentStatusHex = $Matches[1]
        $orderedRows.Add([ordered]@{ kind = 'parent_return'; symbol = $null; return_value = "0x$parentStatusHex"; trace_line = $null })
        continue
    }
    if ($line -eq 'LPAC_FINAL_ERROR_BEGIN') { $inFinalError = $true; continue }
    if ($line -eq 'LPAC_FINAL_ERROR_END') { $inFinalError = $false; continue }
    if ($inFinalError) {
        if (-not $finalErrorHex -and $line -match '(?i)(?:last error|lasterror).*?0x([0-9a-f]{1,8})') {
            $finalErrorHex = '0x' + $Matches[1].ToLowerInvariant()
            $orderedRows.Add([ordered]@{ kind = 'final_win32_error'; symbol = $null; return_value = $finalErrorHex; trace_line = $null })
        }
        continue
    }
    $returnValue = $null
    if ($line -match '(?i)(?:return(?: value)?|retval|rax)\s*(?:=|:)\s*(?:0x)?([0-9a-f`]{1,17})') {
        $hexValue = $Matches[1] -replace '`', ''
        $returnValue = if ($hexValue.Length -le 8) { '0x' + $hexValue.ToLowerInvariant() }
            else { 'POINTER_VALUE_REDACTED' }
    }
    if ($line -match '([A-Za-z0-9_.-]+![A-Za-z0-9_?$@.-]+(?:\+0x[0-9a-f]+)?)') {
        $symbol = $Matches[1]
        $traceCallCount++
        $sanitizedLine = ((($line -replace '(?i)\b[0-9a-f]{4,8}`[0-9a-f]{4,8}\b', '<address>') -replace
            '(?i)\b[0-9a-f]{12,16}\b', '<address>') -replace
            '[^A-Za-z0-9_ !+.:=<>,\[\]()-]', '')
        $orderedRows.Add([ordered]@{
            kind = 'trace_call_or_return'
            symbol = $symbol
            return_value = $returnValue
            trace_line = $sanitizedLine.Substring(0, [Math]::Min(220, $sanitizedLine.Length))
        })
        if ($orderedRows.Count -ge 10000) { $traceRowsTruncated = $true; break }
    } elseif ($returnValue) {
        $orderedRows.Add([ordered]@{
            kind = 'trace_return'
            symbol = $null
            return_value = $returnValue
            trace_line = $null
        })
        if ($orderedRows.Count -ge 10000) { $traceRowsTruncated = $true; break }
    }
}
$endpointMatch = $markers -contains 'LPAC_TRACE_ENDPOINT_MATCH'
$endpointThreadMatch = [bool]($ntThread -and @($raw | Where-Object {
    $_ -match "^LPAC_TRACE_ENDPOINT tid=$ntThread`$"
}).Count -eq 1)
$state = if ($instrumentationError -or -not $endpointMatch -or $traceRowsTruncated -or
    -not $endpointThreadMatch -or -not $ntThread -or -not $parentReturn -or $ntStatus.Count -ne 1 -or
    $handleStates.process -eq 'UNOBSERVED' -or $handleStates.thread -eq 'UNOBSERVED' -or
    -not $parentStatusHex -or -not $finalErrorHex -or $traceCallCount -eq 0) {
    'INCOMPLETE'
} else { 'CALIBRATION_ENDPOINT_OBSERVED_ATTRIBUTION_UNVERIFIED' }
$summary = [ordered]@{
    schema = 'gogoke.37.lpac-cdb-calibration.v2'
    commit_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    test_image_sha256 = $testHash
    debugger_version = $cdbVersion
    debugger_sha256 = $cdbHash
    debugger_signature = 'Valid Microsoft'
    exact_test_exit_code = if ($test.HasExited) { $test.ExitCode } else { $null }
    breakpoint_ready = $markers -contains 'LPAC_BREAKPOINTS_READY'
    calibration_phase = $phase
    calibration_state = $state
    instrumentation_error = $instrumentationError
    ntstatus_hex = if ($ntStatus.Count -eq 1) { $ntStatus[0] } else { $null }
    parent_thread_id_redacted = if ($ntThread) { $true } else { $false }
    parent_thread_verified = [bool]($ntThread -and $parentReturn)
    parent_on_nt_stack = [bool]$parentOnNtStack
    output_handle_states = $handleStates
    parent_status_hex = $parentStatusHex
    final_win32_error_hex = $finalErrorHex
    parent_return_endpoint_observed = $endpointMatch
    parent_return_thread_verified = $endpointThreadMatch
    candidate_call_attribution = 'UNVERIFIED'
    markers = $markers
    ordered_trace_rows = $orderedRows.ToArray()
    trace_call_count = $traceCallCount
    trace_rows_truncated = $traceRowsTruncated
    raw_log_private = $true
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'lpac-cdb-summary.json') -Encoding utf8NoBOM
if ($state -eq 'INCOMPLETE') { throw "CDB calibration incomplete at $phase`: $instrumentationError" }
