param(
    [Parameter(Mandatory)][string]$Installed,
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$RegistryKey,
    [int]$ExpectedPid = 0,
    [int]$Port = 0,
    [switch]$BeforeLaunch
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Run by the existing ordinary Interactive/Limited test task, never elevated.
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'E2E requires the ordinary non-elevated installed view'
}
if ($RegistryKey -cne 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke-candidate') {
    throw 'Only the candidate registration may be used'
}
$root = [IO.Path]::GetFullPath($Installed).TrimEnd('\')
$registration = Get-ItemProperty -LiteralPath $RegistryKey
if ([IO.Path]::GetFullPath($registration.InstallLocation).TrimEnd('\') -cne $root -or
    $registration.InstallDomain -cne 'CI_CANDIDATE_RESOURCE' -or
    $registration.DisplayVersion -cne $Version) { throw 'Actual candidate registration differs' }
if ((Get-Item -LiteralPath $root).Attributes -band [IO.FileAttributes]::ReparsePoint) {
    throw 'Candidate root is a reparse point'
}
if ($BeforeLaunch) { 'PASS_ORDINARY_REGISTERED_CANDIDATE'; exit 0 }
$processes = @(Get-CimInstance Win32_Process)
$product = @($processes | Where-Object ProcessId -eq $ExpectedPid)
if ($product.Count -ne 1 -or $product[0].ExecutablePath -cne (Join-Path $root 'gogoke.exe')) {
    throw 'Actual installed product process differs'
}
$listeners = @(Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction Stop)
if ($listeners.Count -ne 1 -or $listeners[0].LocalAddress -cne '127.0.0.1') {
    throw 'Product diagnostic listener is not unique loopback'
}
$cursor = [int]$listeners[0].OwningProcess
$seen = [Collections.Generic.HashSet[int]]::new()
while ($cursor -ne $ExpectedPid) {
    if (-not $seen.Add($cursor)) { throw 'Process parent cycle' }
    $child = @($processes | Where-Object ProcessId -eq $cursor)
    if ($child.Count -ne 1) { throw 'Diagnostic process ancestry is unavailable' }
    $cursor = [int]$child[0].ParentProcessId
    if ($cursor -le 0) { throw 'Diagnostic endpoint belongs to another product' }
}
'PASS_ORDINARY_REGISTERED_CANDIDATE_ENDPOINT'
