//! Installed-shell uninstall entry. The resource authority supplies every owned byte.
//! The fixed PowerShell finalizer is embedded in this executable, never read from a resource set.

use crate::resource_trust;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::mem::ManuallyDrop;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use windows::core::{Interface, BSTR, GUID, PCWSTR};
use windows::Win32::Foundation::{HGLOBAL, PROPERTYKEY};
use windows::Win32::System::Com::StructuredStorage::{
    CreateStreamOnHGlobal, PropVariantClear, PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0,
    PROPVARIANT_0_0_0,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, IPersistStream,
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STREAM_SEEK_END, STREAM_SEEK_SET,
};
use windows::Win32::System::Variant::VT_BSTR;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, SHGetKnownFolderPath, ShellLink,
};
use windows_sys::Win32::Foundation::GetHandleInformation;
use windows_sys::Win32::Foundation::{
    DuplicateHandle, DUPLICATE_SAME_ACCESS, GENERIC_READ, GENERIC_WRITE, HANDLE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FileAttributeTagInfo, FileDispositionInfoEx, FileIdInfo, GetFileInformationByHandleEx,
    GetFinalPathNameByHandleW, SetFileInformationByHandle, DELETE, FILE_ATTRIBUTE_TAG_INFO,
    FILE_DISPOSITION_FLAG_ON_CLOSE, FILE_DISPOSITION_INFO_EX, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_DELETE_ON_CLOSE, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const REPARSE_POINT: u32 = 0x400;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const FINALIZER: &str = include_str!("../update/gogoke-uninstall-finalizer.ps1");
// Keep the command line below CreateProcess's limit. The fixed script comes
// from this executable over its private stdin before the JSON handoff.
const FINALIZER_BOOTSTRAP: &str = r#"
$ErrorActionPreference = 'Stop'
$encodedScript = [Console]::In.ReadLine()
if (-not $encodedScript -or $encodedScript.Length -gt 131072) {
    throw 'GOGOKE_UNINSTALL_EMBEDDED_SCRIPT_INVALID'
}
$source = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String($encodedScript))
& ([ScriptBlock]::Create($source))
"#;

#[derive(Eq, PartialEq, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OpenedObjectIdentity {
    // Keep the 64-bit value as text so the JSON handoff cannot round it.
    volume_serial_number: String,
    file_id: String,
}

#[derive(Serialize)]
struct OwnedFile {
    path: String,
    sha256: String,
    identity: OpenedObjectIdentity,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OwnedShortcut {
    schema: String,
    instance: String,
    root: String,
    slot: String,
    folder: String,
    sha256: String,
    identity: OpenedObjectIdentity,
}

const SHORTCUT_DESKTOP_VALUE: &str = "GogokeShortcutDesktopV1";
const SHORTCUT_START_VALUE: &str = "GogokeShortcutStartV1";
const SHORTCUT_SCHEMA: &str = "gogoke.shortcut-ownership.v1";
const SHORTCUT_NAME: &str = "gogoke.lnk";
// Matches tauri.conf.json's identifier and the pinned NSIS
// SetLnkAppUserModelId macro's bundle-id property value.
const SHORTCUT_APP_USER_MODEL_ID: &str = "app.gogoke.desktop";
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3),
    pid: 5,
};

fn final_handle_path(handle: HANDLE) -> Result<PathBuf, String> {
    let mut buffer = [0u16; 32768];
    let len =
        unsafe { GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0) }
            as usize;
    if len == 0 || len >= buffer.len() {
        return Err("GOGOKE_SHORTCUT_HANDLE_PATH_UNAVAILABLE".to_string());
    }
    let text = String::from_utf16(&buffer[..len])
        .map_err(|_| "GOGOKE_SHORTCUT_HANDLE_PATH_ENCODING".to_string())?;
    let path = text
        .strip_prefix(r"\\?\")
        .ok_or("GOGOKE_SHORTCUT_HANDLE_PATH_INVALID")?;
    Ok(PathBuf::from(path))
}

fn matches_opened_path(handle: HANDLE, path: &Path) -> Result<bool, String> {
    // Rust canonicalize and GetFinalPathNameByHandleW both use the opened
    // filesystem spelling. Every caller already holds the object with no
    // delete sharing, so this second lookup cannot follow a swapped name.
    let expected =
        fs::canonicalize(path).map_err(|_| "GOGOKE_SHORTCUT_PATH_CHANGED".to_string())?;
    let expected = expected.to_str().ok_or("GOGOKE_SHORTCUT_PATH_ENCODING")?;
    let expected = expected.strip_prefix(r"\\?\").unwrap_or(expected);
    Ok(final_handle_path(handle)? == Path::new(expected))
}

fn pin_directory_chain(path: &Path) -> Result<Vec<File>, String> {
    let mut pinned = Vec::new();
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part.as_os_str());
        if !current.is_absolute() {
            continue;
        }
        let opened = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&current)
            .map_err(|_| "GOGOKE_SHORTCUT_DIRECTORY_OPEN_FAILED".to_string())?;
        let meta = opened
            .metadata()
            .map_err(|_| "GOGOKE_SHORTCUT_DIRECTORY_UNAVAILABLE".to_string())?;
        if !meta.is_dir()
            || meta.file_attributes() & REPARSE_POINT != 0
            || !matches_opened_path(opened.as_raw_handle() as HANDLE, &current)?
        {
            return Err("GOGOKE_SHORTCUT_DIRECTORY_UNSAFE".to_string());
        }
        pinned.push(opened);
    }
    Ok(pinned)
}

fn known_folder(desktop: bool) -> Result<PathBuf, String> {
    let id = if desktop {
        &FOLDERID_Desktop
    } else {
        &FOLDERID_Programs
    };
    let raw = unsafe { SHGetKnownFolderPath(id, Default::default(), None) }
        .map_err(|_| "GOGOKE_SHORTCUT_KNOWN_FOLDER_UNAVAILABLE".to_string())?;
    let result = unsafe { raw.to_string() }
        .map(PathBuf::from)
        .map_err(|_| "GOGOKE_SHORTCUT_KNOWN_FOLDER_ENCODING".to_string());
    unsafe {
        CoTaskMemFree(Some(raw.0.cast()));
    }
    result
}

fn shortcut_bytes(exe: &Path, root: &Path) -> Result<Vec<u8>, String> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() {
        return Err("GOGOKE_SHORTCUT_COM_INIT_FAILED".to_string());
    }
    let result = (|| {
        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .map_err(|_| "GOGOKE_SHORTCUT_COM_CREATE_FAILED".to_string())?;
        let target: Vec<u16> = exe.as_os_str().encode_wide().chain(Some(0)).collect();
        let working: Vec<u16> = root.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            link.SetPath(PCWSTR(target.as_ptr()))
                .map_err(|_| "GOGOKE_SHORTCUT_TARGET_FAILED".to_string())?;
            link.SetWorkingDirectory(PCWSTR(working.as_ptr()))
                .map_err(|_| "GOGOKE_SHORTCUT_WORKING_DIRECTORY_FAILED".to_string())?;
        }
        let properties: IPropertyStore = link
            .cast()
            .map_err(|_| "GOGOKE_SHORTCUT_PROPERTY_INTERFACE_FAILED".to_string())?;
        let mut app_id = PROPVARIANT {
            Anonymous: PROPVARIANT_0 {
                Anonymous: ManuallyDrop::new(PROPVARIANT_0_0 {
                    vt: VT_BSTR,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: PROPVARIANT_0_0_0 {
                        bstrVal: ManuallyDrop::new(BSTR::from(SHORTCUT_APP_USER_MODEL_ID)),
                    },
                }),
            },
        };
        let property_result = unsafe {
            properties
                .SetValue(&PKEY_APP_USER_MODEL_ID, &app_id)
                .and_then(|_| properties.Commit())
        };
        let clear_result = unsafe { PropVariantClear(&mut app_id) };
        property_result.map_err(|_| "GOGOKE_SHORTCUT_APP_ID_FAILED".to_string())?;
        clear_result.map_err(|_| "GOGOKE_SHORTCUT_APP_ID_CLEANUP_FAILED".to_string())?;
        let persist: IPersistStream = link
            .cast()
            .map_err(|_| "GOGOKE_SHORTCUT_STREAM_INTERFACE_FAILED".to_string())?;
        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL(std::ptr::null_mut()), true) }
            .map_err(|_| "GOGOKE_SHORTCUT_MEMORY_STREAM_FAILED".to_string())?;
        unsafe { persist.Save(&stream, true) }
            .map_err(|_| "GOGOKE_SHORTCUT_SERIALIZE_FAILED".to_string())?;
        let mut length = 0u64;
        unsafe { stream.Seek(0, STREAM_SEEK_END, Some(&mut length)) }
            .map_err(|_| "GOGOKE_SHORTCUT_STREAM_SIZE_FAILED".to_string())?;
        if length < 76 || length > 65536 {
            return Err("GOGOKE_SHORTCUT_STREAM_SIZE_INVALID".to_string());
        }
        unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }
            .map_err(|_| "GOGOKE_SHORTCUT_STREAM_REWIND_FAILED".to_string())?;
        let mut bytes = vec![0u8; length as usize];
        let mut read = 0u32;
        let status =
            unsafe { stream.Read(bytes.as_mut_ptr().cast(), length as u32, Some(&mut read)) };
        if status.is_err() || read as u64 != length {
            return Err("GOGOKE_SHORTCUT_STREAM_READ_FAILED".to_string());
        }
        Ok(bytes)
    })();
    unsafe {
        CoUninitialize();
    }
    result
}

fn shortcut_record_for(
    file: &mut File,
    root: &Path,
    slot: &str,
    folder: &str,
    instance: &str,
) -> Result<OwnedShortcut, String> {
    let identity = opened_identity(file)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "GOGOKE_SHORTCUT_SEEK_FAILED".to_string())?;
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| "GOGOKE_SHORTCUT_READ_FAILED".to_string())?;
    if bytes.is_empty() || bytes.len() > 65536 {
        return Err("GOGOKE_SHORTCUT_SIZE_INVALID".to_string());
    }
    Ok(OwnedShortcut {
        schema: SHORTCUT_SCHEMA.to_owned(),
        instance: instance.to_owned(),
        root: path_text(root)?,
        slot: slot.to_owned(),
        folder: folder.to_owned(),
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        identity,
    })
}

fn recorded_shortcut(key: &RegKey, value: &str) -> Result<Option<(String, OwnedShortcut)>, String> {
    let raw: String = match key.get_value(value) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("GOGOKE_SHORTCUT_RECORD_READ_FAILED".to_string()),
    };
    if raw.len() > 2048 {
        return Err("GOGOKE_SHORTCUT_RECORD_TOO_LARGE".to_string());
    }
    let record: OwnedShortcut =
        serde_json::from_str(&raw).map_err(|_| "GOGOKE_SHORTCUT_RECORD_INVALID".to_string())?;
    Ok(Some((raw, record)))
}

/// Installer-only entry. NSIS passes precisely one inherited lifecycle-lock
/// handle through STARTUPINFOEX's handle list. It never hands off lock custody.
pub(crate) fn install_shortcut(args: &[String]) -> Result<(), String> {
    if args.len() != 5 || args[0] != "--gogoke-install-shortcut" {
        return Err("GOGOKE_SHORTCUT_ARGUMENTS_INVALID".to_string());
    }
    let slot = args[1].as_str();
    let mut folder = args[2].clone();
    let create = match args[4].as_str() {
        "create" => true,
        "adopt" => false,
        _ => return Err("GOGOKE_SHORTCUT_ARGUMENTS_INVALID".to_string()),
    };
    if !matches!(slot, "desktop" | "start") {
        return Err("GOGOKE_SHORTCUT_SLOT_INVALID".to_string());
    }
    let handle_number: usize = args[3]
        .parse()
        .map_err(|_| "GOGOKE_SHORTCUT_LOCK_HANDLE_INVALID".to_string())?;
    if handle_number == 0 || handle_number == usize::MAX {
        return Err("GOGOKE_SHORTCUT_LOCK_HANDLE_INVALID".to_string());
    }
    let lock_handle = handle_number as HANDLE;
    let mut flags = 0u32;
    if unsafe { GetHandleInformation(lock_handle, &mut flags) } == 0 || flags & 1 == 0 {
        return Err("GOGOKE_SHORTCUT_LOCK_NOT_INHERITED".to_string());
    }
    let exe = std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE".to_string())?;
    let root = exe.parent().ok_or("GOGOKE_INSTALL_ROOT_UNAVAILABLE")?;
    let _root_pins = pin_directory_chain(root)?;
    let canonical_root = final_handle_path(
        _root_pins
            .last()
            .ok_or("GOGOKE_SHORTCUT_ROOT_UNAVAILABLE")?
            .as_raw_handle() as HANDLE,
    )?;
    let expected_lock = canonical_root
        .parent()
        .ok_or("GOGOKE_INSTALL_PARENT_UNAVAILABLE")?
        .join("gogoke-install-lifecycle.lock");
    if final_handle_path(lock_handle)? != expected_lock {
        return Err("GOGOKE_SHORTCUT_LOCK_PATH_INVALID".to_string());
    }
    let mut lock_attributes = FILE_ATTRIBUTE_TAG_INFO::default();
    let lock_info_ok = unsafe {
        GetFileInformationByHandleEx(
            lock_handle,
            FileAttributeTagInfo,
            (&mut lock_attributes as *mut FILE_ATTRIBUTE_TAG_INFO).cast(),
            std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if lock_info_ok == 0 || lock_attributes.FileAttributes & (REPARSE_POINT | 0x10) != 0 {
        return Err("GOGOKE_SHORTCUT_LOCK_UNSAFE".to_string());
    }
    let verified = resource_trust::verify_bootstrap()?;
    if verified.domain != resource_trust::Domain::Formal {
        return Err("GOGOKE_SHORTCUT_FORMAL_ONLY".to_string());
    }
    let instance = verified.registered_instance()?;
    if verified.install_root != root {
        return Err("GOGOKE_SHORTCUT_ROOT_CHANGED".to_string());
    }
    let desktop = slot == "desktop";
    let value = if desktop {
        SHORTCUT_DESKTOP_VALUE
    } else {
        SHORTCUT_START_VALUE
    };
    let registry = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            format!(
                "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}",
                verified.uninstall_registry_key()
            ),
            KEY_READ | KEY_WRITE,
        )
        .map_err(|_| "GOGOKE_SHORTCUT_REGISTRY_OPEN_FAILED".to_string())?;
    let previous = recorded_shortcut(&registry, value)?;
    if !create && !desktop {
        if let Some((_, old)) = &previous {
            if old.schema == SHORTCUT_SCHEMA && old.root == path_text(root)? && old.slot == "start"
            {
                folder = old.folder.clone();
            }
        }
    }
    if (desktop && !folder.is_empty())
        || (!desktop
            && (folder.len() > 80
                || folder == "."
                || folder == ".."
                || folder.ends_with([' ', '.'])
                || folder
                    .chars()
                    .any(|c| c.is_control() || "\\/:*?\"<>|".contains(c))))
    {
        return Err("GOGOKE_SHORTCUT_SLOT_INVALID".to_string());
    }
    let base = known_folder(desktop)?;
    let _base_pins = pin_directory_chain(&base)?;
    let directory = if desktop || folder.is_empty() {
        base
    } else {
        base.join(&folder)
    };
    if !create && !directory.exists() {
        return Ok(());
    }
    if create && !desktop && !folder.is_empty() {
        match fs::create_dir(&directory) {
            Ok(()) => (),
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(_) => return Err("GOGOKE_SHORTCUT_FOLDER_CREATE_FAILED".to_string()),
        }
    }
    let _slot_pins = pin_directory_chain(&directory)?;
    let path = directory.join(SHORTCUT_NAME);
    if path.exists() {
        let mut existing = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
            .map_err(|_| "GOGOKE_SHORTCUT_EXISTING_OPEN_FAILED".to_string())?;
        if existing
            .metadata()
            .map_err(|_| "GOGOKE_SHORTCUT_EXISTING_UNAVAILABLE")?
            .file_attributes()
            & REPARSE_POINT
            != 0
            || !matches_opened_path(existing.as_raw_handle() as HANDLE, &path)?
        {
            return Err("GOGOKE_SHORTCUT_EXISTING_UNSAFE".to_string());
        }
        if let Some((_, old)) = previous {
            let current = shortcut_record_for(&mut existing, root, slot, &folder, &instance)?;
            if old.schema == SHORTCUT_SCHEMA
                && old.root == current.root
                && old.slot == slot
                && old.folder == folder
                && old.sha256 == current.sha256
                && old.identity == current.identity
            {
                let raw = serde_json::to_string(&current)
                    .map_err(|_| "GOGOKE_SHORTCUT_RECORD_SERIALIZE_FAILED".to_string())?;
                registry
                    .set_value(value, &raw)
                    .map_err(|_| "GOGOKE_SHORTCUT_RECORD_WRITE_FAILED".to_string())?;
            }
        }
        return Ok(()); // Unknown or replaced link remains user-owned.
    }
    if !create {
        return Ok(());
    }
    let bytes = shortcut_bytes(&exe, root)?;
    let mut created = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0)
        .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_DELETE_ON_CLOSE)
        .open(&path)
        .map_err(|_| "GOGOKE_SHORTCUT_CREATE_FAILED".to_string())?;
    created
        .write_all(&bytes)
        .and_then(|_| created.sync_all())
        .map_err(|_| "GOGOKE_SHORTCUT_WRITE_FAILED".to_string())?;
    let created_slot = final_handle_path(
        _slot_pins
            .last()
            .ok_or("GOGOKE_SHORTCUT_DIRECTORY_UNAVAILABLE")?
            .as_raw_handle() as HANDLE,
    )?
    .join(SHORTCUT_NAME);
    if final_handle_path(created.as_raw_handle() as HANDLE)? != created_slot {
        return Err("GOGOKE_SHORTCUT_CREATED_OBJECT_CHANGED".to_string());
    }
    let current = shortcut_record_for(&mut created, root, slot, &folder, &instance)?;
    if current.sha256 != format!("{:x}", Sha256::digest(&bytes)) {
        return Err("GOGOKE_SHORTCUT_WRITTEN_BYTES_CHANGED".to_string());
    }
    let raw = serde_json::to_string(&current)
        .map_err(|_| "GOGOKE_SHORTCUT_RECORD_SERIALIZE_FAILED".to_string())?;
    registry
        .set_value(value, &raw)
        .map_err(|_| "GOGOKE_SHORTCUT_RECORD_WRITE_FAILED".to_string())?;
    let readback: String = registry
        .get_value(value)
        .map_err(|_| "GOGOKE_SHORTCUT_RECORD_READBACK_FAILED".to_string())?;
    if readback != raw {
        return Err("GOGOKE_SHORTCUT_RECORD_CHANGED".to_string());
    }
    // The new link is delete-on-close until its instance-bound record is
    // durable. Clearing the on-close disposition is the final commit step.
    // A process termination before this call cannot publish an unowned link.
    let disposition = FILE_DISPOSITION_INFO_EX {
        Flags: FILE_DISPOSITION_FLAG_ON_CLOSE,
    };
    if unsafe {
        SetFileInformationByHandle(
            created.as_raw_handle() as HANDLE,
            FileDispositionInfoEx,
            (&disposition as *const FILE_DISPOSITION_INFO_EX).cast(),
            std::mem::size_of::<FILE_DISPOSITION_INFO_EX>() as u32,
        )
    } == 0
    {
        return Err("GOGOKE_SHORTCUT_COMMIT_FAILED".to_string());
    }
    drop(created);
    let mut committed = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|_| "GOGOKE_SHORTCUT_COMMIT_NOT_VISIBLE".to_string())?;
    let visible = shortcut_record_for(&mut committed, root, slot, &folder, &instance)?;
    if visible.identity != current.identity || visible.sha256 != current.sha256 {
        return Err("GOGOKE_SHORTCUT_COMMIT_CHANGED".to_string());
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateOwnedInventory {
    schema: &'static str,
    root: String,
    root_identity: OpenedObjectIdentity,
    instance: String,
    version: String,
    files: Vec<OwnedFile>,
}

/// Called by the coordinator while it owns the install lifecycle lock and the
/// old registration still names this root. The installed shell authenticates
/// the old index with its own compiled Owner key before naming any old bytes.
pub(crate) fn write_update_owned_inventory() -> Result<(), String> {
    let verified = resource_trust::verify_bootstrap()?;
    if verified.domain != resource_trust::Domain::Formal {
        return Err("GOGOKE_UPDATE_INVENTORY_FORMAL_ONLY".to_string());
    }
    let root = &verified.install_root;
    let instance = verified.registered_instance()?;
    let root_identity = opened_root_identity(root)?;
    // NSIS consumes and deletes its temporary install receipt before the
    // installation is usable. Only the verified signed inventory persists.
    let files = verified
        .owned_files_for_uninstall()?
        .into_iter()
        .map(|(path, sha256)| owned_file_with_identity(path, sha256))
        .collect::<Result<Vec<_>, String>>()?;
    if opened_root_identity(root)? != root_identity {
        return Err("GOGOKE_UPDATE_INVENTORY_ROOT_CHANGED".to_string());
    }
    let inventory = UpdateOwnedInventory {
        schema: "gogoke.update-owned-inventory.v1",
        root: path_text(root)?,
        root_identity,
        instance,
        version: verified.version,
        files,
    };
    let payload = serde_json::to_string(&inventory)
        .map_err(|_| "GOGOKE_UPDATE_INVENTORY_SERIALIZE".to_string())?;
    if payload.len() > 16 * 1024 * 1024 {
        return Err("GOGOKE_UPDATE_INVENTORY_TOO_LARGE".to_string());
    }
    println!("{payload}");
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizerInput {
    root: String,
    root_identity: OpenedObjectIdentity,
    lock_path: String,
    registry_key: String,
    instance: String,
    domain: String,
    parent_pid: u32,
    nonce: String,
    receipt: String,
    files: Vec<OwnedFile>,
    shortcuts: Vec<ShortcutSnapshot>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutSnapshot {
    registry_value: &'static str,
    raw: String,
    record: OwnedShortcut,
}

fn shortcut_snapshots(
    key_name: &str,
    root: &Path,
    instance: &str,
    domain: resource_trust::Domain,
) -> Result<Vec<ShortcutSnapshot>, String> {
    if domain != resource_trust::Domain::Formal {
        return Ok(Vec::new());
    }
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(
            format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key_name}"),
            KEY_READ,
        )
        .map_err(|_| "GOGOKE_SHORTCUT_REGISTRY_OPEN_FAILED".to_string())?;
    let mut snapshots = Vec::new();
    for value in [SHORTCUT_DESKTOP_VALUE, SHORTCUT_START_VALUE] {
        if let Some((raw, record)) = recorded_shortcut(&key, value)? {
            if record.instance != instance {
                continue;
            }
            if record.schema != SHORTCUT_SCHEMA
                || record.root != path_text(root)?
                || !matches!(
                    (value, record.slot.as_str()),
                    (SHORTCUT_DESKTOP_VALUE, "desktop") | (SHORTCUT_START_VALUE, "start")
                )
            {
                return Err("GOGOKE_SHORTCUT_RECORD_INVALID".to_string());
            }
            snapshots.push(ShortcutSnapshot {
                registry_value: value,
                raw,
                record,
            });
        }
    }
    Ok(snapshots)
}

fn no_reparse_ancestors(path: &Path) -> Result<(), String> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if !current.is_absolute() {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| "GOGOKE_UNINSTALL_PATH_UNAVAILABLE".to_string())?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_UNINSTALL_REPARSE_POINT".to_string());
        }
    }
    Ok(())
}

fn path_text(path: &Path) -> Result<String, String> {
    // Signed index paths use '/', but the embedded finalizer accepts only
    // drive-absolute Windows paths with backslash separators.
    path.to_str()
        .map(|value| value.replace('/', "\\"))
        .ok_or_else(|| "GOGOKE_UNINSTALL_PATH_ENCODING".to_string())
}

#[cfg(test)]
mod handoff_path_tests {
    use super::*;

    #[test]
    fn signed_relative_paths_serialize_with_windows_separators() {
        let root = Path::new(r"C:\gogoke-test");
        for (relative, expected) in [
            (
                "gogoke-service/runtime/node.exe",
                r"C:\gogoke-test\gogoke-service\runtime\node.exe",
            ),
            (
                "gogoke-service/generations/set/frontend/icon one.svg",
                r"C:\gogoke-test\gogoke-service\generations\set\frontend\icon one.svg",
            ),
        ] {
            let joined = root.join(relative);
            assert!(
                joined.to_str().unwrap().contains('/'),
                "the test must use the actual mixed path shape"
            );
            assert_eq!(path_text(&joined).unwrap(), expected);
        }
    }
}

fn opened_identity(file: &File) -> Result<OpenedObjectIdentity, String> {
    let mut info = FILE_ID_INFO::default();
    let result = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle() as HANDLE,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if result == 0 {
        return Err("GOGOKE_UNINSTALL_FILE_ID_UNAVAILABLE".to_string());
    }
    if info.FileId.Identifier == [0; 16] {
        return Err("GOGOKE_UNINSTALL_FILE_ID_UNAVAILABLE".to_string());
    }
    let file_id = info
        .FileId
        .Identifier
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .concat();
    Ok(OpenedObjectIdentity {
        volume_serial_number: info.VolumeSerialNumber.to_string(),
        file_id,
    })
}

fn opened_root_identity(root: &Path) -> Result<OpenedObjectIdentity, String> {
    no_reparse_ancestors(root)?;
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(root)
        .map_err(|_| "GOGOKE_UNINSTALL_ROOT_OPEN_FAILED".to_string())?;
    let metadata = opened
        .metadata()
        .map_err(|_| "GOGOKE_UNINSTALL_ROOT_UNAVAILABLE".to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_UNINSTALL_ROOT_UNSAFE".to_string());
    }
    opened_identity(&opened)
}

fn owned_file_with_identity(path: PathBuf, sha256: String) -> Result<OwnedFile, String> {
    no_reparse_ancestors(&path)?;
    let mut opened = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|_| "GOGOKE_UNINSTALL_OWNED_FILE_OPEN_FAILED".to_string())?;
    let metadata = opened
        .metadata()
        .map_err(|_| "GOGOKE_UNINSTALL_OWNED_FILE_UNAVAILABLE".to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_UNINSTALL_OWNED_FILE_UNSAFE".to_string());
    }
    let identity = opened_identity(&opened)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = opened
            .read(&mut buffer)
            .map_err(|_| "GOGOKE_UNINSTALL_OWNED_FILE_READ_FAILED".to_string())?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    if format!("{:x}", hasher.finalize()) != sha256 {
        return Err("GOGOKE_UNINSTALL_OWNED_FILE_CHANGED".to_string());
    }
    Ok(OwnedFile {
        path: path_text(&path)?,
        sha256,
        identity,
    })
}

fn encoded_bootstrap() -> String {
    let utf16: Vec<u8> = FINALIZER_BOOTSTRAP
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

fn inherited_lock_stdio(lock: &File) -> Result<Stdio, String> {
    let process = unsafe { GetCurrentProcess() };
    let mut duplicate: HANDLE = std::ptr::null_mut();
    let result = unsafe {
        DuplicateHandle(
            process,
            lock.as_raw_handle() as HANDLE,
            process,
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if result == 0 || duplicate.is_null() {
        return Err("GOGOKE_UNINSTALL_LOCK_HANDOFF_FAILED".to_string());
    }
    // Command passes this inheritable duplicate as child stderr. The finalizer
    // checks the inherited handle's actual path before acknowledging custody.
    Ok(unsafe { Stdio::from_raw_handle(duplicate as RawHandle) })
}

pub(crate) fn run() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE".to_string())?;
    let root = exe.parent().ok_or("GOGOKE_INSTALL_ROOT_UNAVAILABLE")?;
    let parent = root.parent().ok_or("GOGOKE_INSTALL_PARENT_UNAVAILABLE")?;
    no_reparse_ancestors(parent)?;
    no_reparse_ancestors(root)?;
    no_reparse_ancestors(&exe)?;

    // This is the same sibling lock path and share mode used by installation
    // and resource update. It remains open through the child custody handshake.
    let lock_path = parent.join("gogoke-install-lifecycle.lock");
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&lock_path)
        .map_err(|_| "GOGOKE_INSTALL_LIFECYCLE_BUSY".to_string())?;
    let lock_metadata = lock
        .metadata()
        .map_err(|_| "GOGOKE_UNINSTALL_LOCK_UNAVAILABLE".to_string())?;
    if !lock_metadata.is_file() || lock_metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_UNINSTALL_LOCK_UNSAFE".to_string());
    }
    lock.set_len(0)
        .and_then(|_| lock.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|_| "GOGOKE_UNINSTALL_LOCK_WITNESS_FAILED".to_string())?;

    let verified = resource_trust::verify_bootstrap()?;
    if verified
        .install_root
        .canonicalize()
        .map_err(|_| "GOGOKE_UNINSTALL_ROOT_CHANGED")?
        != root
            .canonicalize()
            .map_err(|_| "GOGOKE_UNINSTALL_ROOT_CHANGED")?
    {
        return Err("GOGOKE_UNINSTALL_ROOT_CHANGED".to_string());
    }
    let instance = verified.registered_instance()?;
    let owned = verified.owned_files_for_uninstall()?;
    if owned.is_empty() {
        return Err("GOGOKE_UNINSTALL_EMPTY_INVENTORY".to_string());
    }
    let root_identity = opened_root_identity(root)?;
    let shortcuts = shortcut_snapshots(
        verified.uninstall_registry_key(),
        root,
        &instance,
        verified.domain,
    )?;

    let nonce = uuid::Uuid::new_v4().to_string();
    let instance_tag = format!("{:x}", Sha256::digest(instance.as_bytes()));
    let receipt = parent.join(format!(
        "gogoke-uninstall-{}-{}.json",
        &instance_tag[..16],
        nonce
    ));
    let input = FinalizerInput {
        root: path_text(root)?,
        root_identity,
        lock_path: path_text(&lock_path)?,
        registry_key: verified.uninstall_registry_key().to_owned(),
        instance,
        domain: match verified.domain {
            resource_trust::Domain::Candidate => "CI_CANDIDATE_RESOURCE",
            resource_trust::Domain::Formal => "OWNER_RELEASE",
        }
        .to_owned(),
        parent_pid: std::process::id(),
        nonce: nonce.clone(),
        receipt: path_text(&receipt)?,
        files: owned
            .into_iter()
            .map(|(path, sha256)| owned_file_with_identity(path, sha256))
            .collect::<Result<Vec<_>, String>>()?,
        shortcuts,
    };
    let final_root_identity = opened_root_identity(root)?;
    if final_root_identity.file_id != input.root_identity.file_id
        || final_root_identity.volume_serial_number != input.root_identity.volume_serial_number
    {
        return Err("GOGOKE_UNINSTALL_ROOT_CHANGED".to_string());
    }
    let payload = serde_json::to_string(&input)
        .map_err(|_| "GOGOKE_UNINSTALL_HANDOFF_SERIALIZE".to_string())?;
    let mut system_directory = [0u16; 32768];
    let system_length = unsafe {
        GetSystemDirectoryW(system_directory.as_mut_ptr(), system_directory.len() as u32)
    } as usize;
    if system_length == 0 || system_length >= system_directory.len() {
        return Err("GOGOKE_UNINSTALL_SYSTEM_DIRECTORY".to_string());
    }
    let powershell = PathBuf::from(OsString::from_wide(&system_directory[..system_length]))
        .join("WindowsPowerShell/v1.0/powershell.exe");
    no_reparse_ancestors(&powershell)?;
    let encoded = encoded_bootstrap();
    let mut child = Command::new(powershell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
        ])
        .arg(&encoded)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(inherited_lock_stdio(&lock)?)
        .spawn()
        .map_err(|_| "GOGOKE_UNINSTALL_FINALIZER_START_FAILED".to_string())?;

    let mut stdin = child.stdin.take().ok_or("GOGOKE_UNINSTALL_HANDOFF_STDIN")?;
    let embedded = base64::engine::general_purpose::STANDARD.encode(FINALIZER.as_bytes());
    let encoded_payload = base64::engine::general_purpose::STANDARD.encode(payload.as_bytes());
    stdin
        .write_all(embedded.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.write_all(encoded_payload.as_bytes()))
        .and_then(|_| stdin.write_all(b"\n"))
        .map_err(|_| "GOGOKE_UNINSTALL_HANDOFF_WRITE_FAILED".to_string())?;
    drop(stdin);
    let stdout = child
        .stdout
        .take()
        .ok_or("GOGOKE_UNINSTALL_HANDOFF_STDOUT")?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout).read_line(&mut line);
        let _ = sender.send(result.map(|_| line));
    });
    match receiver.recv_timeout(HANDSHAKE_TIMEOUT) {
        Ok(Ok(line)) if line.trim_end() == format!("READY:{nonce}") => {
            let expected = format!("LOCK:{nonce}\n");
            let mut witness = vec![0u8; expected.len()];
            let confirmed = lock.seek(SeekFrom::Start(0)).is_ok()
                && lock.read_exact(&mut witness).is_ok()
                && witness == expected.as_bytes();
            if confirmed {
                // The child has written through the inherited share-mode-zero
                // handle and now holds it while it waits for this process.
                Ok(())
            } else {
                let _ = child.kill();
                let _ = child.wait();
                Err("GOGOKE_UNINSTALL_LOCK_HANDOFF_UNCONFIRMED".to_string())
            }
        }
        _ => {
            let _ = child.kill();
            let _ = child.wait();
            Err("GOGOKE_UNINSTALL_LOCK_HANDOFF_UNCONFIRMED".to_string())
        }
    }
}
