param(
  [Parameter(Mandatory=$true)][string]$Config,
  [Parameter(Mandatory=$true)][string]$SignedNode
)
$ErrorActionPreference='Stop'
$configPath=[IO.Path]::GetFullPath($Config)
$raw=Get-Content -Raw -LiteralPath $configPath | ConvertFrom-Json
$evidence=[IO.Path]::GetFullPath($raw.evidenceDirectory)
if ([IO.Path]::GetPathRoot($evidence) -cne 'D:\' -or
    $raw.result -cne (Join-Path $evidence 'result.json') -or
    (Test-Path -LiteralPath $evidence)) {
  throw 'Fresh private D: evidence directory and exact result required'
}
$nodePath=[IO.Path]::GetFullPath($SignedNode)
if ($raw.signedNodeSha256 -cnotmatch '^[a-f0-9]{64}$' -or
    (Get-FileHash -LiteralPath $nodePath -Algorithm SHA256).Hash.ToLowerInvariant() -cne
      $raw.signedNodeSha256) {
  throw 'Signed Node bytes differ from the reviewed private fixture'
}
$runtimeTemp=Join-Path (Split-Path -Parent $evidence) 'temp'
if (-not (Test-Path -LiteralPath $runtimeTemp -PathType Container)) {
  New-Item -ItemType Directory -Path $runtimeTemp | Out-Null
}
$env:TEMP=$runtimeTemp
$env:TMP=$runtimeTemp
$env:E2E_TELEMETRY_DISABLED='1'
$env:PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD='1'
if (@(Get-Process gogoke,gogoke-native-host -ErrorAction SilentlyContinue).Count -ne 0) {
  throw 'Existing gogoke product/native-host process; no concurrent M2 driver'
}
& $nodePath (Join-Path $PSScriptRoot 'm2-v06-remaining-win11.mjs') $configPath
if ($LASTEXITCODE -ne 0) {
  throw 'Original V06 remaining runner failed; preserve exact result and do not replay'
}
