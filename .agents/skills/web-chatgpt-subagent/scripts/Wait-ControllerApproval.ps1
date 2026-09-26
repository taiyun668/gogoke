param(
    [Parameter(Mandatory = $true)][string] $Task,
    [int] $TimeoutMinutes = 60,
    [int] $PollSeconds = 15
)

$ErrorActionPreference = 'Stop'
if ($PollSeconds -lt 1) { throw 'PollSeconds must be positive.' }
$statePath = Join-Path $env:LOCALAPPDATA 'gogoke\web-chatgpt-subagent\state.json'
$deadline = [DateTimeOffset]::UtcNow.AddMinutes($TimeoutMinutes)
while ([DateTimeOffset]::UtcNow -lt $deadline) {
    if (Test-Path -LiteralPath $statePath) {
        try {
            $state = Get-Content -LiteralPath $statePath -Raw -Encoding utf8 | ConvertFrom-Json
            $active = $state.active_web_task
            if ($null -ne $active -and $active.task -eq $Task -and $active.approved_result_commit) {
                [pscustomobject]@{ task = $Task; approved_result_commit = $active.approved_result_commit } | ConvertTo-Json -Compress
                exit 0
            }
            if ($null -eq $active -or $active.task -ne $Task) { throw 'Active web task changed before result approval.' }
        } catch [System.IO.IOException] {
            # Atomic state replacement may race one read; try again.
        }
    }
    Start-Sleep -Seconds $PollSeconds
}
throw 'Controller result approval did not arrive before the deadline.'
