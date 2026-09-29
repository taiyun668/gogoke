param(
    [Parameter(Mandatory = $true)][string]$Source,
    [Parameter(Mandatory = $true)][string]$Include
)

$ErrorActionPreference = 'Stop'
$sourceFile = Get-Item -LiteralPath $Source -ErrorAction Stop
if ($sourceFile.PSIsContainer) { throw 'NSIS shell payload is not a file.' }
$snapshot = "$Include.exe"

# Tauri has already patched MAINBINARYSRCPATH from UNK to NSS. Freeze those
# exact bytes once; the generated File instruction and digest use this copy.
$sourceStream = [System.IO.File]::Open($sourceFile.FullName, 'Open', 'Read', 'Read')
try {
    $snapshotStream = [System.IO.File]::Open($snapshot, 'CreateNew', 'Write', 'None')
    try { $sourceStream.CopyTo($snapshotStream); $snapshotStream.Flush($true) }
    finally { $snapshotStream.Dispose() }
}
finally { $sourceStream.Dispose() }

$frozen = Get-Item -LiteralPath $snapshot
if ($frozen.Length -le 0 -or $frozen.Length -ge 2147483648) {
    throw 'NSIS shell payload length is outside the bounded verifier range.'
}
$hashStream = [System.IO.File]::Open($snapshot, 'Open', 'Read', 'Read')
try {
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    try { $hashBytes = $sha256.ComputeHash($hashStream) }
    finally { $sha256.Dispose() }
}
finally { $hashStream.Dispose() }
$hash = [System.BitConverter]::ToString($hashBytes).Replace('-', '').ToLowerInvariant()
if ($hash -cnotmatch '^[0-9a-f]{64}$') { throw 'NSIS shell payload hash is invalid.' }
if ($snapshot.Contains('"') -or $snapshot.Contains('$')) {
    throw 'NSIS shell payload snapshot path cannot be embedded safely.'
}
$lines = @(
    "!define GOGOKE_PREFLIGHT_SNAPSHOT `"$snapshot`""
    "!define GOGOKE_PREFLIGHT_SHA256 `"$hash`""
    "!define GOGOKE_PREFLIGHT_LENGTH $($frozen.Length)"
)
[System.IO.File]::WriteAllLines($Include, $lines, [System.Text.UTF8Encoding]::new($false))
