param(
  [Parameter(Mandatory)][string]$OriginalInput,
  [Parameter(Mandatory)][string]$OutputDirectory
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
function Need([bool]$ok,[string]$why){if(-not $ok){throw $why}}
function ReadJson([string]$p){Get-Content -LiteralPath $p -Raw|ConvertFrom-Json -DateKind String}
function WriteJson([string]$p,[object]$v){
  [IO.File]::WriteAllText($p,($v|ConvertTo-Json -Depth 70),[Text.UTF8Encoding]::new($false))
}
$source=ReadJson $OriginalInput
Need ($source.mode -ceq 'verify_only' -and $source.schema -ceq 'gogoke.37.protection-input.v1') 'Original D offline input required'
$script=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\capture-protection.ps1')).Path
$source.scriptSha256=(Get-FileHash -LiteralPath $script).Hash.ToLowerInvariant()
$map=(Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..\installed-protection-fields.json')).Path
$source|Add-Member -Force -NotePropertyName fieldMapSha256 -NotePropertyValue (Get-FileHash -LiteralPath $map).Hash.ToLowerInvariant()
$directory=(Resolve-Path -LiteralPath $OutputDirectory).Path
$label=[Guid]::NewGuid().ToString('N')
$goodInput=Join-Path $directory ("good-$label.input.json")
$goodOutput=Join-Path $directory ("good-$label.report.json")
WriteJson $goodInput $source
& (Join-Path $PSHOME 'pwsh.exe') -NoProfile -NonInteractive -File $script -InputFile $goodInput -OutputFile $goodOutput *> (Join-Path $directory ("good-$label.log"))
Need ($LASTEXITCODE -eq 0 -and (Test-Path -LiteralPath $goodOutput)) 'Original 13-field D replay failed'
$good=ReadJson $goodOutput
Need ($good.measuredFieldCount -eq 13 -and $good.allEqual -eq $true -and $good.acceptance -eq $false) 'Original 13-field D replay changed'
$source.caseClaim=$source.caseBinding
$badInput=Join-Path $directory ("bad-$label.input.json")
$badOutput=Join-Path $directory ("bad-$label.report.json")
WriteJson $badInput $source
& (Join-Path $PSHOME 'pwsh.exe') -NoProfile -NonInteractive -File $script -InputFile $badInput -OutputFile $badOutput *> (Join-Path $directory ("bad-$label.log"))
Need ($LASTEXITCODE -ne 0 -and -not (Test-Path -LiteralPath $badOutput)) 'Original V1 false claim field was accepted'
$source=ReadJson $OriginalInput
$source.scriptSha256=(Get-FileHash -LiteralPath $script).Hash.ToLowerInvariant()
$source|Add-Member -Force -NotePropertyName fieldMapSha256 -NotePropertyValue (Get-FileHash -LiteralPath $map).Hash.ToLowerInvariant()
$observer=ReadJson $source.observerBinding.path
$observer.observers[0].equalFields[1]=$observer.observers[0].equalFields[0]
$badObserver=Join-Path $directory ("duplicate-field-$label.json")
WriteJson $badObserver $observer
$source.observerBinding=[pscustomobject]@{path=$badObserver;sha256=(Get-FileHash -LiteralPath $badObserver).Hash.ToLowerInvariant()}
$fieldInput=Join-Path $directory ("duplicate-field-$label.input.json")
$fieldOutput=Join-Path $directory ("duplicate-field-$label.report.json")
WriteJson $fieldInput $source
& (Join-Path $PSHOME 'pwsh.exe') -NoProfile -NonInteractive -File $script -InputFile $fieldInput -OutputFile $fieldOutput *> (Join-Path $directory ("duplicate-field-$label.log"))
Need ($LASTEXITCODE -ne 0 -and -not (Test-Path -LiteralPath $fieldOutput)) 'Duplicate original field was accepted'
'ORIGINAL_D_THIRTEEN_FALSE_CLAIM_AND_DUPLICATE_FIELD_NEGATIVES_OK'
