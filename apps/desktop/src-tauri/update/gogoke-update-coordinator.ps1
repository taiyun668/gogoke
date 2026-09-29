[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$ReleaseSetDirectory,
    [Parameter(Mandatory = $true)][int]$ParentPid,
    [Parameter(Mandatory = $true)][string]$CurrentExe,
    [Parameter(Mandatory = $true)][string]$TargetDir,
    [Parameter(Mandatory = $true)][string]$ReadyFile,
    [Parameter(Mandatory = $true)][string]$ExpectedVersion,
    [Parameter(Mandatory = $true)][string]$ExpectedGenerationId,
    [Parameter(Mandatory = $true)][string]$ExpectedSha256,
    [Parameter(Mandatory = $true)][string]$ExpectedManifestSha256,
    [Parameter(Mandatory = $true)][string]$ExpectedSignatureSha256,
    [Parameter(Mandatory = $true)][string]$LockFile,
    [Parameter(Mandatory = $true)][string]$FailureLog,
    [Parameter(Mandatory = $true)][string]$StateFile,
    [Parameter(Mandatory = $true)][string]$RetentionNoticeFile,
    [switch]$Silent,
    [switch]$NoRestartOnRollback
)

$ErrorActionPreference = "Stop"
$phase = "initialize"
$lock = $null
$lifecycleLock = $null
$backupTag = [Guid]::NewGuid().ToString('N')
$backup = $null
$newProcess = $null
$targetMoved = $false
$installStarted = $false
$installerCompletionUnknown = $false
$oldApplicationStopped = $false
$hadUninstallRegistration = $false
$oldInstallInstanceId = $null
$newInstallInstanceId = $null
$oldExeSha256 = $null
$oldInventory = $null
$newInstallCommitted = $false
$uninstallKey = "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
$uninstallRegistryPath = "Registry::HKEY_CURRENT_USER\Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke"
$registryBackup = "$FailureLog.registry.reg"
$regExe = "$env:SystemRoot\System32\reg.exe"

function Set-Phase([string]$value) {
    $script:phase = $value
    Write-Output "gogoke update phase=$value"
}

function Write-Failure([string]$message) {
    $parent = Split-Path -Parent $FailureLog
    if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
    [IO.File]::WriteAllText($FailureLog, "phase=$phase`r`n$message`r`n", [Text.Encoding]::UTF8)
}

function Assert-Absolute([string]$path, [string]$label) {
    $isDriveAbsolute = $path -match '^[A-Za-z]:[\\/]'
    $isUncAbsolute = $path -match '^[\\/]{2}[^\\/]+[\\/][^\\/]+(?:[\\/]|$)'
    if (-not ($isDriveAbsolute -or $isUncAbsolute)) { throw "$label must be absolute" }
}

function Get-Sha256Hex([string]$path) {
    $stream = [IO.File]::OpenRead($path)
    try {
        $sha256 = [Security.Cryptography.SHA256]::Create()
        try {
            $hash = $sha256.ComputeHash($stream)
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
    return ([BitConverter]::ToString($hash)).Replace('-', '')
}

function Get-InstallRegistration {
    if (-not (Test-Path -LiteralPath $uninstallRegistryPath)) { return $null }
    return Get-ItemProperty -LiteralPath $uninstallRegistryPath
}

function Assert-PhysicalDirectory([string]$path, [string]$label) {
    if (-not (Test-Path -LiteralPath $path -PathType Container)) { throw "$label is missing" }
    $item = Get-Item -LiteralPath $path -Force
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        throw "$label is a reparse point"
    }
}

function Write-UpdateState([string]$status, [string]$message) {
    $state = [IO.File]::ReadAllText($StateFile) | ConvertFrom-Json -ErrorAction Stop
    if ($state.offer.version -cne $ExpectedVersion -or
        $state.status -cnotin @('applying', 'installed_cleanup_pending',
            'installed_backup_retained', 'installed')) {
        throw "prepared update state changed before completion"
    }
    $state.status = $status
    $state.lastError = if ($message) { $message } else { $null }
    $partial = "$StateFile.$([Guid]::NewGuid().ToString('N')).part"
    $payload = $state | ConvertTo-Json -Depth 10
    $bytes = [Text.UTF8Encoding]::new($false).GetBytes($payload)
    $partialStream = [IO.FileStream]::new($partial, [IO.FileMode]::CreateNew,
        [IO.FileAccess]::Write, [IO.FileShare]::None, 4096, [IO.FileOptions]::WriteThrough)
    try {
        $partialStream.Write($bytes, 0, $bytes.Length)
        $partialStream.Flush($true)
    } finally { $partialStream.Dispose() }
    try {
        # The payload bytes are flushed before the single atomic replacement.
        [IO.File]::Replace($partial, $StateFile, [NullString]::Value)
    } finally {
        if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Force }
    }
}

function Get-OldOwnedInventory {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $CurrentExe
    $start.Arguments = '--gogoke-update-owned-inventory'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = [Text.Encoding]::UTF8
    $start.StandardErrorEncoding = [Text.Encoding]::UTF8
    $process = [Diagnostics.Process]::Start($start)
    if (-not $process) { throw 'old inventory process did not start' }
    try {
        $outputTask = $process.StandardOutput.ReadToEndAsync()
        $errorTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(120000)) { throw 'old inventory process timed out' }
        $output = $outputTask.GetAwaiter().GetResult()
        $errorText = $errorTask.GetAwaiter().GetResult()
        if ($process.ExitCode -ne 0) {
            throw "old installed shell could not attest its owned inventory: $errorText"
        }
        if ($output.Length -gt 16777216) { throw 'old owned inventory is too large' }
        $record = $output | ConvertFrom-Json -ErrorAction Stop
        if ($record.schema -cne 'gogoke.update-owned-inventory.v1' -or
            -not [string]::Equals([IO.Path]::GetFullPath([string]$record.root).TrimEnd('\'),
                $targetFull, [StringComparison]::OrdinalIgnoreCase) -or
            $record.instance -cne $oldInstallInstanceId -or
            $record.files.Count -lt 1 -or $record.files.Count -gt 100000 -or
            [string]$record.rootIdentity.volumeSerialNumber -cnotmatch '^[0-9]+$' -or
            [string]$record.rootIdentity.fileId -cnotmatch '^[0-9a-f]{32}$') {
            throw 'old owned inventory identity is invalid'
        }
        return $record
    } finally {
        if (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(5000) }
        $process.Dispose()
    }
}

function Assert-InstallRegistration([string]$expectedInstance) {
    $registration = Get-InstallRegistration
    if (-not $registration -or $registration.InstallInstanceId -ne $expectedInstance -or
        $registration.InstallDomain -ne "OWNER_RELEASE" -or
        -not [string]::Equals([IO.Path]::GetFullPath($registration.InstallLocation),
            $targetFull, [StringComparison]::OrdinalIgnoreCase)) {
        throw "gogoke installation registration changed during update"
    }
}

function Acquire-LifecycleLock {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        try {
            return [IO.File]::Open($lifecyclePath, [IO.FileMode]::OpenOrCreate,
                [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
        } catch [IO.IOException] {
            Start-Sleep -Milliseconds 100
        }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "gogoke install lifecycle lock is unavailable"
}

function Initialize-BackupNative {
    if ($script:backupNativeInitialized) { return }
    $assembly = [AppDomain]::CurrentDomain.DefineDynamicAssembly(
        [Reflection.AssemblyName]::new('GogokeBackupNative'),
        [Reflection.Emit.AssemblyBuilderAccess]::Run)
    $module = $assembly.DefineDynamicModule('GogokeBackupNative')
    $builder = $module.DefineType('GogokeBackupNative',
        [Reflection.TypeAttributes]::Public -bor [Reflection.TypeAttributes]::Sealed)
    $flags = [Reflection.MethodAttributes]::Public -bor
        [Reflection.MethodAttributes]::Static -bor [Reflection.MethodAttributes]::PinvokeImpl
    function Add-Native([string]$name, [Type]$result, [Type[]]$arguments) {
        $method = $builder.DefineMethod($name, $flags, $result, $arguments)
        $attribute = [Runtime.InteropServices.DllImportAttribute]
        $constructor = $attribute.GetConstructor([Type[]]@([string]))
        $fields = [Reflection.FieldInfo[]]@(
            $attribute.GetField('SetLastError'), $attribute.GetField('CharSet'),
            $attribute.GetField('ExactSpelling'), $attribute.GetField('CallingConvention'))
        $values = [object[]]@($true, [Runtime.InteropServices.CharSet]::Unicode,
            $true, [Runtime.InteropServices.CallingConvention]::Winapi)
        $method.SetCustomAttribute([Reflection.Emit.CustomAttributeBuilder]::new(
            $constructor, [object[]]@('kernel32.dll'), $fields, $values))
        $method.SetImplementationFlags(
            [Reflection.MethodImplAttributes]($method.GetMethodImplementationFlags() -bor
                [Reflection.MethodImplAttributes]::PreserveSig))
    }
    Add-Native 'CreateFileW' ([IntPtr]) @([string],[uint32],[uint32],[IntPtr],[uint32],[uint32],[IntPtr])
    Add-Native 'GetFileInformationByHandleEx' ([bool]) @([IntPtr],[int],[IntPtr],[uint32])
    Add-Native 'GetFinalPathNameByHandleW' ([uint32]) @([IntPtr],[Text.StringBuilder],[uint32],[uint32])
    Add-Native 'SetFileInformationByHandle' ([bool]) @([IntPtr],[int],[IntPtr],[uint32])
    $null = $builder.CreateType()
    $script:backupNativeInitialized = $true
}

function Open-BackupObject([string]$path, [bool]$directory, [bool]$delete) {
    $access = [uint32]0x80
    if (-not $directory) { $access = $access -bor [uint32]2147483648 }
    if ($delete) { $access = $access -bor [uint32]0x10000 }
    $flags = [uint32]0x200000 # OPEN_REPARSE_POINT
    if ($directory) { $flags = $flags -bor [uint32]0x2000000 }
    $raw = [GogokeBackupNative]::CreateFileW($path, $access, [uint32]1,
        [IntPtr]::Zero, [uint32]3, $flags, [IntPtr]::Zero)
    if ($raw -eq [IntPtr]::Zero -or $raw.ToInt64() -eq -1) {
        throw "old backup object could not be opened: $path"
    }
    return [Microsoft.Win32.SafeHandles.SafeFileHandle]::new($raw, $true)
}

function Backup-Info([IntPtr]$handle, [int]$kind, [int]$length) {
    $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal($length)
    try {
        if (-not [GogokeBackupNative]::GetFileInformationByHandleEx($handle, $kind,
            $buffer, [uint32]$length)) { throw 'old backup handle information unavailable' }
        $bytes = [byte[]]::new($length)
        [Runtime.InteropServices.Marshal]::Copy($buffer, $bytes, 0, $length)
        return ,$bytes
    } finally { [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer) }
}

function Backup-Identity([IntPtr]$handle) {
    $bytes = Backup-Info $handle 18 24
    $id = ([BitConverter]::ToString($bytes, 8, 16)).Replace('-', '').ToLowerInvariant()
    if ($id -ceq ('0' * 32)) { throw 'old backup file ID unavailable' }
    return @{ volumeSerialNumber = [BitConverter]::ToUInt64($bytes, 0).ToString(); fileId = $id }
}

function Assert-BackupOpened([IntPtr]$handle, [string]$path, [bool]$directory,
    [object]$expected) {
    $attributes = [BitConverter]::ToUInt32((Backup-Info $handle 9 8), 0)
    $name = [Text.StringBuilder]::new(32768)
    $count = [GogokeBackupNative]::GetFinalPathNameByHandleW($handle, $name,
        [uint32]$name.Capacity, [uint32]0)
    if ($count -eq 0 -or $count -ge $name.Capacity -or
        -not $name.ToString().StartsWith('\\?\', [StringComparison]::Ordinal) -or
        ($attributes -band [uint32]0x400) -ne 0 -or
        ((($attributes -band [uint32]0x10) -ne 0) -ne $directory) -or
        -not [string]::Equals($name.ToString().Substring(4), $path,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "old backup opened object is unsafe: $path"
    }
    $actual = Backup-Identity $handle
    if ($expected -and ($actual.volumeSerialNumber -cne [string]$expected.volumeSerialNumber -or
            $actual.fileId -cne [string]$expected.fileId)) {
        throw "old backup object identity changed: $path"
    }
    return $actual
}

function Pin-BackupDirectory([string]$path, [object]$expected) {
    if ($script:backupPins.ContainsKey($path)) {
        $pin = $script:backupPins[$path]
        $null = Assert-BackupOpened $pin.handle.DangerousGetHandle() $path $true $expected
        return
    }
    $handle = Open-BackupObject $path $true $false
    try {
        $identity = Assert-BackupOpened $handle.DangerousGetHandle() $path $true $expected
        $script:backupPins.Add($path, @{ handle = $handle; identity = $identity })
    } catch { $handle.Dispose(); throw }
}

function Pin-BackupAncestors([string]$path, [object]$rootIdentity) {
    $full = [IO.Path]::GetFullPath($path)
    if ($full -cnotmatch '^[A-Za-z]:\\') { throw 'old backup path must be drive absolute' }
    $current = $full.Substring(0, 3)
    Pin-BackupDirectory $current $null
    foreach ($part in $full.Substring(3).Split([char]'\')) {
        if (-not $part) { continue }
        $current = [IO.Path]::Combine($current, $part)
        Pin-BackupDirectory $current $(if ([string]::Equals($current, $backup,
            [StringComparison]::OrdinalIgnoreCase)) { $rootIdentity } else { $null })
    }
}

function Assert-BackupCustody([object]$inventory, [string]$newInstance) {
    if (-not $lifecycleLock -or $lifecycleLock.SafeFileHandle.IsClosed -or
        $lifecycleLock.SafeFileHandle.IsInvalid) { throw 'install lifecycle custody lost' }
    Assert-InstallRegistration $newInstance
    $rootPin = $script:backupPins[$backup]
    if (-not $rootPin) { throw 'old backup root is not pinned' }
    $null = Assert-BackupOpened $rootPin.handle.DangerousGetHandle() $backup $true `
        $inventory.rootIdentity
}

function Verify-BackupFile([object]$entry, [bool]$delete, [object]$inventory,
    [string]$newInstance) {
    Assert-BackupCustody $inventory $newInstance
    $handle = Open-BackupObject $entry.backupPath $false $delete
    $stream = $null
    try {
        $raw = $handle.DangerousGetHandle()
        $null = Assert-BackupOpened $raw $entry.backupPath $false $entry.identity
        $stream = [IO.FileStream]::new($handle, [IO.FileAccess]::Read)
        $sha = [Security.Cryptography.SHA256]::Create()
        try { $hash = ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
        finally { $sha.Dispose() }
        if ($hash -cne [string]$entry.sha256) { throw "old backup file bytes changed: $($entry.backupPath)" }
        $null = Assert-BackupOpened $raw $entry.backupPath $false $entry.identity
        Assert-BackupCustody $inventory $newInstance
        if ($delete) { Delete-BackupOpened $handle }
    } finally {
        if ($stream) { $stream.Dispose() }
        $handle.Dispose()
    }
    if ($delete -and (Test-Path -LiteralPath $entry.backupPath)) {
        throw "old backup deletion unconfirmed: $($entry.backupPath)"
    }
}

function Delete-BackupOpened([Microsoft.Win32.SafeHandles.SafeFileHandle]$handle) {
    $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal(1)
    try {
        [Runtime.InteropServices.Marshal]::WriteByte($buffer, 0, 1)
        if (-not [GogokeBackupNative]::SetFileInformationByHandle(
            $handle.DangerousGetHandle(), 4, $buffer, [uint32]1)) {
            throw "old backup handle deletion failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
        }
    } finally { [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer) }
}

function Invoke-OwnedBackupCleanup([object]$inventory, [string]$newInstance) {
    Initialize-BackupNative
    $script:backupPins = [System.Collections.Generic.Dictionary[string,object]]::new(
        [StringComparer]::OrdinalIgnoreCase)
    $mapped = [System.Collections.Generic.List[object]]::new()
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $prefix = $targetFull.TrimEnd('\') + '\'
    try {
        Pin-BackupAncestors $backup $inventory.rootIdentity
        foreach ($entry in $inventory.files) {
            # Rust PathBuf may retain '/' inside a relative component. Treat
            # it as a separator before applying the strict absolute/path
            # traversal checks; object IDs and hashes still bind the target.
            $source = ([string]$entry.path).Replace('/', '\')
            if (-not $source.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw 'old backup inventory path is outside the old root'
            }
            if (-not [string]::Equals([IO.Path]::GetFullPath($source), $source,
                [StringComparison]::OrdinalIgnoreCase)) {
                throw 'old backup inventory path is not normalized'
            }
            if ([string]$entry.sha256 -cnotmatch '^[0-9a-f]{64}$') {
                throw 'old backup inventory hash is invalid'
            }
            if ([string]$entry.identity.volumeSerialNumber -cnotmatch '^[0-9]+$' -or
                [string]$entry.identity.fileId -cnotmatch '^[0-9a-f]{32}$') {
                throw 'old backup inventory object identity is invalid'
            }
            if (-not $seen.Add($source)) { throw 'old backup inventory path is duplicated' }
            $relative = $source.Substring($prefix.Length)
            if ($relative.Split([char]'\') | Where-Object { $_ -in @('', '.', '..') }) {
                throw 'old backup inventory path is unsafe'
            }
            $destination = [IO.Path]::Combine($backup, $relative)
            $parent = [IO.Path]::GetDirectoryName($destination)
            Pin-BackupAncestors $parent $inventory.rootIdentity
            $mapped.Add(@{ backupPath = $destination; sha256 = $entry.sha256;
                identity = $entry.identity })
        }
        foreach ($entry in $mapped) { Verify-BackupFile $entry $false $inventory $newInstance }
        foreach ($entry in $mapped) { Verify-BackupFile $entry $true $inventory $newInstance }
        # Empty directories only. Unknown/user files make disposition fail and
        # leave their containing directories and the unique retained root.
        $directories = @($script:backupPins.Keys | Where-Object {
            $_.StartsWith($backup, [StringComparison]::OrdinalIgnoreCase)
        } | Sort-Object Length -Descending)
        foreach ($directory in $directories) {
            Assert-InstallRegistration $newInstance
            $pin = $script:backupPins[$directory]
            $pin.handle.Dispose()
            $null = $script:backupPins.Remove($directory)
            $handle = Open-BackupObject $directory $true $true
            try {
                $null = Assert-BackupOpened $handle.DangerousGetHandle() $directory $true $pin.identity
                $buffer = [Runtime.InteropServices.Marshal]::AllocHGlobal(1)
                try {
                    [Runtime.InteropServices.Marshal]::WriteByte($buffer, 0, 1)
                    if (-not [GogokeBackupNative]::SetFileInformationByHandle(
                        $handle.DangerousGetHandle(), 4, $buffer, [uint32]1)) {
                        $code = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
                        if ($code -ne 145) { throw "old backup directory deletion failed: $code" }
                    }
                } finally { [Runtime.InteropServices.Marshal]::FreeHGlobal($buffer) }
            } finally { $handle.Dispose() }
        }
    } finally {
        foreach ($pin in $script:backupPins.Values) { $pin.handle.Dispose() }
        $script:backupPins.Clear()
    }
}

# The coordinator and NSIS must hold the same file object continuously. The
# handle list gives only the installer created here a duplicate; a copied
# command line cannot borrow the coordinator's exclusive lock.
function Invoke-LockedInstaller([string]$path, [string]$target, [bool]$silent,
    [IO.FileStream]$heldLock) {
    if ([IntPtr]::Size -ne 8 -or $target.IndexOfAny([char[]]@('"', [char]13, [char]10)) -ge 0) {
        throw "unsupported installer handoff path or process architecture"
    }
    $assemblyName = [Reflection.AssemblyName]::new('GogokeUpdateNative')
    $assembly = [AppDomain]::CurrentDomain.DefineDynamicAssembly(
        $assemblyName, [Reflection.Emit.AssemblyBuilderAccess]::Run)
    $module = $assembly.DefineDynamicModule('GogokeUpdateNative')
    $builder = $module.DefineType('GogokeUpdateNative',
        [Reflection.TypeAttributes]::Public -bor [Reflection.TypeAttributes]::Sealed)
    $methodFlags = [Reflection.MethodAttributes]::Public -bor
        [Reflection.MethodAttributes]::Static -bor [Reflection.MethodAttributes]::PinvokeImpl
    function Native-Method([string]$name, [Type]$result, [Type[]]$arguments) {
        $method = $builder.DefineMethod($name, $methodFlags, $result, $arguments)
        $attribute = [Runtime.InteropServices.DllImportAttribute]
        $constructor = $attribute.GetConstructor([Type[]]@([string]))
        $fields = [Reflection.FieldInfo[]]@(
            $attribute.GetField('SetLastError'), $attribute.GetField('CharSet'),
            $attribute.GetField('ExactSpelling'), $attribute.GetField('CallingConvention'))
        $values = [object[]]@($true, [Runtime.InteropServices.CharSet]::Unicode,
            $true, [Runtime.InteropServices.CallingConvention]::Winapi)
        $method.SetCustomAttribute([Reflection.Emit.CustomAttributeBuilder]::new(
            $constructor, [object[]]@('kernel32.dll'), $fields, $values))
        $method.SetImplementationFlags(
            [Reflection.MethodImplAttributes]($method.GetMethodImplementationFlags() -bor
                [Reflection.MethodImplAttributes]::PreserveSig))
    }
    Native-Method 'GetCurrentProcess' ([IntPtr]) @()
    Native-Method 'DuplicateHandle' ([bool]) @([IntPtr],[IntPtr],[IntPtr],[IntPtr],[uint32],[bool],[uint32])
    Native-Method 'InitializeProcThreadAttributeList' ([bool]) @([IntPtr],[uint32],[uint32],[IntPtr])
    Native-Method 'UpdateProcThreadAttribute' ([bool]) @([IntPtr],[uint32],[IntPtr],[IntPtr],[IntPtr],[IntPtr],[IntPtr])
    Native-Method 'DeleteProcThreadAttributeList' ([void]) @([IntPtr])
    Native-Method 'CreateProcessW' ([bool]) @([string],[IntPtr],[IntPtr],[IntPtr],[bool],[uint32],[IntPtr],[string],[IntPtr],[IntPtr])
    Native-Method 'WaitForSingleObject' ([uint32]) @([IntPtr],[uint32])
    Native-Method 'GetExitCodeProcess' ([bool]) @([IntPtr],[IntPtr])
    Native-Method 'CloseHandle' ([bool]) @([IntPtr])
    $null = $builder.CreateType()

    $duplicateAddress = [IntPtr]::Zero
    $sizeAddress = [IntPtr]::Zero
    $attributes = [IntPtr]::Zero
    $handleList = [IntPtr]::Zero
    $startup = [IntPtr]::Zero
    $processInfo = [IntPtr]::Zero
    $command = [IntPtr]::Zero
    $duplicate = [IntPtr]::Zero
    $process = [IntPtr]::Zero
    $thread = [IntPtr]::Zero
    $attributesInitialized = $false
    try {
        $duplicateAddress = [Runtime.InteropServices.Marshal]::AllocHGlobal(8)
        [Runtime.InteropServices.Marshal]::WriteIntPtr($duplicateAddress, [IntPtr]::Zero)
        $self = [GogokeUpdateNative]::GetCurrentProcess()
        if (-not [GogokeUpdateNative]::DuplicateHandle($self,
            $heldLock.SafeFileHandle.DangerousGetHandle(), $self, $duplicateAddress,
            [uint32]0, $true, [uint32]2)) {
            throw "installer lifecycle handle duplication failed"
        }
        $duplicate = [Runtime.InteropServices.Marshal]::ReadIntPtr($duplicateAddress)
        if ($duplicate -eq [IntPtr]::Zero -or $duplicate.ToInt64() -eq -1) {
            throw "installer lifecycle duplicate is invalid"
        }
        $handleList = [Runtime.InteropServices.Marshal]::AllocHGlobal(8)
        [Runtime.InteropServices.Marshal]::WriteIntPtr($handleList, $duplicate)
        $sizeAddress = [Runtime.InteropServices.Marshal]::AllocHGlobal(8)
        [Runtime.InteropServices.Marshal]::WriteInt64($sizeAddress, 0)
        $null = [GogokeUpdateNative]::InitializeProcThreadAttributeList(
            [IntPtr]::Zero, [uint32]1, [uint32]0, $sizeAddress)
        $size = [Runtime.InteropServices.Marshal]::ReadInt64($sizeAddress)
        if ($size -le 0 -or $size -gt 4096) { throw "installer attribute list size is invalid" }
        $attributes = [Runtime.InteropServices.Marshal]::AllocHGlobal([int]$size)
        if (-not [GogokeUpdateNative]::InitializeProcThreadAttributeList(
            $attributes, [uint32]1, [uint32]0, $sizeAddress)) {
            throw "installer attribute list initialization failed"
        }
        $attributesInitialized = $true
        if (-not [GogokeUpdateNative]::UpdateProcThreadAttribute($attributes,
            [uint32]0, [IntPtr]0x00020002, $handleList, [IntPtr]8,
            [IntPtr]::Zero, [IntPtr]::Zero)) {
            throw "installer lifecycle handle list failed"
        }

        # STARTUPINFOEXW is 112 bytes on x64; its attribute pointer is at 104.
        # PROCESS_INFORMATION holds two handles followed by two DWORD IDs.
        $startup = [Runtime.InteropServices.Marshal]::AllocHGlobal(112)
        [Runtime.InteropServices.Marshal]::Copy([byte[]]::new(112), 0, $startup, 112)
        [Runtime.InteropServices.Marshal]::WriteInt32($startup, 112)
        [Runtime.InteropServices.Marshal]::WriteIntPtr($startup, 104, $attributes)
        $processInfo = [Runtime.InteropServices.Marshal]::AllocHGlobal(24)
        [Runtime.InteropServices.Marshal]::Copy([byte[]]::new(24), 0, $processInfo, 24)
        $commandLine = '"' + $path + '"'
        if ($silent) { $commandLine += ' /S' }
        $commandLine += " /GOGOKE_LOCK_HANDLE=$($duplicate.ToInt64()) /D=$target"
        $command = [Runtime.InteropServices.Marshal]::StringToHGlobalUni($commandLine)
        if (-not [GogokeUpdateNative]::CreateProcessW($path, $command,
            [IntPtr]::Zero, [IntPtr]::Zero, $true, [uint32]0x00080000,
            [IntPtr]::Zero, (Split-Path -Parent $path), $startup, $processInfo)) {
            throw "installer creation with lifecycle handle failed"
        }
        $script:installerCompletionUnknown = $true
        $process = [Runtime.InteropServices.Marshal]::ReadIntPtr($processInfo)
        $thread = [Runtime.InteropServices.Marshal]::ReadIntPtr($processInfo, 8)
        $null = [GogokeUpdateNative]::CloseHandle($duplicate)
        $duplicate = [IntPtr]::Zero
        if ([GogokeUpdateNative]::WaitForSingleObject($process, [uint32]::MaxValue) -ne 0) {
            throw "installer process wait failed; target and backup require recovery"
        }
        $script:installerCompletionUnknown = $false
        $exitAddress = [Runtime.InteropServices.Marshal]::AllocHGlobal(4)
        try {
            if (-not [GogokeUpdateNative]::GetExitCodeProcess($process, $exitAddress)) {
                throw "installer exit status is unavailable"
            }
            return [Runtime.InteropServices.Marshal]::ReadInt32($exitAddress)
        } finally {
            [Runtime.InteropServices.Marshal]::FreeHGlobal($exitAddress)
        }
    } finally {
        if ($thread -ne [IntPtr]::Zero) { $null = [GogokeUpdateNative]::CloseHandle($thread) }
        if ($process -ne [IntPtr]::Zero) { $null = [GogokeUpdateNative]::CloseHandle($process) }
        if ($duplicate -ne [IntPtr]::Zero) { $null = [GogokeUpdateNative]::CloseHandle($duplicate) }
        if ($attributesInitialized) { [GogokeUpdateNative]::DeleteProcThreadAttributeList($attributes) }
        foreach ($address in @($command, $processInfo, $startup, $handleList,
                $attributes, $sizeAddress, $duplicateAddress)) {
            if ($address -ne [IntPtr]::Zero) {
                [Runtime.InteropServices.Marshal]::FreeHGlobal($address)
            }
        }
    }
}

function Invoke-Reg([string[]]$arguments) {
    if (-not (Test-Path -LiteralPath $regExe -PathType Leaf)) { throw "reg.exe is unavailable" }
    $previousPreference = $ErrorActionPreference
    $previousExitCode = $global:LASTEXITCODE
    try {
        $ErrorActionPreference = "Continue"
        $global:LASTEXITCODE = $null
        & $regExe @arguments 1>$null 2>$null
        $exitCode = $global:LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
        $global:LASTEXITCODE = $previousExitCode
    }
    if ($null -eq $exitCode) { throw "reg.exe did not return an exit code" }
    return [int]$exitCode
}

try {
    Assert-Absolute $Installer "Installer"
    Assert-Absolute $ReleaseSetDirectory "ReleaseSetDirectory"
    Assert-Absolute $CurrentExe "CurrentExe"
    Assert-Absolute $TargetDir "TargetDir"
    Assert-Absolute $ReadyFile "ReadyFile"
    Assert-Absolute $LockFile "LockFile"
    Assert-Absolute $FailureLog "FailureLog"
    Assert-Absolute $StateFile "StateFile"
    Assert-Absolute $RetentionNoticeFile "RetentionNoticeFile"
    if (-not (Test-Path -LiteralPath $Installer -PathType Leaf)) { throw "verified installer is missing" }
    if ($ExpectedSha256 -notmatch '^[0-9A-Fa-f]{64}$') { throw "ExpectedSha256 is invalid" }
    if ($ExpectedManifestSha256 -notmatch '^[0-9A-Fa-f]{64}$' -or
        $ExpectedSignatureSha256 -notmatch '^[0-9A-Fa-f]{64}$') {
        throw "expected signed sidecar hashes are invalid"
    }
    if ($ExpectedGenerationId -cnotmatch '^[0-9a-f]{64}$') { throw "ExpectedGenerationId is invalid" }
    $targetFull = [IO.Path]::GetFullPath($TargetDir).TrimEnd('\')
    $backup = "$targetFull.update-backup-$backupTag"
    $targetRoot = [IO.Path]::GetPathRoot($targetFull).TrimEnd('\')
    if ([string]::Equals($targetFull, $targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw "target directory cannot be a volume root"
    }
    $expectedCurrent = Join-Path $targetFull "gogoke.exe"
    if (-not [string]::Equals([IO.Path]::GetFullPath($CurrentExe), $expectedCurrent,
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "current executable does not belong to target directory"
    }
    if (-not [string]::Equals([IO.Path]::GetFullPath((Split-Path -Parent $Installer)),
        [IO.Path]::GetFullPath($ReleaseSetDirectory), [StringComparison]::OrdinalIgnoreCase)) {
        throw "installer is outside the signed release set"
    }

    Set-Phase "lock"
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $LockFile) | Out-Null
    $lock = [IO.File]::Open($LockFile, [IO.FileMode]::OpenOrCreate,
        [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
    $lifecyclePath = Join-Path (Split-Path -Parent $targetFull) "gogoke-install-lifecycle.lock"
    $lifecycleLock = Acquire-LifecycleLock

    Set-Phase "wait-parent"
    $parent = Get-Process -Id $ParentPid -ErrorAction SilentlyContinue
    if ($parent -and -not $parent.WaitForExit(30000)) { throw "gogoke did not exit within 30 seconds" }
    $oldApplicationStopped = $true

    Set-Phase "verify-release-set"
    $verifyArgument = '--gogoke-verify-install-set="' + $ReleaseSetDirectory + '"'
    $verifyProcess = Start-Process -FilePath $CurrentExe -ArgumentList @($verifyArgument) -Wait -PassThru -WindowStyle Hidden
    if ($verifyProcess.ExitCode -ne 0) { throw "Owner signed release set failed verification" }
    $manifestFile = Join-Path $ReleaseSetDirectory "SHA256SUMS.windows"
    $signatureFile = Join-Path $ReleaseSetDirectory "SHA256SUMS.windows.sig"
    if (-not [string]::Equals((Get-Sha256Hex $manifestFile), $ExpectedManifestSha256,
        [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals((Get-Sha256Hex $signatureFile), $ExpectedSignatureSha256,
            [StringComparison]::OrdinalIgnoreCase)) {
        throw "Owner signed release sidecars changed after application authorization"
    }
    $installerHash = Get-Sha256Hex $Installer
    if (-not [string]::Equals($installerHash, $ExpectedSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "installer changed after application authorization"
    }

    Set-Phase "prepare-target"
    if ((Test-Path -LiteralPath $TargetDir) -and
        ((Get-Item -LiteralPath $TargetDir -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "target directory is a reparse point"
    }
    if (Test-Path -LiteralPath $backup) { throw 'new backup custody path is occupied' }
    $hadUninstallRegistration = (Invoke-Reg @("query", $uninstallKey)) -eq 0
    if (-not $hadUninstallRegistration) { throw "old gogoke install registration is missing" }
    if ($hadUninstallRegistration) {
        $oldRegistration = Get-InstallRegistration
        if (-not $oldRegistration -or $oldRegistration.InstallDomain -ne "OWNER_RELEASE" -or
            [string]::IsNullOrWhiteSpace($oldRegistration.InstallInstanceId)) {
            throw "old gogoke install registration is invalid"
        }
        $oldInstallInstanceId = [string]$oldRegistration.InstallInstanceId
        Assert-InstallRegistration $oldInstallInstanceId
        if ((Invoke-Reg @("export", $uninstallKey, $registryBackup, "/y")) -ne 0) {
            throw "could not back up gogoke install registration"
        }
    }
    if (Test-Path -LiteralPath $TargetDir) {
        $owned = Test-Path -LiteralPath (Join-Path $TargetDir "gogoke.exe") -PathType Leaf
        if (-not $owned) { throw "target directory is not owned by gogoke" }
        $oldExeSha256 = Get-Sha256Hex (Join-Path $TargetDir "gogoke.exe")
        $oldInventory = Get-OldOwnedInventory
        Move-Item -LiteralPath $TargetDir -Destination $backup
        $targetMoved = $true
        Initialize-BackupNative
        $oldRoot = Open-BackupObject $backup $true $false
        try {
            $null = Assert-BackupOpened $oldRoot.DangerousGetHandle() $backup $true `
                $oldInventory.rootIdentity
        } finally { $oldRoot.Dispose() }
    }

    Set-Phase "install"
    $installStarted = $true
    $installerExitCode = Invoke-LockedInstaller $Installer $targetFull ([bool]$Silent) $lifecycleLock
    if ($installerExitCode -ne 0) { throw "installer exited with code $installerExitCode" }
    if ($targetMoved -and -not (Test-Path -LiteralPath $backup -PathType Container)) {
        throw "old gogoke installation backup changed during installer handoff"
    }
    if ($targetMoved) { Assert-PhysicalDirectory $backup "old installation backup" }
    if ($targetMoved -and -not [string]::Equals(
        (Get-Sha256Hex (Join-Path $backup "gogoke.exe")), $oldExeSha256,
        [StringComparison]::OrdinalIgnoreCase)) {
        throw "old gogoke executable backup changed during installer handoff"
    }
    $newRegistration = Get-InstallRegistration
    if (-not $newRegistration -or [string]::IsNullOrWhiteSpace($newRegistration.InstallInstanceId) -or
        $newRegistration.InstallInstanceId -eq $oldInstallInstanceId -or
        $newRegistration.DisplayVersion -cne $ExpectedVersion) {
        throw "new gogoke installation registration is invalid"
    }
    $newInstallInstanceId = [string]$newRegistration.InstallInstanceId
    Assert-InstallRegistration $newInstallInstanceId
    if (-not [string]::Equals((Get-Sha256Hex (Join-Path $TargetDir "SHA256SUMS.windows")),
        $ExpectedManifestSha256, [StringComparison]::OrdinalIgnoreCase) -or
        -not [string]::Equals((Get-Sha256Hex (Join-Path $TargetDir "SHA256SUMS.windows.sig")),
            $ExpectedSignatureSha256, [StringComparison]::OrdinalIgnoreCase)) {
        throw "installed Owner signed sidecars do not match the prepared set"
    }

    Set-Phase "launch"
    $newExe = Join-Path $TargetDir "gogoke.exe"
    if (-not (Test-Path -LiteralPath $newExe -PathType Leaf)) { throw "installed gogoke executable is missing" }
    if (Test-Path -LiteralPath $ReadyFile) { Remove-Item -LiteralPath $ReadyFile -Force }
    # Windows PowerShell joins -ArgumentList arrays without quoting each item.
    # Keep the entire readiness argument together when the temp path has spaces.
    $readyArgument = '"--gogoke-update-ready=' + $ReadyFile + '"'
    $newProcess = Start-Process -FilePath $newExe -ArgumentList $readyArgument -PassThru

    Set-Phase "wait-ready"
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        Assert-InstallRegistration $newInstallInstanceId
        if (Test-Path -LiteralPath $ReadyFile -PathType Leaf) {
            try {
                $ready = [IO.File]::ReadAllText($ReadyFile) | ConvertFrom-Json -ErrorAction Stop
            } catch {
                throw "new application readiness receipt is not JSON"
            }
            if ($ready.version -cne $ExpectedVersion -or
                $ready.generationId -cne $ExpectedGenerationId) {
                throw "new application reported the wrong version or resource generation"
            }
            Write-UpdateState 'installed_cleanup_pending' "new install committed; old owned cleanup pending at $backup"
            $newInstallCommitted = $true
            Set-Phase "cleanup-old-owned"
            Invoke-OwnedBackupCleanup $oldInventory $newInstallInstanceId
            if (Test-Path -LiteralPath $backup) {
                [IO.File]::AppendAllText($RetentionNoticeFile,
                    "The new installation is ready. Unlisted files remain at $backup.`r`n",
                    [Text.UTF8Encoding]::new($false))
                Write-UpdateState 'installed_backup_retained' "unlisted old backup files retained at $backup"
            } else {
                Write-UpdateState 'installed' ''
            }
            Set-Phase "complete"
            Remove-Item -LiteralPath $ReadyFile -Force -ErrorAction SilentlyContinue
            if (Test-Path -LiteralPath $registryBackup) {
                Remove-Item -LiteralPath $registryBackup -Force -ErrorAction SilentlyContinue
            }
            if (Test-Path -LiteralPath $FailureLog) {
                Remove-Item -LiteralPath $FailureLog -Force -ErrorAction SilentlyContinue
            }
            exit 0
        }
        $newProcess.Refresh()
        if ($newProcess.HasExited) { throw "new application exited before reporting readiness" }
        Start-Sleep -Milliseconds 100
    }
    throw "new application did not report readiness within 30 seconds"
} catch {
    $initialError = $_.Exception.Message
    Write-Failure $initialError
    if ($newInstallCommitted) {
        # New readiness was committed before any possible old-file deletion.
        # A partially deleted backup can never serve as a rollback source.
        try {
            Write-UpdateState 'installed_backup_retained' "old owned cleanup incomplete at $backup : $initialError"
            [IO.File]::AppendAllText($RetentionNoticeFile,
                "The new installation is ready. Old backup cleanup stopped at $backup : $initialError`r`n",
                [Text.UTF8Encoding]::new($false))
        } catch { Write-Failure "committed install cleanup state could not be recorded: $($_.Exception.Message)" }
        exit 1
    }
    if ($installerCompletionUnknown) {
        Write-Failure "$initialError`r`ninstaller completion is unknown; target and backup retained for recovery"
        exit 1
    }
    if ($newProcess -and -not $newProcess.HasExited) {
        Stop-Process -Id $newProcess.Id -Force -ErrorAction SilentlyContinue
        $null = $newProcess.WaitForExit(5000)
    }
    try {
        Set-Phase "rollback"
        if (($targetMoved -or $installStarted) -and -not $lifecycleLock) {
            throw "lifecycle lock was not held; target and backup retained for safe recovery"
        }
        $registrationDuringRollback = Get-InstallRegistration
        if ($registrationDuringRollback -and
            $registrationDuringRollback.InstallInstanceId -ne $oldInstallInstanceId -and
            $registrationDuringRollback.InstallInstanceId -ne $newInstallInstanceId) {
            throw "installation instance changed; target and backup retained for safe recovery"
        }
        if ($targetMoved -or $installStarted) {
            if (Test-Path -LiteralPath $TargetDir) {
                Assert-PhysicalDirectory $TargetDir "failed new installation"
                $failedTarget = "$TargetDir.update-failed-$([Guid]::NewGuid().ToString('N'))"
                Move-Item -LiteralPath $TargetDir -Destination $failedTarget
                Write-Failure "$initialError`r`nfailed new installation retained at $failedTarget; old installation restoration in progress"
            }
            if ($targetMoved) {
                Assert-PhysicalDirectory $backup "old installation backup"
                if (-not [string]::Equals((Get-Sha256Hex (Join-Path $backup "gogoke.exe")),
                    $oldExeSha256, [StringComparison]::OrdinalIgnoreCase)) {
                    throw "old executable backup changed during rollback"
                }
                Move-Item -LiteralPath $backup -Destination $TargetDir
            }
            if ($installStarted -and $hadUninstallRegistration -and
                -not (Test-Path -LiteralPath $registryBackup -PathType Leaf)) {
                throw "gogoke install registration backup is missing during rollback"
            }
            if (Test-Path -LiteralPath $registryBackup -PathType Leaf) {
                if ((Invoke-Reg @("import", $registryBackup)) -ne 0) {
                    throw "could not restore gogoke install registration"
                }
                Remove-Item -LiteralPath $registryBackup -Force
            } elseif ($installStarted -and -not $hadUninstallRegistration -and
                (Invoke-Reg @("query", $uninstallKey)) -eq 0) {
                if ((Invoke-Reg @("delete", $uninstallKey, "/f")) -ne 0) {
                    throw "could not remove new gogoke install registration"
                }
            }
        } elseif (Test-Path -LiteralPath $registryBackup -PathType Leaf) {
            Remove-Item -LiteralPath $registryBackup -Force
        }
        if ($oldApplicationStopped -and -not $NoRestartOnRollback -and
            (Test-Path -LiteralPath $CurrentExe -PathType Leaf)) {
            Start-Process -FilePath $CurrentExe | Out-Null
        }
    } catch {
        Write-Failure "rollback failed: $($_.Exception.Message)"
    }
    exit 1
} finally {
    if ($lifecycleLock) { $lifecycleLock.Dispose() }
    if ($lock) { $lock.Dispose() }
}
