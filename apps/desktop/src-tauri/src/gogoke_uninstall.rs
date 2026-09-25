//! Installed-shell uninstall entry. The resource authority supplies every owned byte.
//! The fixed PowerShell finalizer is embedded in this executable, never read from a resource set.

use crate::resource_trust;
use base64::Engine;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

const REPARSE_POINT: u32 = 0x400;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const FINALIZER: &str = include_str!("../update/gogoke-uninstall-finalizer.ps1");

#[derive(Serialize)]
struct OwnedFile {
    path: String,
    sha256: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FinalizerInput {
    root: String,
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
        if !current.is_absolute() { continue; }
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

fn encoded_finalizer() -> String {
    let utf16: Vec<u8> = FINALIZER.encode_utf16().flat_map(u16::to_le_bytes).collect();
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
    let mut lock = OpenOptions::new().read(true).write(true).create(true)
        .share_mode(0).custom_flags(OPEN_REPARSE_POINT)
        .open(&lock_path)
        .map_err(|_| "GOGOKE_INSTALL_LIFECYCLE_BUSY".to_string())?;
    let lock_metadata = lock.metadata()
        .map_err(|_| "GOGOKE_UNINSTALL_LOCK_UNAVAILABLE".to_string())?;
    if !lock_metadata.is_file() || lock_metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_UNINSTALL_LOCK_UNSAFE".to_string());
    }
    lock.set_len(0).and_then(|_| lock.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|_| "GOGOKE_UNINSTALL_LOCK_WITNESS_FAILED".to_string())?;

    let verified = resource_trust::verify_bootstrap()?;
    if verified.install_root.canonicalize().map_err(|_| "GOGOKE_UNINSTALL_ROOT_CHANGED")?
        != root.canonicalize().map_err(|_| "GOGOKE_UNINSTALL_ROOT_CHANGED")? {
        return Err("GOGOKE_UNINSTALL_ROOT_CHANGED".to_string());
    }
    let instance = verified.registered_instance()?;
    let owned = verified.owned_files_for_uninstall()?;
    if owned.is_empty() { return Err("GOGOKE_UNINSTALL_EMPTY_INVENTORY".to_string()); }
    for (path, _) in &owned { no_reparse_ancestors(path)?; }

    let nonce = uuid::Uuid::new_v4().to_string();
    let instance_tag = format!("{:x}", Sha256::digest(instance.as_bytes()));
    let receipt = parent.join(format!("gogoke-uninstall-{}-{}.json", &instance_tag[..16], nonce));
    let input = FinalizerInput {
        root: path_text(root)?,
        lock_path: path_text(&lock_path)?,
        registry_key: verified.uninstall_registry_key().to_owned(),
        instance,
        domain: match verified.domain {
            resource_trust::Domain::Candidate => "CI_CANDIDATE_RESOURCE",
            resource_trust::Domain::Formal => "OWNER_RELEASE",
        }.to_owned(),
        parent_pid: std::process::id(),
        nonce: nonce.clone(),
        receipt: path_text(&receipt)?,
        files: owned.into_iter().map(|(path, sha256)| {
            Ok(OwnedFile { path: path_text(&path)?, sha256 })
        }).collect::<Result<Vec<_>, String>>()?,
    };
    let payload = serde_json::to_string(&input)
        .map_err(|_| "GOGOKE_UNINSTALL_HANDOFF_SERIALIZE".to_string())?;
    let system_root = std::env::var_os("SystemRoot")
        .ok_or_else(|| "GOGOKE_UNINSTALL_SYSTEM_ROOT".to_string())?;
    let powershell = PathBuf::from(system_root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    no_reparse_ancestors(&powershell)?;
    let encoded = encoded_finalizer();
    let mut child = Command::new(powershell)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand"])
        .arg(&encoded)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(inherited_lock_stdio(&lock)?)
        .spawn()
        .map_err(|_| "GOGOKE_UNINSTALL_FINALIZER_START_FAILED".to_string())?;

    let mut stdin = child.stdin.take().ok_or("GOGOKE_UNINSTALL_HANDOFF_STDIN")?;
    stdin.write_all(payload.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .map_err(|_| "GOGOKE_UNINSTALL_HANDOFF_WRITE_FAILED".to_string())?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or("GOGOKE_UNINSTALL_HANDOFF_STDOUT")?;
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
                Err("GOGOKE_UNINSTALL_FINALIZER_DELETE_NOT_VERIFIED".to_string())
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
