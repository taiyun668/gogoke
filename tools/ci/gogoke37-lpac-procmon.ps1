param(
    [Parameter(Mandatory = $true)][string]$EvidenceRoot
)

$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $false
$privateRoot = Join-Path $env:RUNNER_TEMP 'gogoke37-lpac-private'
New-Item -ItemType Directory -Path $privateRoot -Force | Out-Null
New-Item -ItemType Directory -Path $EvidenceRoot -Force | Out-Null

$build = & cargo test --locked --manifest-path apps/desktop/native-host/Cargo.toml --lib --no-run --message-format=json 2>&1
if ($LASTEXITCODE -ne 0) { throw 'Exact native test image build failed' }
$images = @($build | ForEach-Object {
    try { $_ | ConvertFrom-Json -ErrorAction Stop } catch { $null }
} | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'gogoke_native_host' -and
    $_.target.kind -contains 'lib' -and $_.profile.test -eq $true -and $_.executable
} | ForEach-Object { $_.executable } | Select-Object -Unique)
if ($images.Count -ne 1 -or -not (Test-Path -LiteralPath $images[0] -PathType Leaf)) {
    throw "Expected exactly one native lib test image; discovered $($images.Count)"
}
$testImage = $images[0]
$imageHash = (Get-FileHash -LiteralPath $testImage -Algorithm SHA256).Hash.ToLowerInvariant()

$archive = Join-Path $privateRoot 'ProcessMonitor.zip'
Invoke-WebRequest -Uri 'https://download.sysinternals.com/files/ProcessMonitor.zip' -OutFile $archive
Expand-Archive -LiteralPath $archive -DestinationPath (Join-Path $privateRoot 'procmon') -Force
$procmon = Join-Path $privateRoot 'procmon/Procmon64.exe'
if (-not (Test-Path -LiteralPath $procmon -PathType Leaf)) { throw 'Microsoft Procmon64.exe missing' }
$signature = Get-AuthenticodeSignature -LiteralPath $procmon
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft') {
    throw 'Microsoft ProcMon signature invalid'
}
$toolHash = (Get-FileHash -LiteralPath $procmon -Algorithm SHA256).Hash.ToLowerInvariant()
$toolVersion = (Get-Item -LiteralPath $procmon).VersionInfo.FileVersion
$trace = Join-Path $privateRoot 'lpac.pml'
$csv = Join-Path $privateRoot 'lpac.csv'
$testLog = Join-Path $privateRoot 'exact-test.log'
$testExit = $null
try {
    Start-Process -FilePath $procmon -WindowStyle Hidden -ArgumentList @(
        '/AcceptEula', '/Quiet', '/Minimized', '/BackingFile', "`"$trace`""
    ) | Out-Null
    Start-Sleep -Seconds 3
    & $testImage --exact process::windows::tests::real_app_container_descendant_reaches_native_seat_pipe --nocapture *> $testLog
    $testExit = $LASTEXITCODE
} finally {
    Start-Process -FilePath $procmon -WindowStyle Hidden -Wait -ArgumentList '/Terminate' | Out-Null
}
if (-not (Test-Path -LiteralPath $trace -PathType Leaf) -or
    (Get-Item -LiteralPath $trace).Length -eq 0) { throw 'ProcMon produced no native trace' }
Start-Process -FilePath $procmon -WindowStyle Hidden -Wait -ArgumentList @(
    '/OpenLog', "`"$trace`"", '/SaveAs', "`"$csv`""
) | Out-Null
if (-not (Test-Path -LiteralPath $csv -PathType Leaf) -or
    (Get-Item -LiteralPath $csv).Length -eq 0) { throw 'ProcMon trace export missing' }

function Convert-PathClass([string]$path) {
    foreach ($prefix in @(
        @{ Value = $env:RUNNER_TEMP; Label = '%RUNNER_TEMP%' },
        @{ Value = $env:WINDIR; Label = '%WINDIR%' },
        @{ Value = $env:USERPROFILE; Label = '%USERPROFILE%' },
        @{ Value = $env:GITHUB_WORKSPACE; Label = '%GITHUB_WORKSPACE%' }
    )) {
        if ($prefix.Value -and $path.StartsWith($prefix.Value, [System.StringComparison]::OrdinalIgnoreCase)) {
            return $prefix.Label + $path.Substring($prefix.Value.Length)
        }
    }
    if ($path.StartsWith('HKLM\', [System.StringComparison]::OrdinalIgnoreCase)) { return $path }
    if ($path.StartsWith('HKCU\', [System.StringComparison]::OrdinalIgnoreCase)) {
        return '%HKCU%' + $path.Substring(4)
    }
    if ($path -match '(?i)(?:\\|^)gogoke\.seat\.v1\.') {
        return '%GOGOKE_SEAT_PIPE%'
    }
    $bytes = [System.Text.Encoding]::UTF8.GetBytes($path)
    $hash = [Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($bytes)).ToLowerInvariant()
    return 'HASH:' + $hash.Substring(0, 16) + '/' + [System.IO.Path]::GetFileName($path)
}

$parent = @(Import-Csv -LiteralPath $csv | Where-Object {
    $_.'Process Name' -eq 'seat-pipe-parent.exe'
})
$denied = @($parent | Where-Object {
    $_.Result -match 'DENIED|PRIVILEGE|BLOCKED|INVALID IMAGE|POLICY'
})
$processEvents = @($parent | Where-Object {
    $_.Operation -match 'Process|Thread|Load Image|CreateFileMapping'
})
$createdChildren = @($parent | Where-Object { $_.Operation -eq 'Process Create' } | ForEach-Object {
    $pidMatch = [regex]::Match($_.Detail, '(?i)(?:^|[,; ]+)PID:\s*(\d+)')
    if ($pidMatch.Success) {
        [ordered]@{
            pid = $pidMatch.Groups[1].Value
            image = Convert-PathClass $_.Path
            result = $_.Result
            time = $_.'Time of Day'
        }
    }
})
$childPids = @($createdChildren | ForEach-Object { $_.pid } | Select-Object -Unique)
$childEvents = @(Import-Csv -LiteralPath $csv | Where-Object { $childPids -contains $_.PID })
$childFailures = @($childEvents | Where-Object {
    $_.Result -match 'DENIED|PRIVILEGE|BLOCKED|INVALID IMAGE|POLICY|DLL NOT FOUND' -or
    $_.Operation -eq 'Process Exit' -or
    ($_.Path -match '(?i)gogoke\.seat\.v1\.' -and $_.Result -ne 'SUCCESS')
})
$pipeEvents = @($childEvents | Where-Object { $_.Path -match '(?i)gogoke\.seat\.v1\.' })
$childLifecycle = @($childEvents | Where-Object {
    $_.Operation -match 'Process|Thread|Load Image' -or
    $_.Result -match 'DENIED|PRIVILEGE|BLOCKED|INVALID IMAGE|POLICY|DLL NOT FOUND'
})
$selected = @($denied | Select-Object -First 100 | ForEach-Object {
    [ordered]@{
        time = $_.'Time of Day'
        pid = $_.PID
        operation = $_.Operation
        object = Convert-PathClass $_.Path
        result = $_.Result
    }
})
$summary = [ordered]@{
    schema = 'gogoke.37.lpac-procmon-diagnostic.v1'
    source_sha = $env:GITHUB_SHA
    test_image_sha256 = $imageHash
    exact_test_exit = $testExit
    test_only_registry_read = ($env:GOGOKE_TEST_LPAC_REGISTRY_READ -eq '1')
    procmon_version = $toolVersion
    procmon_sha256 = $toolHash
    procmon_signature = 'Valid Microsoft'
    trace_bytes = (Get-Item -LiteralPath $trace).Length
    parent_event_count = $parent.Count
    parent_denied_count = $denied.Count
    parent_process_event_count = $processEvents.Count
    created_children = $createdChildren
    child_event_count = $childEvents.Count
    selected_child_failures = @($childFailures | Select-Object -First 100 | ForEach-Object {
        [ordered]@{
            time = $_.'Time of Day'
            pid = $_.PID
            operation = $_.Operation
            object = Convert-PathClass $_.Path
            result = $_.Result
        }
    })
    selected_seat_pipe_events = @($pipeEvents | Select-Object -Last 40 | ForEach-Object {
        [ordered]@{
            time = $_.'Time of Day'
            pid = $_.PID
            operation = $_.Operation
            object = Convert-PathClass $_.Path
            result = $_.Result
        }
    })
    selected_child_lifecycle = @($childLifecycle | Select-Object -First 160 | ForEach-Object {
        $exitMatch = [regex]::Match($_.Detail, '(?i)Exit Status:\s*(0x[0-9a-f]+|\d+)')
        [ordered]@{
            time = $_.'Time of Day'
            pid = $_.PID
            operation = $_.Operation
            object = Convert-PathClass $_.Path
            result = $_.Result
            exit_status = if ($exitMatch.Success) { $exitMatch.Groups[1].Value } else { $null }
        }
    })
    selected_process_events = @($processEvents | Select-Object -Last 40 | ForEach-Object {
        [ordered]@{
            time = $_.'Time of Day'
            pid = $_.PID
            operation = $_.Operation
            object = Convert-PathClass $_.Path
            result = $_.Result
        }
    })
    selected_denied = $selected
    raw_trace_uploaded = $false
}
$summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $EvidenceRoot 'lpac-procmon-summary.json') -Encoding utf8NoBOM
if ($parent.Count -eq 0) { throw 'ProcMon trace has no exact LPAC parent events' }
if ($testExit -notin @(0, 101)) { throw "Exact test exit was unexpected: $testExit" }
exit 0
