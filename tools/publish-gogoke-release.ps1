[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ArtifactDirectory,
    [string]$NotesFile,
    [switch]$Publish
)

$ErrorActionPreference = "Stop"
$root = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$artifacts = (Resolve-Path -LiteralPath $ArtifactDirectory).Path
$package = Get-Content (Join-Path $root "apps\desktop\package.json") -Raw | ConvertFrom-Json
$version = [string]$package.version
$installerName = "gogoke-$version-windows-x64-unsigned-setup.exe"
$portableName = "gogoke-$version-windows-x64-unsigned-portable.zip"
$manifestName = "SHA256SUMS.windows"
$signatureName = "$manifestName.sig"
$installer = Join-Path $artifacts $installerName
$portable = Join-Path $artifacts $portableName
$manifest = Join-Path $artifacts $manifestName

foreach ($path in @($installer, $portable, $manifest)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Missing release artifact: $path" }
}
$text = [IO.File]::ReadAllText($manifest, [Text.Encoding]::ASCII)
$versions = @([regex]::Matches($text, '(?m)^# gogoke-Version:\s*([^\s#]+)\s*$'))
if ($versions.Count -ne 1 -or $versions[0].Groups[1].Value -ne $version) {
    throw "Checksum manifest does not bind version $version"
}
foreach ($path in @($installer, $portable)) {
    $name = Split-Path -Leaf $path
    $matches = @($text -split "`r?`n" | Where-Object { $_ -match "^[0-9A-Fa-f]{64}\s+\*?$([regex]::Escape($name))$" })
    if ($matches.Count -ne 1) { throw "Manifest must contain one checksum for $name" }
    $expected = ($matches[0] -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected) { throw "Checksum mismatch for $name" }
}
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

& (Join-Path $root "tools\sign-gogoke-release-manifest.ps1") -Manifest $manifest -Version $version
$signature = Join-Path $artifacts $signatureName
if (-not (Test-Path -LiteralPath $signature -PathType Leaf)) { throw "Manifest signature is missing" }
$publicHex = ([IO.File]::ReadAllText(
    (Join-Path $root "apps\desktop\src-tauri\gogoke-release-public-key.txt"))).Trim()
$signatureHex = ([IO.File]::ReadAllText($signature)).Trim()
function ConvertFrom-Hex([string]$value) {
    $bytes = New-Object byte[] ($value.Length / 2)
    for ($index = 0; $index -lt $bytes.Length; $index++) {
        $bytes[$index] = [Convert]::ToByte($value.Substring($index * 2, 2), 16)
    }
    return ,$bytes
}
$parameters = New-Object Security.Cryptography.ECParameters
$parameters.Curve = [Security.Cryptography.ECCurve]::CreateFromFriendlyName('nistP256')
$point = New-Object Security.Cryptography.ECPoint
$point.X = ConvertFrom-Hex $publicHex.Substring(0, 64)
$point.Y = ConvertFrom-Hex $publicHex.Substring(64, 64)
$parameters.Q = $point
$verifier = [Security.Cryptography.ECDsa]::Create($parameters)
if (-not $verifier.VerifyData(
    [IO.File]::ReadAllBytes($manifest),
    (ConvertFrom-Hex $signatureHex),
    [Security.Cryptography.HashAlgorithmName]::SHA256)) {
    throw "Release manifest signature does not match the public key embedded in gogoke"
}

if (-not $Publish) {
    Write-Output "PASS signed gogoke release candidate v$version (not published)"
    exit 0
}
if (-not $NotesFile -or -not (Test-Path -LiteralPath $NotesFile -PathType Leaf)) {
    throw "-NotesFile is required with -Publish"
}
& gh release view "v$version" --repo taiyun668/gogoke *> $null
if ($LASTEXITCODE -eq 0) { throw "Release v$version already exists" }
& gh release create "v$version" --repo taiyun668/gogoke --title "gogoke v$version" `
    --notes-file $NotesFile $installer $portable $manifest $signature
if ($LASTEXITCODE -ne 0) { throw "GitHub release publication failed" }
Write-Output "Published gogoke v$version"
