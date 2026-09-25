# Fixed, embedded finalizer. This WIP deliberately stops before deletion.
# PowerShell 5.1 has no built-in verify-then-delete-by-handle operation. Its
# FileOptions.DeleteOnClose irrevocably arms deletion before same-handle hash
# verification, and Add-Type would compile native code on the Owner machine.
$ErrorActionPreference = 'Stop'

function Same([string]$a, [string]$b) {
    [string]::Equals($a, $b, [StringComparison]::OrdinalIgnoreCase)
}
function Assert-NoReparse([string]$path) {
    $current = [IO.Path]::GetFullPath($path)
    while ($current) {
        $attributes = [IO.File]::GetAttributes($current)
        if (($attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
            throw 'GOGOKE_UNINSTALL_REPARSE_POINT'
        }
        $parent = [IO.Path]::GetDirectoryName($current)
        if (-not $parent -or (Same $parent $current)) { break }
        $current = $parent
    }
}
function Assert-Instance {
    $key = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\' + $data.registryKey
    $record = Get-ItemProperty -LiteralPath $key -ErrorAction Stop
    if (-not (Same ([IO.Path]::GetFullPath([string]$record.InstallLocation)) $data.root) -or
        -not (Same ([string]$record.InstallInstanceId) $data.instance) -or
        -not (Same ([string]$record.InstallDomain) $data.domain)) {
        throw 'GOGOKE_UNINSTALL_INSTANCE_CHANGED'
    }
    Assert-NoReparse $data.root
}
function Write-Receipt([string]$state, [string]$detail) {
    if (-not $data) { return }
    $record = @{ schema = 'gogoke.uninstall-result.v1'; state = $state;
        detail = $detail.Substring(0, [Math]::Min($detail.Length, 256));
        domain = $data.domain; atUtc = [DateTime]::UtcNow.ToString('o') } | ConvertTo-Json -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($record)
    if ($bytes.Length -gt 2048) { return }
    try {
        $stream = [IO.File]::Open($data.receipt, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
        try { $stream.Write($bytes, 0, $bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
    } catch { }
}

$data = $null
try {
    $line = [Console]::In.ReadLine()
    if (-not $line -or $line.Length -gt 1048576) { throw 'GOGOKE_UNINSTALL_HANDOFF_INVALID' }
    $data = $line | ConvertFrom-Json
    if ($data.registryKey -cnotin @('gogoke','gogoke-candidate') -or
        $data.files.Count -lt 1 -or $data.files.Count -gt 100000 -or
        $data.nonce -notmatch '^[0-9a-f-]{36}$') { throw 'GOGOKE_UNINSTALL_HANDOFF_INVALID' }
    $expectedDomain = if ($data.registryKey -ceq 'gogoke') { 'OWNER_RELEASE' } else { 'CI_CANDIDATE_RESOURCE' }
    if ($data.domain -cne $expectedDomain) { throw 'GOGOKE_UNINSTALL_DOMAIN_CHANGED' }
    if (-not (Same ([IO.Path]::GetFullPath([string]$data.lockPath)) ([IO.Path]::Combine([IO.Path]::GetDirectoryName($data.root), 'gogoke-install-lifecycle.lock')))) {
        throw 'GOGOKE_UNINSTALL_LOCK_PATH_CHANGED'
    }
    Assert-NoReparse ([IO.Path]::GetDirectoryName($data.lockPath))
    Assert-Instance
    $rootPrefix = [IO.Path]::GetFullPath($data.root).TrimEnd('\') + '\'
    foreach ($file in $data.files) {
        $path = [IO.Path]::GetFullPath([string]$file.path)
        if (-not $path.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            [string]$file.sha256 -cnotmatch '^[0-9a-f]{64}$') { throw 'GOGOKE_UNINSTALL_INVENTORY_INVALID' }
        Assert-NoReparse $path
    }
    # stderr is the duplicate of the share-mode-zero lifecycle lock. The
    # parent reads this marker from its original handle before exiting.
    $lockStream = [Console]::OpenStandardError()
    $marker = [Text.Encoding]::ASCII.GetBytes('LOCK:' + $data.nonce + "`n")
    $lockStream.Write($marker, 0, $marker.Length)
    $lockStream.Flush()
    $parent = [Diagnostics.Process]::GetProcessById([int]$data.parentPid)
    $null = $parent.Handle
    [Console]::Out.WriteLine('READY:' + $data.nonce)
    [Console]::Out.Flush()
    if (-not $parent.WaitForExit(120000)) { throw 'GOGOKE_UNINSTALL_PARENT_STILL_RUNNING' }
    Assert-Instance
    Write-Receipt 'NOT_VERIFIED' 'safe verify-then-delete-by-handle finalizer unavailable'
    exit 2
} catch {
    Write-Receipt 'FAILED' ([string]$_.Exception.Message)
    exit 1
}
