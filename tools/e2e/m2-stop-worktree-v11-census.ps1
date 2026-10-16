param([Parameter(Mandatory)][string]$Installed,[Parameter(Mandatory)][string]$Python)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$root = [IO.Path]::GetFullPath($Installed).TrimEnd('\')
$targets = @((Join-Path $root 'gogoke.exe'), (Join-Path $root 'gogoke-native-host.exe'))
$aliases = @'
import ctypes,json,ntpath,sys
api=ctypes.WinDLL('kernel32',use_last_error=True).GetShortPathNameW
api.argtypes=[ctypes.c_wchar_p,ctypes.c_wchar_p,ctypes.c_uint]
api.restype=ctypes.c_uint
names=set()
for path in json.loads(sys.argv[1]):
    names.add(ntpath.basename(path))
    size=api(path,None,0)
    if not size: raise ctypes.WinError(ctypes.get_last_error())
    buffer=ctypes.create_unicode_buffer(size)
    if not api(path,buffer,size): raise ctypes.WinError(ctypes.get_last_error())
    names.add(ntpath.basename(buffer.value))
print(json.dumps(sorted(names)))
'@
$namesJson = & $Python -c $aliases (ConvertTo-Json -InputObject $targets -Compress)
if ($LASTEXITCODE -ne 0) { throw 'V11 installed executable aliases cannot be identified' }
$names = @($namesJson | ConvertFrom-Json)
$observed = @()
# The runner launches only these installed names; include their real 8.3 names.
# Compare executable objects below, so ancestor junctions/aliases are not lexical exclusions.
foreach ($process in @(Get-CimInstance Win32_Process -ErrorAction Stop |
        Where-Object { $_.Name -in $names })) {
    if ([string]::IsNullOrWhiteSpace($process.ExecutablePath)) {
        throw 'V11 candidate process executable path is unknown; closed DB not proven'
    }
    $observed += @{pid=$process.ProcessId;path=$process.ExecutablePath}
}
$inputJson = @{targets=$targets;processes=$observed} | ConvertTo-Json -Depth 4 -Compress
$identityCheck = @'
import json,os,sys
inputs=json.loads(sys.argv[1])
def identity(path):
    value=os.stat(path,follow_symlinks=True)
    if not value.st_ino: raise RuntimeError('V11 executable physical identity unavailable')
    return value.st_dev,value.st_ino
targets={identity(path) for path in inputs['targets']}
for process in inputs['processes']:
    if identity(process['path']) in targets:
        raise RuntimeError('V11 installed executable object is active; immutable read refused')
print('NO_INSTALLED_CANDIDATE_PROCESS_PHYSICAL_IDENTITIES_CHECKED')
'@
& $Python -c $identityCheck $inputJson
if ($LASTEXITCODE -ne 0) { throw 'V11 physical process census failed; closed DB not proven' }
