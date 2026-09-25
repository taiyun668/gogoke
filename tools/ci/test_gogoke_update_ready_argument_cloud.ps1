#requires -Version 5.1
param([Parameter(Mandatory = $true)][string]$Source)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    $Source, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw 'Coordinator parse failure' }
$assignment = $ast.Find({
    param($node)
    $node -is [Management.Automation.Language.AssignmentStatementAst] -and
    $node.Left.Extent.Text -eq '$readyArgument'
}, $true)
if (-not $assignment) { throw 'Production readiness argument expression missing' }
$launch = $ast.Find({
    param($node)
    $node -is [Management.Automation.Language.CommandAst] -and
    $node.GetCommandName() -eq 'Start-Process' -and
    $node.Extent.Text -match '-FilePath\s+\$newExe\b'
}, $true)
if (-not $launch) { throw 'Production new-shell Start-Process command missing' }
$argumentExpression = $null
for ($i = 0; $i -lt $launch.CommandElements.Count - 1; $i++) {
    if ($launch.CommandElements[$i].Extent.Text -eq '-ArgumentList') {
        $argumentExpression = $launch.CommandElements[$i + 1].Extent.Text
        break
    }
}
if (-not $argumentExpression) { throw 'Production new-shell argument expression missing' }

$nodePath = (Get-Command node.exe).Source
$javascript = 'process.exit(process.argv.length===2&&process.argv[1]===process.env.GOGOKE_QUOTE_EXPECTED?0:19)'
$prefix = '-e "' + $javascript + '" -- '
$volumeRoot = [IO.Path]::GetPathRoot([IO.Path]::GetFullPath($env:TEMP))
try {
    foreach ($ReadyFile in @(
        (Join-Path $volumeRoot 'Temporary Space\gogoke-update-0123456789abcdef.ready'),
        (Join-Path $volumeRoot 'Temporary & (Probe)\gogoke-update-0123456789abcdef.ready')
    )) {
        $env:GOGOKE_QUOTE_EXPECTED = '--gogoke-update-ready=' + $ReadyFile
        . ([ScriptBlock]::Create($assignment.Extent.Text))
        $actualArgument = . ([ScriptBlock]::Create($argumentExpression))
        $argumentText = @($actualArgument) -join ' '
        $child = Start-Process -FilePath $nodePath -ArgumentList ($prefix + $argumentText) `
            -WindowStyle Hidden -Wait -PassThru
        if ($child.ExitCode -ne 0) { throw 'Quoted readiness argument was split or changed' }
        $oldArgument = '--gogoke-update-ready=' + $ReadyFile
        $child = Start-Process -FilePath $nodePath -ArgumentList ($prefix + $oldArgument) `
            -WindowStyle Hidden -Wait -PassThru
        if ($child.ExitCode -ne 19) { throw 'Unquoted negative control did not detect splitting' }
        Write-Output 'PASS readiness argument quoting and unquoted negative control'
    }
} finally {
    $env:GOGOKE_QUOTE_EXPECTED = $null
}
