# Embedded in gogoke.exe. Never load an installed or updateable script.
# Only the inherited lifecycle-lock handle may authorize this handoff.
$ErrorActionPreference = 'Stop'
$data = $null
$receiptAllowed = $false
$receiptStream = $null
$pinned = [System.Collections.Generic.List[Microsoft.Win32.SafeHandles.SafeFileHandle]]::new()

function Fail([string]$code) { throw $code }
function Same([string]$a, [string]$b) {
    return [string]::Equals($a, $b, [StringComparison]::OrdinalIgnoreCase)
}
function Full-Path([string]$path) {
    if ($path -cnotmatch '^[A-Za-z]:\\' -or $path -match '[\x00-\x1f]' -or
        $path.Substring(2) -match ':') { Fail 'GOGOKE_UNINSTALL_PATH_INVALID' }
    $full = [IO.Path]::GetFullPath($path)
    if (-not (Same $full $path) -or $full -match '[\\/]\.\.?([\\/]|$)') {
        Fail 'GOGOKE_UNINSTALL_PATH_INVALID'
    }
    return $full
}

# Reflection.Emit supplies fixed system P/Invoke signatures in memory. There
# is no Add-Type compilation, native helper, landed DLL, or downloaded code.
$assemblyName = [Reflection.AssemblyName]::new('GogokeUninstallNative')
$assembly = [AppDomain]::CurrentDomain.DefineDynamicAssembly(
    $assemblyName, [Reflection.Emit.AssemblyBuilderAccess]::Run)
$module = $assembly.DefineDynamicModule('GogokeUninstallNative')
$builder = $module.DefineType('GogokeUninstallNative',
    [Reflection.TypeAttributes]::Public -bor [Reflection.TypeAttributes]::Sealed)
$methodFlags = [Reflection.MethodAttributes]::Public -bor
    [Reflection.MethodAttributes]::Static -bor [Reflection.MethodAttributes]::PinvokeImpl
function Native-Method([string]$name, [Type]$result, [Type[]]$arguments) {
    $method = $builder.DefineMethod($name, $methodFlags, $result, $arguments)
    $attribute = [Runtime.InteropServices.DllImportAttribute]
    $constructor = $attribute.GetConstructor([Type[]]@([string]))
    $fields = [Reflection.FieldInfo[]]@(
        $attribute.GetField('SetLastError'),
        $attribute.GetField('CharSet'),
        $attribute.GetField('ExactSpelling'),
        $attribute.GetField('CallingConvention'))
    $values = [object[]]@(
        $true,
        [Runtime.InteropServices.CharSet]::Unicode,
        $true,
        [Runtime.InteropServices.CallingConvention]::Winapi)
    $method.SetCustomAttribute([Reflection.Emit.CustomAttributeBuilder]::new(
        $constructor, [object[]]@('kernel32.dll'), $fields, $values))
    $method.SetImplementationFlags(
        [Reflection.MethodImplAttributes]($method.GetMethodImplementationFlags() -bor
            [Reflection.MethodImplAttributes]::PreserveSig))
}
Native-Method 'CreateFileW' ([IntPtr]) @([string],[uint32],[uint32],[IntPtr],[uint32],[uint32],[IntPtr])
Native-Method 'GetFileInformationByHandleEx' ([bool]) @([IntPtr],[int],[IntPtr],[uint32])
Native-Method 'GetFinalPathNameByHandleW' ([uint32]) @([IntPtr],[Text.StringBuilder],[uint32],[uint32])
Native-Method 'SetFileInformationByHandle' ([bool]) @([IntPtr],[int],[IntPtr],[uint32])
Native-Method 'GetStdHandle' ([IntPtr]) @([int])
Native-Method 'GetFileAttributesW' ([uint32]) @([string])
$null = $builder.CreateType()

function Open-Object([string]$path, [bool]$directory, [bool]$delete) {
    # OPEN_REPARSE_POINT prevents following the final component. Every
    # ancestor is opened and retained separately before a descendant opens.
    $access = [uint32]0x80 # FILE_READ_ATTRIBUTES
    if (-not $directory) { $access = $access -bor [uint32]2147483648 } # GENERIC_READ
    if ($delete) { $access = $access -bor [uint32]0x10000 } # DELETE
    $flags = [uint32]0x200000 # FILE_FLAG_OPEN_REPARSE_POINT
    if ($directory) { $flags = $flags -bor [uint32]0x2000000 } # BACKUP_SEMANTICS
    # Share READ only. A pinned ancestor cannot be renamed/deleted or
    # modified into a reparse point through a WRITE_DATA/WRITE_ATTRIBUTES
    # handle. The share mode applies to this directory object, not its
    # children; owned child files can still be opened with DELETE access.
    $share = [uint32]1
    $raw = [GogokeUninstallNative]::CreateFileW($path, $access, $share, [IntPtr]::Zero,
        [uint32]3, $flags, [IntPtr]::Zero)
    if ($raw.ToInt64() -eq -1 -or $raw -eq [IntPtr]::Zero) {
        Fail 'GOGOKE_UNINSTALL_OPEN_FAILED'
    }
    return [Microsoft.Win32.SafeHandles.SafeFileHandle]::new($raw, $true)
}
function Info-Bytes([IntPtr]$handle, [int]$kind, [int]$length) {
    $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal($length)
    try {
        if (-not [GogokeUninstallNative]::GetFileInformationByHandleEx($handle, $kind, $buffer, [uint32]$length)) {
            Fail 'GOGOKE_UNINSTALL_HANDLE_INFO_FAILED'
        }
        $bytes = [byte[]]::new($length)
        [Runtime.InteropServices.Marshal]::Copy($buffer, $bytes, 0, $length)
        return ,$bytes
    } finally {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer)
    }
}
function Object-Identity([IntPtr]$handle) {
    $bytes = Info-Bytes $handle 18 24 # FileIdInfo / FILE_ID_INFO
    $id = ([BitConverter]::ToString($bytes, 8, 16)).Replace('-', '').ToLowerInvariant()
    if ($id -ceq ('0' * 32)) { Fail 'GOGOKE_UNINSTALL_FILE_ID_UNAVAILABLE' }
    return @{ volumeSerialNumber = [BitConverter]::ToUInt64($bytes, 0).ToString();
        fileId = $id }
}
function Object-Attributes([IntPtr]$handle) {
    $bytes = Info-Bytes $handle 9 8 # FileAttributeTagInfo
    return [BitConverter]::ToUInt32($bytes, 0)
}
function Object-Path([IntPtr]$handle) {
    $buffer = [Text.StringBuilder]::new(32768)
    $count = [GogokeUninstallNative]::GetFinalPathNameByHandleW($handle, $buffer, [uint32]$buffer.Capacity, [uint32]0)
    if ($count -eq 0 -or $count -ge $buffer.Capacity) {
        Fail 'GOGOKE_UNINSTALL_HANDLE_PATH_FAILED'
    }
    $value = $buffer.ToString()
    if (-not $value.StartsWith('\\?\', [StringComparison]::Ordinal)) {
        Fail 'GOGOKE_UNINSTALL_HANDLE_PATH_INVALID'
    }
    return $value.Substring(4)
}
function Assert-Opened([IntPtr]$handle, [string]$path, [bool]$directory,
    [object]$expectedIdentity) {
    $attributes = Object-Attributes $handle
    if (($attributes -band [uint32]0x400) -ne 0 -or
        ((($attributes -band [uint32]0x10) -ne 0) -ne $directory) -or
        -not (Same (Object-Path $handle) $path)) {
        Fail 'GOGOKE_UNINSTALL_OPENED_OBJECT_UNSAFE'
    }
    $identity = Object-Identity $handle
    if ($null -ne $expectedIdentity -and
        ($identity.volumeSerialNumber -cne [string]$expectedIdentity.volumeSerialNumber -or
         $identity.fileId -cne [string]$expectedIdentity.fileId)) {
        Fail 'GOGOKE_UNINSTALL_OBJECT_CHANGED'
    }
    return $identity
}
function Pin-Directory([string]$path, [object]$expectedIdentity) {
    $handle = Open-Object $path $true $false
    try {
        $identity = Assert-Opened $handle.DangerousGetHandle() $path $true $expectedIdentity
        $script:pinned.Add($handle)
        return $identity
    } catch {
        $handle.Dispose()
        throw
    }
}
function Pin-Ancestors([string]$path, [object]$rootIdentity) {
    $full = Full-Path $path
    $current = $full.Substring(0, 3)
    $null = Pin-Directory $current $null
    foreach ($part in $full.Substring(3).Split([char]'\')) {
        if (-not $part) { continue }
        $current = [IO.Path]::Combine($current, $part)
        $expected = if (Same $current $data.root) { $rootIdentity } else { $null }
        $null = Pin-Directory $current $expected
    }
}
function Assert-Instance {
    $key = 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\' + $data.registryKey
    $record = Get-ItemProperty -LiteralPath $key -ErrorAction Stop
    if (-not (Same (Full-Path ([string]$record.InstallLocation)) $data.root) -or
        -not (Same ([string]$record.InstallInstanceId) ([string]$data.instance)) -or
        [string]$record.InstallDomain -cne [string]$data.domain -or
        -not (Same ([string]$record.UninstallString) ('"' + $data.root + '\gogoke.exe" --uninstall'))) {
        Fail 'GOGOKE_UNINSTALL_INSTANCE_CHANGED'
    }
}
function Assert-Root {
    $rootHandle = $script:pinned[$script:rootPin]
    $null = Assert-Opened $rootHandle.DangerousGetHandle() $data.root $true $data.rootIdentity
    Assert-Instance
}
function Delete-Opened([Microsoft.Win32.SafeHandles.SafeFileHandle]$handle) {
    $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal(1)
    try {
        [Runtime.InteropServices.Marshal]::WriteByte($buffer, 0, 1)
        # FileDispositionInfo=4; Windows performs the ordinary deletion on
        # close. No POSIX or ignore-readonly disposition flags are used.
        if (-not [GogokeUninstallNative]::SetFileInformationByHandle(
            $handle.DangerousGetHandle(), 4, $buffer, [uint32]1)) {
            Fail 'GOGOKE_UNINSTALL_DELETE_FAILED'
        }
    } finally {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer)
    }
}
function Path-State([string]$path) {
    $attributes = [GogokeUninstallNative]::GetFileAttributesW($path)
    if ($attributes -ne [uint32]::MaxValue) { return 'PRESENT' }
    $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($errorCode -eq 2) { return 'MISSING' } # ERROR_FILE_NOT_FOUND
    if ($errorCode -eq 3) { return 'PARENT_MISSING' }
    Fail 'GOGOKE_UNINSTALL_DELETE_CHECK_FAILED'
}
function Verify-File([object]$entry, [bool]$delete) {
    Assert-Root
    $handle = Open-Object $entry.path $false $delete
    $stream = $null
    try {
        $raw = $handle.DangerousGetHandle()
        $null = Assert-Opened $raw $entry.path $false $entry.identity
        $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read)
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            $hash = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        } finally { $sha.Dispose() }
        if ($hash -cne [string]$entry.sha256) { Fail 'GOGOKE_UNINSTALL_FILE_CHANGED' }
        $null = Assert-Opened $raw $entry.path $false $entry.identity
        Assert-Root
        if ($delete) { Delete-Opened $handle }
    } finally {
        if ($stream) { $stream.Dispose() }
        $handle.Dispose()
    }
    if ($delete -and (Path-State $entry.path) -cne 'MISSING') {
        Fail 'GOGOKE_UNINSTALL_DELETE_UNCONFIRMED'
    }
}
function Write-Receipt([string]$state, [string]$detail) {
    if (-not $script:receiptAllowed) { return }
    $record = @{ schema = 'gogoke.uninstall-result.v1'; state = $state;
        detail = $detail.Substring(0, [Math]::Min($detail.Length, 256));
        domain = $data.domain; atUtc = [DateTime]::UtcNow.ToString('o') } | ConvertTo-Json -Compress
    $bytes = [Text.Encoding]::UTF8.GetBytes($record)
    if ($bytes.Length -gt 2048) { Fail 'GOGOKE_UNINSTALL_RECEIPT_TOO_LARGE' }
    if (-not $script:receiptStream) { Fail 'GOGOKE_UNINSTALL_RECEIPT_NOT_RESERVED' }
    $script:receiptStream.SetLength(0)
    $script:receiptStream.Position = 0
    $script:receiptStream.Write($bytes, 0, $bytes.Length)
    $script:receiptStream.Flush($true)
}
function Reserve-Receipt {
    # Parent directory is already pinned. CreateNew refuses a preexisting
    # file or reparse point at the unpredictable nonce-bound name.
    $script:receiptStream = [IO.File]::Open($data.receipt, [IO.FileMode]::CreateNew,
        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $script:receiptAllowed = $true
    Write-Receipt 'PENDING' 'finalizer owns lifecycle lock; no deletion completed'
}

try {
    # The frozen index contains nearly 13,000 installed files. Absolute paths,
    # hashes, and file IDs need several MiB in the handoff.
    $line = [Console]::In.ReadLine()
    if (-not $line -or $line.Length -gt 16777216) { Fail 'GOGOKE_UNINSTALL_HANDOFF_INVALID' }
    $payload = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($line))
    $data = $payload | ConvertFrom-Json
    if ($data.registryKey -cnotin @('gogoke','gogoke-candidate') -or
        $data.files.Count -lt 1 -or $data.files.Count -gt 100000 -or
        [string]$data.nonce -cnotmatch '^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$' -or
        [int]$data.parentPid -le 0 -or -not $data.instance) {
        Fail 'GOGOKE_UNINSTALL_HANDOFF_INVALID'
    }
    $data.root = Full-Path ([string]$data.root)
    $data.lockPath = Full-Path ([string]$data.lockPath)
    $data.receipt = Full-Path ([string]$data.receipt)
    $expectedDomain = if ($data.registryKey -ceq 'gogoke') { 'OWNER_RELEASE' } else { 'CI_CANDIDATE_RESOURCE' }
    if ($data.domain -cne $expectedDomain -or
        -not (Same $data.lockPath ([IO.Path]::Combine([IO.Path]::GetDirectoryName($data.root),
            'gogoke-install-lifecycle.lock')))) {
        Fail 'GOGOKE_UNINSTALL_DOMAIN_OR_LOCK_CHANGED'
    }
    $instanceBytes = [Text.Encoding]::UTF8.GetBytes([string]$data.instance)
    $sha = [Security.Cryptography.SHA256]::Create()
    try { $tag = ([BitConverter]::ToString($sha.ComputeHash($instanceBytes))).Replace('-', '').ToLowerInvariant().Substring(0, 16) }
    finally { $sha.Dispose() }
    $expectedReceipt = [IO.Path]::Combine([IO.Path]::GetDirectoryName($data.root),
        ('gogoke-uninstall-' + $tag + '-' + $data.nonce + '.json'))
    if (-not (Same $data.receipt $expectedReceipt) -or
        [string]$data.rootIdentity.volumeSerialNumber -cnotmatch '^[0-9]+$' -or
        [string]$data.rootIdentity.fileId -cnotmatch '^[0-9a-f]{32}$') {
        Fail 'GOGOKE_UNINSTALL_HANDOFF_INVALID'
    }
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $rootPrefix = $data.root.TrimEnd('\') + '\'
    foreach ($entry in $data.files) {
        $entry.path = Full-Path ([string]$entry.path)
        if (-not $entry.path.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            -not $seen.Add($entry.path) -or
            [string]$entry.sha256 -cnotmatch '^[0-9a-f]{64}$' -or
            [string]$entry.identity.volumeSerialNumber -cnotmatch '^[0-9]+$' -or
            [string]$entry.identity.fileId -cnotmatch '^[0-9a-f]{32}$') {
            Fail 'GOGOKE_UNINSTALL_INVENTORY_INVALID'
        }
    }
    # Pin from the volume root to the installed root, including every
    # ancestor. Holding no-delete-sharing handles prevents a junction swap
    # between a check and a later path open.
    Pin-Ancestors $data.root $data.rootIdentity
    $script:rootPin = $pinned.Count - 1
    $lockHandle = [GogokeUninstallNative]::GetStdHandle(-12) # inherited stderr handle
    if ($lockHandle -eq [IntPtr]::Zero -or $lockHandle.ToInt64() -eq -1) {
        Fail 'GOGOKE_UNINSTALL_LOCK_HANDLE_MISSING'
    }
    $null = Assert-Opened $lockHandle $data.lockPath $false $null
    Assert-Root
    $parent = [Diagnostics.Process]::GetProcessById([int]$data.parentPid)
    $null = $parent.Handle
    Reserve-Receipt
    $marker = [Text.Encoding]::ASCII.GetBytes('LOCK:' + $data.nonce + "`n")
    $lockStream = [Console]::OpenStandardError()
    $lockStream.Write($marker, 0, $marker.Length)
    $lockStream.Flush()
    [Console]::Out.WriteLine('READY:' + $data.nonce)
    [Console]::Out.Flush()
    if (-not $parent.WaitForExit(120000)) { Fail 'GOGOKE_UNINSTALL_PARENT_STILL_RUNNING' }
    Assert-Root
    foreach ($entry in $data.files) {
        # Pin and reject every intermediate subdirectory without following
        # any reparse point. These handles remain open through deletion.
        $parentPath = [IO.Path]::GetDirectoryName($entry.path)
        $relative = if (Same $parentPath $data.root) { '' }
            else { $parentPath.Substring($rootPrefix.Length) }
        $current = $data.root
        foreach ($part in $relative.Split([char]'\')) {
            if (-not $part) { continue }
            $current = [IO.Path]::Combine($current, $part)
            if ($seen.Add('dir:' + $current)) { $null = Pin-Directory $current $null }
        }
        Verify-File $entry $false
    }
    foreach ($entry in $data.files) { Verify-File $entry $true }
    # Shortcut deletion requires installer-recorded identity. A matching
    # filename and TargetPath do not prove that a later user-created .lnk
    # belongs to this installation.
    Assert-Root
    $subkey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\' + $data.registryKey
    [Microsoft.Win32.Registry]::CurrentUser.DeleteSubKey($subkey, $false)
    Write-Receipt 'DELETED' 'owned files removed; unknown files and user data retained'
    exit 0
} catch {
    # Temporary cloud diagnostic for the first real process failure. Remove
    # after the failing operation has been identified and repaired.
    if ($env:GITHUB_ACTIONS -ceq 'true') {
        $detail = ('{0} line={1} {2}' -f $_.Exception.GetType().Name,
            $_.InvocationInfo.ScriptLineNumber, $_.Exception.Message) -replace '[\r\n]', ' '
        [Console]::Out.WriteLine('CI_FINALIZER_ERROR ' + $detail.Substring(0, [Math]::Min($detail.Length, 512)))
        [Console]::Out.Flush()
    }
    try { Write-Receipt 'FAILED' ([string]$_.Exception.Message) } catch { }
    exit 1
} finally {
    if ($receiptStream) { $receiptStream.Dispose() }
    foreach ($handle in $pinned) { $handle.Dispose() }
}
