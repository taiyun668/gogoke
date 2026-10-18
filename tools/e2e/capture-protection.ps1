param(
  [Parameter(Mandatory)][string]$InputFile,
  [Parameter(Mandatory)][string]$OutputFile
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
function Need([bool]$ok,[string]$message){if(-not $ok){throw $message}}
function Sha([string]$file){(Get-FileHash -Algorithm SHA256 -LiteralPath $file).Hash.ToLowerInvariant()}
function ReadJson([string]$file){Get-Content -LiteralPath $file -Raw|ConvertFrom-Json -DateKind String}
function Pin([object]$ref){Need ($null -ne $ref -and $ref.path -and $ref.sha256 -and (Sha $ref.path) -ceq $ref.sha256) 'Protection source pin differs'}
function Same([object]$a,[object]$b){($a|ConvertTo-Json -Depth 70 -Compress) -ceq ($b|ConvertTo-Json -Depth 70 -Compress)}
function Fresh([string]$file,[object]$value){
  $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($value|ConvertTo-Json -Depth 70)+"`n")
  $handle=[IO.File]::Open($file,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
  try{$handle.Write($bytes,0,$bytes.Length);$handle.Flush($true)}finally{$handle.Dispose()}
}
$i=ReadJson $InputFile
$mapPath=Join-Path $PSScriptRoot 'installed-protection-fields.json'
$mapHash=Sha $mapPath
Need ($i.schema -ceq 'gogoke.37.protection-input.v1' -and $i.acceptance -eq $false -and
      $i.mode -in @('capture','verify_only') -and $i.scriptSha256 -ceq (Sha $PSCommandPath) -and
      $i.fieldMapSha256 -ceq $mapHash -and
      -not (Test-Path -LiteralPath $OutputFile)) 'Protection input/mode/script/output differs'
$fieldMap=ReadJson $mapPath
Need ((@($fieldMap.PSObject.Properties.Name|Sort-Object) -join ',') -ceq 'formal,ledger,memory' -and
      @($fieldMap.formal).Count -eq 5 -and @($fieldMap.memory).Count -eq 5 -and
      @($fieldMap.ledger).Count -eq 3) 'Pinned protection field map shape differs'
foreach($name in @('caseConfig','caseBefore','caseClaim','caseBinding','observerBinding')){
  Pin $i.PSObject.Properties[$name].Value
}
if($i.mode -ceq 'capture'){Pin $i.python;Pin $i.pwsh}
$config=ReadJson $i.caseConfig.path;$before=ReadJson $i.caseBefore.path
$claim=ReadJson $i.caseClaim.path;$binding=ReadJson $i.caseBinding.path;$observer=ReadJson $i.observerBinding.path
foreach($item in @(
  [pscustomobject]@{value=$config;names=@('schema','state','acceptance','sourceCommit','version','stateRoot','testbedSource','installed','installedSha256')},
  [pscustomobject]@{value=$before;names=@('state','acceptance','claimSha256','bindingSha256','snapshots')},
  [pscustomobject]@{value=$claim;names=@('state','acceptance','bindingSha256','configSha256','wrapperSha256')},
  [pscustomobject]@{value=$binding;names=@('state','acceptance','configSha256','wrapperSha256')},
  [pscustomobject]@{value=$observer;names=@('state','acceptance','sourceCommit','installedVersion','stateRoot','testbedSource','installedRoot','installedSha256','observers','observerTools')}
)){
  foreach($name in $item.names){Need ($null -ne $item.value.PSObject.Properties[$name]) ('Missing original protection source field: '+$name)}
}
foreach($coreName in @('gogoke.exe','gogoke-native-host.exe','resource-index.json')){
  Need ($null -ne $config.installedSha256.PSObject.Properties[$coreName] -and
        $null -ne $observer.installedSha256.PSObject.Properties[$coreName] -and
        $config.installedSha256.$coreName -ceq $observer.installedSha256.$coreName) 'Installed candidate core source differs'
}
Need ($before.state -ceq 'MEASURED_BEFORE_NOT_ACCEPTANCE' -and $before.acceptance -eq $false -and
      $claim.state -ceq 'CLAIMED_ONCE' -and $claim.acceptance -eq $false -and
      $binding.acceptance -eq $false -and $binding.state -match '_NOT_RUN$' -and
      $config.acceptance -eq $false -and $config.state -match '_NOT_RUN$' -and
      $observer.acceptance -eq $false -and $observer.state -match '_NOT_RUN$' -and
      $config.schema -match '^gogoke\.37\..+\.v[0-9]+$' -and
      $before.claimSha256 -ceq $i.caseClaim.sha256 -and
      $before.bindingSha256 -ceq $i.caseBinding.sha256 -and
      $claim.bindingSha256 -ceq $i.caseBinding.sha256 -and
      $claim.configSha256 -ceq $i.caseConfig.sha256 -and
      $binding.configSha256 -ceq $i.caseConfig.sha256 -and
      $claim.wrapperSha256 -ceq $binding.wrapperSha256 -and
      $config.sourceCommit -ceq $observer.sourceCommit -and
      $config.version -ceq $observer.installedVersion -and
      $config.stateRoot -ceq $observer.stateRoot -and
      $config.testbedSource -ceq $observer.testbedSource -and
      $config.installed -ceq $observer.installedRoot) 'Original before/claim/binding/config/candidate chain differs'
Need (@($observer.observers).Count -eq 3 -and
      (@($observer.observers|ForEach-Object {@($_.equalFields).Count}|Measure-Object -Sum).Sum -eq 13) -and
      ((@($observer.observers|ForEach-Object {$_.name}|Sort-Object) -join ',') -ceq 'formal,ledger,memory')) 'Original 3/13 shape differs'
$after=[ordered]@{};$fields=@()
foreach($row in $observer.observers){
  $actualFields=@($row.equalFields)
  $requiredFields=@($fieldMap.($row.name))
  Need ($actualFields.Count -eq $requiredFields.Count -and
        @($actualFields|Sort-Object -Unique).Count -eq $requiredFields.Count -and
        ((@($actualFields|Sort-Object) -join ',') -ceq (@($requiredFields|Sort-Object) -join ','))) 'Original observer field set differs'
  $tool=$observer.observerTools.($row.name)
  Need ($null -ne $tool -and $row.runtime -ceq $tool.runtime -and
        $row.runtime -in @($i.python.path,$i.pwsh.path) -and
        @($row.args|Where-Object {$_ -ceq $tool.script}).Count -eq 1 -and
        (Sha $tool.script) -ceq $tool.sha256) 'Original observer executable differs'
  foreach($companion in $tool.companions.PSObject.Properties){
    Need ((Sha (Join-Path (Split-Path $tool.script) $companion.Name)) -ceq $companion.Value) 'Observer companion differs'
  }
  $original=$before.snapshots.($row.name);Pin $original
  if($i.mode -ceq 'capture'){
    $who=[Security.Principal.WindowsIdentity]::GetCurrent()
    try{Need (-not $who.IsSystem -and -not ([Security.Principal.WindowsPrincipal]::new($who)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator) -and [Environment]::UserInteractive) 'Ordinary Limited required'}finally{$who.Dispose()}
    Need ((Get-AuthenticodeSignature -LiteralPath $i.python.path).Status -eq 'Valid' -and
          (Get-AuthenticodeSignature -LiteralPath $i.pwsh.path).Status -eq 'Valid') 'Signed runtime differs'
    $target=Join-Path (Split-Path $OutputFile) ($row.name+'-after.json')
    Need (-not (Test-Path -LiteralPath $target)) 'After snapshot already exists'
    $args=@($row.args|ForEach-Object {if($_ -ceq '{output}'){$target}else{$_}})
    & $row.runtime @args
    Need ($LASTEXITCODE -eq 0) 'Original protection observer failed'
    $new=[ordered]@{path=$target;sha256=(Sha $target)}
  }else{
    $new=$i.after.($row.name);Pin $new
  }
  $after[$row.name]=$new
  $left=ReadJson $original.path;$right=ReadJson $new.path
  foreach($field in $row.equalFields){
    $beforeField=$left.PSObject.Properties[$field];$afterField=$right.PSObject.Properties[$field]
    Need ($null -ne $beforeField -and $null -ne $afterField) ('Missing original protection field: '+$row.name+'.'+$field)
    $fields+=[ordered]@{group=$row.name;field=$field;equal=(Same $beforeField.Value $afterField.Value)}
  }
}
$allEqual=@($fields|Where-Object {-not $_.equal}).Count -eq 0
Need ($fields.Count -eq 13) 'Original 13 measurements incomplete'
$report=[ordered]@{schema='gogoke.37.protection-report.v1';state='PROTECTION_THIRTEEN_REVIEW_REQUIRED';acceptance=$false;
  mode=$i.mode;inputSha256=(Sha $InputFile);loadedScriptSha256=(Sha $PSCommandPath);
  loadedFieldMapSha256=$mapHash;
  sourceRefs=[ordered]@{caseConfig=$i.caseConfig;caseBefore=$i.caseBefore;caseClaim=$i.caseClaim;
    caseBinding=$i.caseBinding;observerBinding=$i.observerBinding};before=$before.snapshots;after=$after;
  measuredFieldCount=$fields.Count;fields=$fields;allEqual=$allEqual}
Fresh $OutputFile $report
Need ($allEqual) 'Original 13 protection fields differ; retained review report'
'PROTECTION_THIRTEEN_REVIEW_REQUIRED'
