param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]] $LedgerArgs
)

$ErrorActionPreference = 'Stop'
$receiptDir = Join-Path $env:LOCALAPPDATA 'gogoke\web-chatgpt-subagent\receipts'
New-Item -ItemType Directory -Path $receiptDir -Force | Out-Null
$receipt = Join-Path $receiptDir ("ledger-$([guid]::NewGuid().ToString('N')).json")
$pythonw = (Get-Command pythonw.exe -ErrorAction Stop).Source
$entry = Join-Path $PSScriptRoot 'ledgerw.py'
$start = [System.Diagnostics.ProcessStartInfo]::new($pythonw)
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
foreach ($argument in @($entry, '--receipt', $receipt, '--') + $LedgerArgs) {
    [void] $start.ArgumentList.Add($argument)
}
$process = [System.Diagnostics.Process]::Start($start)
if ($null -eq $process) { throw 'Could not start windowless ledger process.' }
$process.WaitForExit()
if (-not (Test-Path -LiteralPath $receipt)) { throw 'Windowless ledger process wrote no receipt.' }
$result = Get-Content -LiteralPath $receipt -Raw -Encoding utf8 | ConvertFrom-Json
if ($result.console_window_present) { throw 'Ledger process unexpectedly had a console window.' }
if ($process.ExitCode -ne $result.exit_code) { throw 'Ledger exit code differs from its receipt.' }
if ($result.exit_code -ne 0) { throw "Ledger command failed: $($result.stderr)" }
$result | ConvertTo-Json -Compress -Depth 5
