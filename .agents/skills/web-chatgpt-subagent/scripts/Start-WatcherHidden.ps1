param(
    [Parameter(Mandatory = $true)][string] $Task,
    [Parameter(Mandatory = $true)][string] $Tier,
    [Parameter(Mandatory = $true)][string] $Repo,
    [Parameter(Mandatory = $true)][string] $Branch,
    [Parameter(Mandatory = $true)][string] $Path,
    [Parameter(Mandatory = $true)][string] $BaseSha,
    [Parameter(Mandatory = $true)][string] $Thread,
    [double] $MaxHours = 6
)

$ErrorActionPreference = 'Stop'
$pythonw = (Get-Command pythonw.exe -ErrorAction Stop).Source
$entry = Join-Path $PSScriptRoot 'watch_github_result.py'
$start = [System.Diagnostics.ProcessStartInfo]::new($pythonw)
$start.UseShellExecute = $false
$start.CreateNoWindow = $true
$start.WindowStyle = [System.Diagnostics.ProcessWindowStyle]::Hidden
foreach ($argument in @($entry, '--task', $Task, '--tier', $Tier, '--repo', $Repo, '--branch', $Branch, '--path', $Path, '--base-sha', $BaseSha, '--thread', $Thread, '--max-hours', [string]$MaxHours)) {
    [void] $start.ArgumentList.Add($argument)
}
$process = [System.Diagnostics.Process]::Start($start)
if ($null -eq $process) { throw 'Could not start windowless GitHub watcher.' }
[pscustomobject]@{ task = $Task; watcher_pid = $process.Id; window_style = 'hidden'; console = 'pythonw' } | ConvertTo-Json -Compress
