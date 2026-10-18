param([string]$InputFile=(Join-Path $PSScriptRoot 'close.input.json'))
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$i=Get-Content -LiteralPath $InputFile -Raw|ConvertFrom-Json
function Need([bool]$x,[string]$m){if(-not $x){throw $m}}
function Sha([string]$p){(Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLowerInvariant()}
$who=[Security.Principal.WindowsIdentity]::GetCurrent()
try{Need (-not $who.IsSystem -and -not ([Security.Principal.WindowsPrincipal]::new($who)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) -and [Environment]::UserInteractive) 'Ordinary interactive Limited required'}finally{$who.Dispose()}
Need ($i.schema -ceq 'gogoke.37.retained-installed-close-input.v1' -and $i.acceptance -eq $false -and
  $i.wrapperSha256 -ceq (Sha $PSCommandPath) -and $i.actorSha256 -ceq (Sha $i.actor.path) -and
  $i.node.sha256 -ceq (Sha $i.node.path) -and (Get-AuthenticodeSignature -LiteralPath $i.node.path).Status -eq 'Valid') 'Pinned shared close entry/runtime differ'
$env:TEMP=$PSScriptRoot;$env:TMP=$PSScriptRoot
& $i.node.path $i.actor.path $InputFile
Need ($LASTEXITCODE -eq 0) 'Shared retained close failed'
