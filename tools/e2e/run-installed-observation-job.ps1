param([string]$InputFile = (Join-Path $PSScriptRoot 'observation.job.json'))
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$job = Get-Content -LiteralPath $InputFile -Raw | ConvertFrom-Json -DateKind String
if ($job.schema -cne 'gogoke.37.installed-observation-job.v1' -or $job.acceptance -ne $false) { throw 'Invalid observation job' }
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
try {
    if ($identity.IsSystem -or ([Security.Principal.WindowsPrincipal]::new($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) -or -not [Environment]::UserInteractive) { throw 'Ordinary interactive Limited instrument run required' }
} finally { $identity.Dispose() }
foreach ($ref in @($job.node, $job.runner)) {
    if ((Get-FileHash -LiteralPath $ref.path -Algorithm SHA256).Hash.ToLowerInvariant() -cne $ref.sha256) { throw 'Observation runner/runtime changed' }
}
if ((Get-AuthenticodeSignature -LiteralPath $job.node.path).Status -ne 'Valid') { throw 'Signed Node runtime required' }
& $job.node.path $job.runner.path $InputFile
exit $LASTEXITCODE
