//! Installed-shell uninstall entry. The resource authority supplies every owned byte.
//! The fixed PowerShell finalizer is embedded in this executable, never read from a resource set.

use crate::resource_trust;
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    FileIdInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

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

#[derive(Eq, PartialEq, Serialize)]
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
    let files = verified.owned_files_for_uninstall()?
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
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| "GOGOKE_UNINSTALL_PATH_ENCODING".to_string())
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
        GetSystemDirectoryW(
            system_directory.as_mut_ptr(),
            system_directory.len() as u32,
        )
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
