[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArtifactDirectory,
    [string]$NotesFile,
    [ValidateSet("full", "resources")][string]$ReleaseType = "full",
    [switch]$Publish
)

$ErrorActionPreference = "Stop"
$root = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$artifacts = (Resolve-Path -LiteralPath $ArtifactDirectory).Path
$package = Get-Content (Join-Path $root "apps\desktop\package.json") -Raw | ConvertFrom-Json
$version = [string]$package.version
$manifestName = "SHA256SUMS.windows"
$signatureName = "$manifestName.sig"
$manifest = Join-Path $artifacts $manifestName
$indexName = "resource-index.json"
$packName = "gogoke-resources.windows.zip"
$installerName = "gogoke-$version-windows-x64-unsigned-setup.exe"
$portableName = "gogoke-$version-windows-x64-unsigned-portable.zip"

function Get-ExpectedAssetNames([string]$ManifestVersion, [string]$Type) {
    $names = @("resource-index.json", "gogoke-resources.windows.zip")
    if ($Type -ceq "full") {
        $names += "gogoke-$ManifestVersion-windows-x64-unsigned-setup.exe"
        $names += "gogoke-$ManifestVersion-windows-x64-unsigned-portable.zip"
    }
    return $names
}

function Read-Manifest([string]$Path, [string]$ManifestVersion, [string]$Type, [switch]$RequireHeaders) {
    $text = [IO.File]::ReadAllText($Path, [Text.Encoding]::ASCII)
    $headers = @{}
    $hashes = @{}
    foreach ($line in ($text -split "\r?\n")) {
        if (-not $line) { continue }
        if ($line.StartsWith("# gogoke-", [StringComparison]::Ordinal)) {
            if ($line -cnotmatch '^# gogoke-(Version|Release-Type): ([^\s#]+)$') {
                throw "Manifest contains an unknown or malformed header."
            }
            $key = $Matches[1]
            if ($headers.ContainsKey($key)) { throw "Manifest contains duplicate $key headers." }
            $headers[$key] = $Matches[2]
            continue
        }
        if ($line -cnotmatch '^([0-9a-f]{64})  ([A-Za-z0-9._-]+)$') {
            throw "Manifest contains a malformed checksum entry."
        }
        $name = $Matches[2]
        if ($hashes.ContainsKey($name)) { throw "Manifest contains duplicate asset names." }
        $hashes[$name] = $Matches[1]
    }

    if ($RequireHeaders -and ($headers.Count -ne 2 -or -not $headers.ContainsKey("Version") -or -not $headers.ContainsKey("Release-Type"))) {
        throw "Signed manifest must contain exactly Version and Release-Type headers."
    }
    if ($headers.ContainsKey("Version") -and $headers["Version"] -cne $ManifestVersion) {
        throw "Checksum manifest does not bind version $ManifestVersion"
    }
    if ($headers.ContainsKey("Release-Type") -and $headers["Release-Type"] -cne $Type) {
        throw "Checksum manifest release type does not match -ReleaseType."
    }
    $expected = @(Get-ExpectedAssetNames $ManifestVersion $Type)
    if ($hashes.Count -ne $expected.Count) { throw "Manifest must contain exactly the $Type release assets." }
    foreach ($name in $expected) {
        if (-not $hashes.ContainsKey($name)) { throw "Manifest is missing checksum for $name" }
    }
    foreach ($name in $hashes.Keys) {
        if ($expected -cnotcontains $name) { throw "Manifest contains unexpected asset $name" }
    }
    return @{ Text = $text; Headers = $headers; Hashes = $hashes; AssetNames = $expected }
}

function ConvertFrom-Hex([string]$Value) {
    if ($Value -cnotmatch '^[0-9A-Fa-f]{64}$') { throw 'P-256 key/signature hex has invalid shape.' }
    $bytes = New-Object byte[] ($Value.Length / 2)
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        $bytes[$index] = [Convert]::ToByte($Value.Substring($index * 2, 2), 16)
    }
    return ,$bytes
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

if (-not (Test-SemVer $version)) {
    throw "Desktop package version is not SemVer."
}
$before = Read-Manifest $manifest $version $ReleaseType
$payloadPaths = @{}
foreach ($name in $before.AssetNames) {
    $path = Join-Path $artifacts $name
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing release artifact: $name" }
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -cne $before.Hashes[$name]) { throw "Checksum mismatch for $name" }
    $payloadPaths[$name] = $path
}

$indexPath = $payloadPaths[$indexName]
$packPath = $payloadPaths[$packName]
python (Join-Path $root "tools\ci\gogoke_resource_pack.py") verify --pack $packPath --index $indexPath
if ($LASTEXITCODE -ne 0) { throw "Resource pack/index verification failed." }
$index = Get-Content -LiteralPath $indexPath -Raw | ConvertFrom-Json
if ($index.schema -cne "gogoke.resource-index.v1" -or $index.version -cne $version -or
    $index.pack.fileName -cne $packName) {
    throw "Resource index schema, version, or pack identity does not match this release."
}

if ($ReleaseType -ceq "full") {
    $installer = $payloadPaths[$installerName]
    $portable = $payloadPaths[$portableName]
    if ((Get-AuthenticodeSignature -LiteralPath $installer).Status -ne 'NotSigned') {
        throw "$installerName is not the expected unsigned community artifact"
    }

    $portableInspect = Join-Path ([IO.Path]::GetTempPath()) ("gogoke-portable-inspect-" + [Guid]::NewGuid().ToString("N"))
    try {
        Expand-Archive -LiteralPath $portable -DestinationPath $portableInspect
        $expectedPortableFiles = @("gogoke.exe", "LICENSE", "THIRD_PARTY_NOTICES.md")
        $actualPortableFiles = @(Get-ChildItem -LiteralPath $portableInspect -File -Recurse |
            ForEach-Object { $_.FullName.Substring($portableInspect.Length + 1).Replace('\', '/') } |
            Sort-Object)
        if (($actualPortableFiles -join "`n") -ne (($expectedPortableFiles | Sort-Object) -join "`n")) {
            throw "Portable archive contents do not match the expected gogoke payload"
        }
        $portableExe = Join-Path $portableInspect "gogoke.exe"
        if ((Get-AuthenticodeSignature -LiteralPath $portableExe).Status -ne 'NotSigned') {
            throw "$portableName contains an executable outside the unsigned community trust boundary"
        }
    } finally {
        $resolvedInspect = [IO.Path]::GetFullPath($portableInspect)
        $tempPrefix = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
        if ($resolvedInspect.StartsWith($tempPrefix, [StringComparison]::OrdinalIgnoreCase) -and
            (Test-Path -LiteralPath $resolvedInspect)) {
            Remove-Item -LiteralPath $resolvedInspect -Recurse -Force
        }
    }
}

& (Join-Path $root "tools\sign-gogoke-release-manifest.ps1") `
    -Manifest $manifest -Version $version -ReleaseType $ReleaseType
$signature = Join-Path $artifacts $signatureName
if (-not (Test-Path -LiteralPath $signature -PathType Leaf)) { throw "Manifest signature is missing" }
$signed = Read-Manifest $manifest $version $ReleaseType -RequireHeaders
$publicHex = ([IO.File]::ReadAllText(
    (Join-Path $root "apps\desktop\src-tauri\gogoke-release-public-key.txt"), [Text.Encoding]::ASCII)).Trim()
$signatureHex = ([IO.File]::ReadAllText($signature, [Text.Encoding]::ASCII)).Trim()
$parameters = [System.Security.Cryptography.ECParameters]::new()
$parameters.Curve = [System.Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
$point = [System.Security.Cryptography.ECPoint]::new()
$point.X = ConvertFrom-Hex $publicHex.Substring(0, 64)
$point.Y = ConvertFrom-Hex $publicHex.Substring(64, 64)
$parameters.Q = $point
$verifier = [System.Security.Cryptography.ECDsa]::Create($parameters)
if (-not $verifier.VerifyData(
    [IO.File]::ReadAllBytes($manifest),
    (ConvertFrom-Hex $signatureHex),
    [Security.Cryptography.HashAlgorithmName]::SHA256)) {
    throw "Release manifest signature does not match the public key embedded in gogoke"
}

$releaseAssets = @($payloadPaths.Values | Sort-Object)
$releaseAssets += $manifest, $signature
if (-not $Publish) {
    Write-Output "PASS signed gogoke $ReleaseType release candidate v$version (not published)"
    exit 0
}
if (-not $NotesFile -or -not (Test-Path -LiteralPath $NotesFile -PathType Leaf)) {
    throw "-NotesFile is required with -Publish"
}
& gh release view "v$version" --repo taiyun668/gogoke *> $null
if ($LASTEXITCODE -eq 0) { throw "Release v$version already exists" }
& gh release create "v$version" --repo taiyun668/gogoke --title "gogoke v$version" `
    --notes-file $NotesFile $releaseAssets
if ($LASTEXITCODE -ne 0) { throw "GitHub release publication failed" }
Write-Output "Published gogoke $ReleaseType release v$version"
