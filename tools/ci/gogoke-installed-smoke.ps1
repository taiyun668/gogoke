# Cloud-only launcher for the existing installed-product smoke.
# WebView2 150+ ignores diagnostic environment overrides in elevated hosts:
# https://github.com/tauri-apps/wry/issues/1782
# Lower the test process token; never change WebView2/UAC/SAC policy or the product.
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$InstallDirectory,
    [switch]$MediumChild,
    [switch]$NegativeOnly,
    [ValidateSet('node', 'native-host', 'service')][string]$NegativeComponent = 'node'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (-not $IsWindows -or $env:GITHUB_ACTIONS -ne 'true') {
    throw 'Installed UI smoke launcher is Windows cloud CI only'
}
$installed = (Resolve-Path -LiteralPath $InstallDirectory).Path
$smoke = Join-Path $PSScriptRoot 'gogoke-package-service.mjs'
$receipt = $null
$negativeReceipts = @()

if ($MediumChild) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    try {
        $principal = [Security.Principal.WindowsPrincipal]::new($identity)
        if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
            throw 'Installed UI smoke is still elevated; no product request was sent'
        }
    } finally { $identity.Dispose() }
    Write-Output 'R2-04 installed UI smoke: non-administrator cloud process'
    $mode = if ($NegativeOnly) { 'smoke-negative' } else { 'smoke' }
    if ($NegativeOnly) { & node $smoke $mode $installed $NegativeComponent }
    else { & node $smoke $mode $installed }
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
    $positiveExit = $LASTEXITCODE
    Remove-Item Env:GOGOKE_R2_SMOKE_RECEIPT -ErrorAction SilentlyContinue
    if ($null -eq $positiveExit) { throw 'Cloud smoke child exit status unavailable; do not resend' }
    if (-not (Test-Path -LiteralPath $receipt -PathType Leaf)) {
        throw "Non-administrator installed smoke produced no receipt; exit $positiveExit"
    }
    if ($positiveExit -ne 0) {
        & node $smoke record-smoke $receipt
        if ($LASTEXITCODE -ne 0) { throw "Installed smoke failure receipt recording failed: $LASTEXITCODE" }
        throw "Non-administrator installed smoke failed: $positiveExit"
    }

    # Hold one installed component at a time after the preceding Medium process exits.
    $cases = @(
        @{ Name = 'node'; Relative = 'gogoke-service/runtime/node.exe' },
        @{ Name = 'native-host'; Relative = 'gogoke-native-host.exe' },
        @{ Name = 'service'; Relative = 'gogoke-service/dist/bin.mjs' }
    )
    foreach ($case in $cases) {
        $component = [IO.Path]::GetFullPath((Join-Path $installed $case.Relative))
        if (-not $component.StartsWith($installed + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Installed negative component escaped installation directory'
        }
        $held = "$component.r205-held"
        if (-not (Test-Path -LiteralPath $component -PathType Leaf) -or (Test-Path -LiteralPath $held)) {
            throw "Installed negative component ownership invalid: $($case.Name)"
        }
        $originalHash = (Get-FileHash -LiteralPath $component -Algorithm SHA256).Hash
        Move-Item -LiteralPath $component -Destination $held
        try {
            if ($case.Name -eq 'service') {
                Set-Content -LiteralPath $component -NoNewline -Value 'throw new Error("EXPECTED_R205_SERVICE_START_FAILURE")'
            }
            $negativeReceipt = Join-Path $env:RUNNER_TEMP ('gogoke-r2-smoke-receipt-' + [Guid]::NewGuid().ToString('N') + '.json')
            if (Test-Path -LiteralPath $negativeReceipt) { throw 'Fresh negative smoke receipt path already exists' }
            $env:GOGOKE_R2_SMOKE_RECEIPT = $negativeReceipt
            & $launcher --integrity Medium --direct $shell -NoProfile -File $PSCommandPath -InstallDirectory $installed -MediumChild -NegativeOnly -NegativeComponent $case.Name
            $negativeExit = $LASTEXITCODE
        } finally {
            Remove-Item Env:GOGOKE_R2_SMOKE_RECEIPT -ErrorAction SilentlyContinue
            if ($case.Name -eq 'service' -and (Test-Path -LiteralPath $component -PathType Leaf)) {
                Remove-Item -LiteralPath $component -Force
            }
            Move-Item -LiteralPath $held -Destination $component
        }
        if ((Get-FileHash -LiteralPath $component -Algorithm SHA256).Hash -cne $originalHash) {
            throw "Installed negative component bytes changed after restore: $($case.Name)"
        }
        if ($null -eq $negativeExit) { throw 'Cloud negative smoke child exit status unavailable; do not resend' }
        if (-not (Test-Path -LiteralPath $negativeReceipt -PathType Leaf)) {
            throw "Non-administrator negative smoke produced no receipt; exit $negativeExit"
        }
        $negativeReceipts += $negativeReceipt
        if ($negativeExit -ne 0) {
            & node $smoke record-smoke $receipt @negativeReceipts
            throw "Non-administrator negative smoke failed: $negativeExit"
        }
    }
    & node $smoke record-smoke $receipt @negativeReceipts
    if ($null -eq $LASTEXITCODE) { throw 'Installed smoke receipt recording status unavailable' }
    if ($LASTEXITCODE -ne 0) { throw "Installed smoke receipt recording failed: $LASTEXITCODE" }
} finally {
    Remove-Item Env:GOGOKE_R2_SMOKE_RECEIPT -ErrorAction SilentlyContinue
    if ($receipt -and (Test-Path -LiteralPath $receipt -PathType Leaf)) { Remove-Item -LiteralPath $receipt -Force }
    foreach ($negativeReceipt in $negativeReceipts) {
        if (Test-Path -LiteralPath $negativeReceipt -PathType Leaf) { Remove-Item -LiteralPath $negativeReceipt -Force }
    }
    Remove-Item -LiteralPath $tools -Recurse -Force
}
