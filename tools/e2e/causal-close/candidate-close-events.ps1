param(
    [Parameter(Mandatory)][string]$Installed,
    [Parameter(Mandatory)][int]$ShellPid,
    [Parameter(Mandatory)][string]$ShellSha256,
    [Parameter(Mandatory)][string]$NativeSha256
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class GogokeCloseWait {
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern uint WaitForMultipleObjects(uint count, IntPtr[] handles, bool waitAll, uint milliseconds);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool GetExitCodeProcess(IntPtr handle, out uint code);
}
'@

function Emit($Value) {
    [Console]::Out.WriteLine(($Value | ConvertTo-Json -Compress -Depth 8))
    [Console]::Out.Flush()
}

function Read-CausalArmFrame {
    param(
        [Parameter(Mandatory)][string]$Line,
        [Parameter(Mandatory)][int]$ExpectedShellPid,
        [Parameter(Mandatory)][string]$ExpectedShellCreationUtc,
        [Parameter(Mandatory)][int]$ExpectedNativePid,
        [Parameter(Mandatory)][string]$ExpectedNativeCreationUtc
    )
    # PowerShell 7.6.5 otherwise converts ISO JSON strings to DateTime and a
    # subsequent string cast loses all fractional creation-time digits.
    $arm = ConvertFrom-Json -InputObject $Line -DateKind String
    $names = @($arm.PSObject.Properties.Name)
    $expectedNames = @('kind', 'token', 'shellPid', 'shellCreationUtc',
        'nativePid', 'nativeCreationUtc')
    $utcPattern = '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{7}Z$'
    if ($names.Count -ne $expectedNames.Count -or
        @($expectedNames | Where-Object { $_ -cnotin $names }).Count -ne 0 -or
        $arm.kind -isnot [string] -or $arm.kind -cne 'ARM' -or
        $arm.token -isnot [string] -or $arm.token -cnotmatch '^[A-Za-z0-9_-]{16,128}$' -or
        $arm.shellPid -isnot [long] -or $arm.shellPid -ne $ExpectedShellPid -or
        $arm.nativePid -isnot [long] -or $arm.nativePid -ne $ExpectedNativePid -or
        $arm.shellCreationUtc -isnot [string] -or
        $arm.nativeCreationUtc -isnot [string] -or
        $arm.shellCreationUtc -cnotmatch $utcPattern -or
        $arm.nativeCreationUtc -cnotmatch $utcPattern -or
        $ExpectedShellCreationUtc -cnotmatch $utcPattern -or
        $ExpectedNativeCreationUtc -cnotmatch $utcPattern -or
        $arm.shellCreationUtc -cne $ExpectedShellCreationUtc -or
        $arm.nativeCreationUtc -cne $ExpectedNativeCreationUtc) {
        throw 'Original candidate close ARM token or retained object identity differs'
    }
    return $arm
}

function CreationKey([datetime]$Value) {
    # CIM reports microseconds; the retained process handle exposes 100 ns.
    # Compare the exact CIM precision rather than a time-window tolerance.
    return $Value.ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.ffffffZ',
        [Globalization.CultureInfo]::InvariantCulture)
}

function ExactImage([string]$Path, [string]$ExpectedSha256) {
    $item = Get-Item -LiteralPath $Path -Force
    if (!$item -or $item.PSIsContainer -or
        ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or
        (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ExpectedSha256) {
        throw 'Candidate process executable is not the pinned physical image'
    }
    $fileIdText = ((& fsutil file queryfileid $Path) -join ' ').Trim()
    $fileId = [regex]::Match($fileIdText, '0x[0-9a-fA-F]{32}').Value.ToLowerInvariant()
    $volume = Get-CimInstance Win32_LogicalDisk -Filter ("DeviceID='" + $item.PSDrive.Name + ":'")
    if ($LASTEXITCODE -ne 0 -or !$fileId -or !$volume -or !$volume.VolumeSerialNumber) {
        throw 'Candidate process executable physical FileID is unavailable'
    }
    return [ordered]@{path = $item.FullName; sha256 = $ExpectedSha256;
        volumeSerial = [string]$volume.VolumeSerialNumber; fileId = $fileId}
}

function Capture($Row, [string]$Role, [string]$ExpectedPath, [string]$ExpectedSha256) {
    $process = [Diagnostics.Process]::GetProcessById([int]$Row.ProcessId)
    # Accessing Handle here pins the OS process object before READY/caption.
    $handle = $process.Handle
    if ($handle -eq [IntPtr]::Zero -or $process.HasExited) {
        throw "Original $Role process handle is absent before caption"
    }
    $actualPath = [IO.Path]::GetFullPath($process.MainModule.FileName)
    if ($actualPath -ine [IO.Path]::GetFullPath($ExpectedPath)) {
        throw "Original $Role process image path differs"
    }
    $observed = Get-CimInstance Win32_Process -Filter ("ProcessId=" + $Row.ProcessId)
    if (!$observed -or (CreationKey $observed.CreationDate) -cne (CreationKey $Row.CreationDate) -or
        (CreationKey $process.StartTime) -cne (CreationKey $observed.CreationDate) -or
        [int]$observed.ParentProcessId -ne [int]$Row.ParentProcessId -or
        [IO.Path]::GetFullPath([string]$observed.ExecutablePath) -ine $actualPath -or
        $process.HasExited) {
        throw "Original $Role PID/creation/image changed while opening handle"
    }
    $image = ExactImage $actualPath $ExpectedSha256
    if ($process.HasExited) { throw "Original $Role exited before caption readiness" }
    return [pscustomobject]@{role = $Role; process = $process; handle = $handle;
        evidence = [ordered]@{role = $Role; pid = [int]$Row.ProcessId;
            creationUtc = $process.StartTime.ToUniversalTime().ToString('o');
            cimCreationUtc = $Row.CreationDate.ToUniversalTime().ToString('o');
            parentPid = [int]$Row.ParentProcessId;
            image = $image}}
}

if ($ShellPid -le 0 -or $ShellSha256 -cnotmatch '^[a-f0-9]{64}$' -or
    $NativeSha256 -cnotmatch '^[a-f0-9]{64}$') {
    throw 'Candidate close requires exact process/image pins'
}
$root = [IO.Path]::GetFullPath($Installed).TrimEnd('\')
if ((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw 'Candidate installation root is reparse'
}
$shellPath = Join-Path $root 'gogoke.exe'
$nativePath = Join-Path $root 'gogoke-native-host.exe'
$all = @(Get-CimInstance Win32_Process)
$byPid = @{}
foreach ($row in $all) { $byPid[[int]$row.ProcessId] = $row }
if (!$byPid.ContainsKey($ShellPid) -or
    [IO.Path]::GetFullPath([string]$byPid[$ShellPid].ExecutablePath) -ine $shellPath) {
    throw 'Original installed shell PID/image absent before caption'
}
$lineages = @{}
$nativeRows = @(foreach ($row in $all) {
    if (!$row.ExecutablePath -or
        [IO.Path]::GetFullPath([string]$row.ExecutablePath) -ine $nativePath) { continue }
    $parent = [int]$row.ParentProcessId
    $seen = [Collections.Generic.HashSet[int]]::new()
    $child = $row
    $lineage = [Collections.Generic.List[object]]::new()
    while ($parent -gt 0 -and $seen.Add($parent)) {
        if (!$byPid.ContainsKey($parent)) { break }
        $ancestor = $byPid[$parent]
        if ($ancestor.CreationDate -ge $child.CreationDate) {
            throw 'Candidate native ancestry has a reused or younger parent PID'
        }
        $lineage.Add($ancestor)
        if ($parent -eq $ShellPid) {
            $lineages[[int]$row.ProcessId] = @($lineage.ToArray())
            $row
            break
        }
        $child = $ancestor
        $parent = [int]$ancestor.ParentProcessId
    }
})
if ($nativeRows.Count -ne 1) { throw 'Original installed native descendant is missing or ambiguous' }
$shell = Capture $byPid[$ShellPid] 'shell' $shellPath $ShellSha256
$native = Capture $nativeRows[0] 'native' $nativePath $NativeSha256
$ancestry = @()
foreach ($ancestor in $lineages[[int]$nativeRows[0].ProcessId]) {
    $again = Get-CimInstance Win32_Process -Filter ("ProcessId=" + $ancestor.ProcessId)
    if (!$again -or (CreationKey $again.CreationDate) -cne (CreationKey $ancestor.CreationDate) -or
        [int]$again.ParentProcessId -ne [int]$ancestor.ParentProcessId -or
        [IO.Path]::GetFullPath([string]$again.ExecutablePath) -ine
            [IO.Path]::GetFullPath([string]$ancestor.ExecutablePath)) {
        throw 'Original native ancestor PID/creation/image changed before caption'
    }
    $ancestry += [ordered]@{pid = [int]$again.ProcessId;
        creationUtc = $again.CreationDate.ToUniversalTime().ToString('o');
        parentPid = [int]$again.ParentProcessId}
}
if ($shell.process.HasExited -or $native.process.HasExited) {
    throw 'Original process exited before both close handles were retained'
}
Emit ([ordered]@{kind = 'READY'; observedUtc = [DateTime]::UtcNow.ToString('o');
    shell = $shell.evidence; native = $native.evidence;
    nativeAncestorChain = $ancestry})

$remaining = [Collections.Generic.List[object]]::new()
$remaining.Add($shell)
$remaining.Add($native)
$clock = [Diagnostics.Stopwatch]::StartNew()
$armRead = [Console]::In.ReadLineAsync()
$armed = $false
while ($remaining.Count -gt 0) {
    $left = 30000 - $clock.ElapsedMilliseconds
    if ($left -le 0) { throw 'Original candidate close process exit observation deadline' }
    if (!$armed -and $armRead.IsCompleted) {
        $line = $armRead.GetAwaiter().GetResult()
        if (!$line) { throw 'Original candidate close ARM input is absent' }
        $arm = Read-CausalArmFrame -Line $line -ExpectedShellPid $shell.evidence.pid `
            -ExpectedShellCreationUtc $shell.evidence.creationUtc `
            -ExpectedNativePid $native.evidence.pid `
            -ExpectedNativeCreationUtc $native.evidence.creationUtc
        $allHandles = [IntPtr[]]@($shell.handle, $native.handle)
        $zero = [GogokeCloseWait]::WaitForMultipleObjects([uint32]2,
            $allHandles, $false, [uint32]0)
        if ($zero -ne 258) {
            throw "Original candidate close ARM refused: retained object wait=$zero"
        }
        $armed = $true
        Emit ([ordered]@{kind = 'ARMED'; token = [string]$arm.token;
            waitResult = 'WAIT_TIMEOUT'; waitMilliseconds = 0;
            shellPid = $shell.evidence.pid; shellCreationUtc = $shell.evidence.creationUtc;
            nativePid = $native.evidence.pid; nativeCreationUtc = $native.evidence.creationUtc;
            observedUtc = [DateTime]::UtcNow.ToString('o')})
    }
    $handles = [IntPtr[]]@($remaining | ForEach-Object { $_.handle })
    $result = [GogokeCloseWait]::WaitForMultipleObjects([uint32]$handles.Length,
        $handles, $false, [uint32][Math]::Min($left, $(if ($armed) { $left } else { 50 })))
    if ($result -eq 258) { continue }
    if ($result -eq [uint32]::MaxValue -or $result -ge $handles.Length) {
        throw 'Original candidate close process wait failed'
    }
    if (!$armed) { throw 'Original candidate process exited before ARM zero wait' }
    $ended = $remaining[[int]$result]
    $at = [DateTime]::UtcNow.ToString('o')
    $code = [uint32]0
    $readCode = [GogokeCloseWait]::GetExitCodeProcess($ended.handle, [ref]$code)
    if (!$readCode) {
        $win32 = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "Original $($ended.role) exit code is UNKNOWN; GetExitCodeProcess Win32=$win32"
    }
    if ($code -eq 259) { throw 'Signaled original process still reports active' }
    Emit ([ordered]@{kind = 'EXIT'; role = $ended.role; pid = $ended.evidence.pid;
        creationUtc = $ended.evidence.creationUtc; observedUtc = $at;
        code = [int64]$code; codeBasis = 'GET_EXIT_CODE_PROCESS'})
    $remaining.RemoveAt([int]$result)
}
$leftovers = @(Get-CimInstance Win32_Process | Where-Object {
    $_.ExecutablePath -and
    ([IO.Path]::GetFullPath([string]$_.ExecutablePath) -ieq $shellPath -or
     [IO.Path]::GetFullPath([string]$_.ExecutablePath) -ieq $nativePath)
})
if ($leftovers.Count -ne 0) { throw 'Installed candidate process remains after original exit events' }
Emit ([ordered]@{kind = 'COMPLETE'; observedUtc = [DateTime]::UtcNow.ToString('o');
    installedCandidateProcesses = 0})
