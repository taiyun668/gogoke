[CmdletBinding()]
param(
    [switch]$NewKey,
    [string]$PrivateKey = (Join-Path $env:USERPROFILE ".gogoke\release-key.txt"),
    [string]$PublicKey = (Join-Path $PSScriptRoot "..\apps\desktop\src-tauri\gogoke-release-public-key.txt"),
    [string]$Manifest,
    [string]$Version
)

$ErrorActionPreference = "Stop"

function ConvertFrom-HexString([string]$text) {
    $bytes = New-Object byte[] ($text.Length / 2)
    for ($i = 0; $i -lt $bytes.Length; $i++) {
        $bytes[$i] = [Convert]::ToByte($text.Substring($i * 2, 2), 16)
    }
    return $bytes
}

if ($NewKey) {
    if (Test-Path -LiteralPath $PrivateKey) {
        throw "$PrivateKey already exists; refusing to overwrite the gogoke release key."
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $PrivateKey) | Out-Null
    $key = [System.Security.Cryptography.ECDsa]::Create(
        [System.Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256'))
    $full = $key.ExportParameters($true)
    Set-Content -LiteralPath $PrivateKey -Encoding ascii -Value @(
        [BitConverter]::ToString($full.D).Replace('-', ''),
        [BitConverter]::ToString($full.Q.X).Replace('-', ''),
        [BitConverter]::ToString($full.Q.Y).Replace('-', '')
    )
    $public = ([BitConverter]::ToString($full.Q.X) + [BitConverter]::ToString($full.Q.Y)).Replace('-', '')
    Set-Content -LiteralPath $PublicKey -Encoding ascii -Value $public
    Write-Output "Created the private gogoke release key outside the repository and wrote only its public half into the repository."
    exit 0
}

if (-not $Manifest -or -not (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
    throw "-Manifest must name an existing SHA256SUMS.windows file."
}
if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw "-Version must be SemVer."
}
$lines = @(Get-Content -LiteralPath $PrivateKey | Where-Object { $_.Trim() })
if ($lines.Count -ne 3) { throw "The gogoke release key must contain D, X, and Y." }
$parameters = New-Object System.Security.Cryptography.ECParameters
$parameters.Curve = [System.Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
$parameters.D = ConvertFrom-HexString $lines[0].Trim()
$point = New-Object System.Security.Cryptography.ECPoint
$point.X = ConvertFrom-HexString $lines[1].Trim()
$point.Y = ConvertFrom-HexString $lines[2].Trim()
$parameters.Q = $point
$key = [System.Security.Cryptography.ECDsa]::Create($parameters)
$path = (Resolve-Path -LiteralPath $Manifest).Path
$text = [IO.File]::ReadAllText($path, [Text.Encoding]::ASCII)
$versionLine = "# gogoke-Version: $Version`n"
$existing = @([regex]::Matches($text, '(?m)^# gogoke-Version:\s*([^\s#]+)\s*$'))
if ($existing.Count -gt 1) { throw "Manifest contains multiple gogoke version lines." }
if ($existing.Count -eq 1 -and $existing[0].Groups[1].Value -ne $Version) {
    throw "Manifest version does not match -Version."
}
if ($existing.Count -eq 0) {
    [IO.File]::WriteAllText($path, $versionLine + $text, [Text.Encoding]::ASCII)
}
$payload = [IO.File]::ReadAllBytes($path)
$signature = $key.SignData($payload, [Security.Cryptography.HashAlgorithmName]::SHA256)
Set-Content -LiteralPath "$path.sig" -Encoding ascii -Value ([BitConverter]::ToString($signature).Replace('-', ''))
Write-Output "Signed the gogoke desktop release manifest."
