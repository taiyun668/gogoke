# Cloud-only launcher for the existing installed-product smoke.
# WebView2 150+ ignores diagnostic environment overrides in elevated hosts:
# https://github.com/tauri-apps/wry/issues/1782
# Lower the test process token; never change WebView2/UAC/SAC policy or the product.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstallDirectory,
    [switch]$MediumChild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true') {
    throw 'Installed UI smoke launcher is Windows cloud CI only'
}
$installed = (Resolve-Path -LiteralPath $InstallDirectory).Path
$smoke = Join-Path $PSScriptRoot 'gogoke-package-service.mjs'
$receipt = $null

if ($MediumChild) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Installed UI smoke is still elevated; no product request was sent'
        }
    } finally { $identity.Dispose() }
    Write-Output 'R2-04 installed UI smoke: non-administrator cloud process'
    & node $smoke smoke $installed
    if ($null -eq $LASTEXITCODE) { throw 'Installed smoke exit status unavailable; do not resend' }
    exit $LASTEXITCODE
}

# Portable test tooling only: not installed, placed on PATH, or shipped in Gogoke.
# Pin both the upstream release and its published archive digest.
$tools = Join-Path $env:RUNNER_TEMP ('gogoke-smoke-token-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tools | Out-Null
try {
    $archive = Join-Path $tools 'gsudo.portable.zip'
    Invoke-WebRequest -Uri 'https://github.com/gerardog/gsudo/releases/download/v2.6.1/gsudo.portable.zip' -OutFile $archive
    $expected = '21130bf178d7b9891207f00bff56f05b6b363ce9cec7b2d084e45fb12ee51f44'
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -cne $expected) {
        throw 'Pinned cloud token launcher archive digest mismatch'
    }
    Expand-Archive -LiteralPath $archive -DestinationPath $tools
    $launcher = Join-Path $tools 'x64/gsudo.exe'
    if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) {
        throw 'Pinned x64 cloud token launcher missing'
    }
    $shell = Join-Path $PSHOME 'pwsh.exe'
    $receipt = Join-Path $env:RUNNER_TEMP ('gogoke-r2-smoke-receipt-' + [Guid]::NewGuid().ToString('N') + '.json')
    if (Test-Path -LiteralPath $receipt) { throw 'Fresh installed smoke receipt path already exists' }
    $env:GOGOKE_R2_SMOKE_RECEIPT = $receipt
    & $launcher --integrity Medium --direct $shell -NoProfile -File $PSCommandPath -InstallDirectory $installed -MediumChild
    $childExit = $LASTEXITCODE
    Remove-Item Env:GOGOKE_R2_SMOKE_RECEIPT -ErrorAction SilentlyContinue
    if ($null -eq $childExit) { throw 'Cloud smoke child exit status unavailable; do not resend' }
    if (-not (Test-Path -LiteralPath $receipt -PathType Leaf)) {
        throw "Non-administrator installed smoke produced no receipt; exit $childExit"
    }
    & node $smoke record-smoke $receipt
    if ($null -eq $LASTEXITCODE) { throw 'Installed smoke receipt recording status unavailable' }
    if ($LASTEXITCODE -ne 0) { throw "Installed smoke receipt recording failed: $LASTEXITCODE" }
    if ($childExit -ne 0) { throw "Non-administrator installed smoke failed: $childExit" }
} finally {
    Remove-Item Env:GOGOKE_R2_SMOKE_RECEIPT -ErrorAction SilentlyContinue
    if ($receipt -and (Test-Path -LiteralPath $receipt -PathType Leaf)) { Remove-Item -LiteralPath $receipt -Force }
    Remove-Item -LiteralPath $tools -Recurse -Force
}
