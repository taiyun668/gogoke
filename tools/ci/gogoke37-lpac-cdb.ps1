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
bu ntdll!NtTerminateProcess ".echo LPAC_TERM; r rcx; r edx; k 12; gc"
bu ntdll!RtlSetLastWin32ErrorAndNtStatusFromNtStatus ".if (@ecx == 0xc0000022) { .echo LPAC_NT_DENIED; k 12 }; gc"
bu ntdll!RtlSetLastWin32Error ".if (@ecx == 5) { .echo LPAC_WIN32_DENIED; k 12 }; gc"
bu ntdll!NtCreateUserProcess ".echo LPAC_CREATE_ENTER; k 8; gc"
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
    $debugger = Start-Process -FilePath $cdb -ArgumentList @(
        '-p', "$parentPid", '-G', '-cf', "`"$commands`"", '-logo', "`"$debugLog`""
    ) -WindowStyle Hidden -PassThru
    $deadline = (Get-Date).AddSeconds(45)
    while (-not ((Test-Path -LiteralPath $debugLog -PathType Leaf) -and
        ((Get-Content -LiteralPath $debugLog -Raw) -match 'LPAC_BREAKPOINTS_READY'))) {
        if ($debugger.HasExited) { throw 'CDB exited before installing LPAC breakpoints' }
        if ((Get-Date) -gt $deadline) { throw 'CDB breakpoint installation timed out' }
        Start-Sleep -Milliseconds 100
    }
    New-Item -ItemType File -Path (Join-Path $gate 'release') -Force | Out-Null
    if (-not $test.WaitForExit(150000)) { throw 'Exact LPAC diagnostic test timed out' }
    if ($debugger -and -not $debugger.WaitForExit(15000)) { $debugger.Kill() }
} finally {
    if (-not $test.HasExited) { $test.Kill() }
    if ($debugger -and -not $debugger.HasExited) { $debugger.Kill() }
}

$raw = if (Test-Path -LiteralPath $debugLog -PathType Leaf) {
    Get-Content -LiteralPath $debugLog
} else { @() }
$markers = @($raw | Where-Object { $_ -match '^LPAC_(TERM|NT_DENIED|WIN32_DENIED|CREATE_ENTER)' } |
    Select-Object -First 100)
$stackSymbols = @($raw | ForEach-Object {
    if ($_ -match '^\s*[0-9a-f`]+\s+[0-9a-f`]+\s+([A-Za-z0-9_.-]+![A-Za-z0-9_?$@.+-]+)') {
        $Matches[1]
    }
} | Select-Object -Unique -First 100)
$summary = [ordered]@{
    schema = 'gogoke.37.lpac-cdb-diagnostic.v1'
    commit_sha = $env:GITHUB_SHA
    run_id = $env:GITHUB_RUN_ID
    test_image_sha256 = $testHash
    debugger_version = $cdbVersion
    debugger_sha256 = $cdbHash
    debugger_signature = 'Valid Microsoft'
    exact_test_exit_code = $test.ExitCode
    breakpoint_ready = ($raw -match 'LPAC_BREAKPOINTS_READY').Count -gt 0
    markers = $markers
    stack_symbols = $stackSymbols
    raw_log_private = $true
}
$summary | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'lpac-cdb-summary.json') -Encoding utf8NoBOM
if (-not $summary.breakpoint_ready) { throw 'CDB did not prove installed breakpoints' }
