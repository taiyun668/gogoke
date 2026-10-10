param([Parameter(Mandatory)][string]$Installed)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($Installed).TrimEnd('\')
$targets = @((Join-Path $root 'gogoke.exe'), (Join-Path $root 'gogoke-native-host.exe'))
foreach ($process in @(Get-CimInstance Win32_Process -ErrorAction Stop |
        Where-Object { $_.Name -in @('gogoke.exe', 'gogoke-native-host.exe') })) {
    if ([string]::IsNullOrWhiteSpace($process.ExecutablePath)) {
        throw 'V11 candidate process executable path is unknown; closed DB not proven'
    }
    $observed = [IO.Path]::GetFullPath($process.ExecutablePath)
    if ($targets -contains $observed) {
        throw 'V11 exact installed candidate process is active; immutable DB read refused'
    }
}
'NO_INSTALLED_CANDIDATE_PROCESS'
