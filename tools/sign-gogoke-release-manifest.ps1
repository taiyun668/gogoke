[CmdletBinding()]
param(
    [switch]$NewKey,
    [string]$PrivateKey = (Join-Path $env:USERPROFILE ".gogoke\release-key.txt"),
    [string]$PublicKey = (Join-Path $PSScriptRoot "..\apps\desktop\src-tauri\gogoke-release-public-key.txt"),
    [string]$Manifest,
    [string]$Version,
    [ValidateSet("full", "resources")][string]$ReleaseType = "full"
)

$ErrorActionPreference = "Stop"

function ConvertFrom-HexString([string]$Text, [int]$ExpectedLength = 64) {
    if ($Text -cnotmatch "^[0-9A-Fa-f]{$ExpectedLength}$") {
        throw "Hex value has an invalid shape."
    }
    $bytes = New-Object byte[] ($Text.Length / 2)
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        $bytes[$index] = [Convert]::ToByte($Text.Substring($index * 2, 2), 16)
    }
    return ,$bytes
}

function ConvertTo-HexString([byte[]]$Bytes) {
    return ([BitConverter]::ToString($Bytes).Replace('-', '')).ToLowerInvariant()
}

function Test-SemVer([string]$Value) {
    if ($Value -cnotmatch '^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$') {
        return $false
    }
    if ($Matches[4]) {
        foreach ($identifier in $Matches[4].Split('.')) {
            if ($identifier -cmatch '^[0-9]+$' -and $identifier.Length -gt 1 -and $identifier.StartsWith('0')) {
                return $false
            }
        }
    }
    return $true
}

function Get-ExpectedAssetNames([string]$ManifestVersion, [string]$Type) {
    $names = @("resource-index.json", "gogoke-resources.windows.zip")
    if ($Type -ceq "full") {
        $names += "gogoke-$ManifestVersion-windows-x64-unsigned-setup.exe"
        $names += "gogoke-$ManifestVersion-windows-x64-unsigned-portable.zip"
    }
    return $names
}

function ConvertTo-CanonicalManifest([string]$Text, [string]$ManifestVersion, [string]$Type) {
    if (-not (Test-SemVer $ManifestVersion)) {
        throw "-Version must be SemVer."
    }

    $headers = @{}
    $hashes = @{}
    $lines = $Text -split "\r?\n"
    foreach ($line in $lines) {
        if (-not $line) { continue }
        if ($line.StartsWith("# gogoke-", [StringComparison]::Ordinal)) {
            if ($line -cnotmatch '^# gogoke-(Version|Release-Type): ([^\s#]+)$') {
                throw "Manifest contains an unknown or malformed header."
            }
            $name = $Matches[1]
            if ($headers.ContainsKey($name)) { throw "Manifest contains duplicate $name headers." }
            $headers[$name] = $Matches[2]
            continue
        }
        if ($line -cnotmatch '^([0-9a-f]{64})  ([A-Za-z0-9._+-]+)$') {
            throw "Manifest contains a malformed checksum entry."
        }
        $name = $Matches[2]
        if ($hashes.ContainsKey($name)) { throw "Manifest contains duplicate asset names." }
        $hashes[$name] = $Matches[1]
    }

    if ($headers.ContainsKey("Version") -and $headers["Version"] -cne $ManifestVersion) {
        throw "Manifest version does not match -Version."
    }
    if ($headers.ContainsKey("Release-Type") -and $headers["Release-Type"] -cne $Type) {
        throw "Manifest release type does not match -ReleaseType."
    }

    $expectedNames = @(Get-ExpectedAssetNames $ManifestVersion $Type)
    if ($hashes.Count -ne $expectedNames.Count) {
        throw "Manifest must contain exactly the $Type release assets."
    }
    foreach ($name in $expectedNames) {
        if (-not $hashes.ContainsKey($name)) { throw "Manifest is missing checksum for $name." }
    }
    foreach ($name in $hashes.Keys) {
        if ($expectedNames -cnotcontains $name) { throw "Manifest contains unexpected asset $name." }
    }

    $canonical = @(
        "# gogoke-Version: $ManifestVersion"
        "# gogoke-Release-Type: $Type"
    )
    foreach ($name in $expectedNames) { $canonical += "$($hashes[$name])  $name" }
    return ($canonical -join "`n") + "`n"
}

if ($env:GITHUB_ACTIONS -eq "true") {
    throw "Owner release signing must run offline and cannot run in GitHub Actions."
}
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$privateKeyPath = [IO.Path]::GetFullPath($PrivateKey)
$repoPrefix = $repoRoot.TrimEnd([char[]]@('\', '/')) + [IO.Path]::DirectorySeparatorChar
if ($privateKeyPath.StartsWith($repoPrefix, [StringComparison]::OrdinalIgnoreCase)) {
    throw "The Owner release private key must remain outside the repository."
}

if ($NewKey) {
    if (Test-Path -LiteralPath $PrivateKey) {
        throw "$PrivateKey already exists; refusing to overwrite the gogoke release key."
    }
    if (Test-Path -LiteralPath $PublicKey) {
        throw "$PublicKey already exists; refusing to replace the compiled release trust root."
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
    Write-Output "Created the private gogoke release key outside the repository and wrote its public half to the configured public-key file."
    exit 0
}

if (-not $Manifest -or -not (Test-Path -LiteralPath $Manifest -PathType Leaf)) {
    throw "-Manifest must name an existing SHA256SUMS.windows file."
}
if (-not (Test-Path -LiteralPath $PrivateKey -PathType Leaf)) {
    throw "The offline gogoke release private key is unavailable."
}
$lines = @(Get-Content -LiteralPath $PrivateKey | Where-Object { $_.Trim() })
if ($lines.Count -ne 3) { throw "The gogoke release key must contain D, X, and Y." }
$parameters = [System.Security.Cryptography.ECParameters]::new()
$parameters.Curve = [System.Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
$parameters.D = ConvertFrom-HexString $lines[0].Trim()
$point = [System.Security.Cryptography.ECPoint]::new()
$point.X = ConvertFrom-HexString $lines[1].Trim()
$point.Y = ConvertFrom-HexString $lines[2].Trim()
$parameters.Q = $point
$key = [System.Security.Cryptography.ECDsa]::Create($parameters)

$publicText = ([IO.File]::ReadAllText((Resolve-Path -LiteralPath $PublicKey).Path, [Text.Encoding]::ASCII)).Trim()
$expectedPublic = (ConvertTo-HexString $point.X) + (ConvertTo-HexString $point.Y)
if ($publicText.ToLowerInvariant() -cne $expectedPublic) {
    throw "Offline private key does not match the compiled Owner release public key."
}

$path = (Resolve-Path -LiteralPath $Manifest).Path
$text = [IO.File]::ReadAllText($path, [Text.Encoding]::ASCII)
$canonicalText = ConvertTo-CanonicalManifest $text $Version $ReleaseType
$payload = [Text.Encoding]::ASCII.GetBytes($canonicalText)
$signature = $key.SignData($payload, [Security.Cryptography.HashAlgorithmName]::SHA256)
[IO.File]::WriteAllBytes($path, $payload)
[IO.File]::WriteAllText("$path.sig", (ConvertTo-HexString $signature) + "`n", [Text.Encoding]::ASCII)
Write-Output "Signed the gogoke $ReleaseType release manifest offline."
