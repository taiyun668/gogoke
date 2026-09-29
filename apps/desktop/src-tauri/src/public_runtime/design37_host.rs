//! Tauri-owned native-host lifecycle for the Design 37 desktop path.
//!
//! The existing R2 service still owns its short-lived native host.  This module
//! is the long-lived owner used when the service is switched to `connectExisting`.
//! Keeping the owner here makes the process relationship explicit: Tauri starts
//! the exact host, keeps its process handle, and never accepts a host endpoint
//! from Node or another caller.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const MAX_HANDSHAKE_LINE: usize = 16 * 1024;

#[derive(Debug)]
pub(crate) struct Design37Host {
    child: Mutex<Child>,
    user_pipe: String,
    capability: String,
    root: PathBuf,
    host_pid: u32,
}

impl Design37Host {
    /// Spawn the host owned by this Tauri process and consume its complete
    /// startup handshake.  The binary and root are supplied by the verified
    /// installed resource set; callers cannot provide either through IPC.
    #[cfg(target_os = "windows")]
    pub(crate) fn spawn(binary: &Path, root: &Path) -> Result<Self, String> {
        let user_pid = std::process::id();
        let root_text = root
            .to_str()
            .ok_or_else(|| "GOGOKE_DESIGN37_ROOT_NOT_UTF8".to_string())?;
        let mut child = Command::new(binary)
            .arg("--root")
            .arg(root_text)
            .arg("--desktop-session")
            .arg("--user-pid")
            .arg(user_pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .spawn()
            .map_err(|error| {
                format!(
                    "GOGOKE_DESIGN37_HOST_START_FAILED:WIN32_{}",
                    error.raw_os_error().unwrap_or(0)
                )
            })?;
        let host_pid = child.id();
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            // The host remains long-lived. Drain stderr so a diagnostic cannot
            // block it; the process exit path still reports the exit status.
            std::thread::Builder::new()
                .name("gogoke-design37-host-stderr".to_string())
                .spawn(move || {
                    let mut reader = BufReader::new(stderr);
                    let mut tail = Vec::with_capacity(4096);
                    let mut chunk = [0u8; 1024];
                    while let Ok(count) = reader.read(&mut chunk) {
                        if count == 0 {
                            break;
                        }
                        tail.extend_from_slice(&chunk[..count]);
                        if tail.len() > 4096 {
                            let drain = tail.len() - 4096;
                            tail.drain(..drain);
                        }
                    }
                })
                .map_err(|error| format!("GOGOKE_DESIGN37_HOST_STDERR_FAILED:{error}"))?;
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "GOGOKE_DESIGN37_HOST_STDOUT_MISSING".to_string())?;
        let mut lines = BufReader::new(stdout).lines();
        let locked = next_line(&mut lines, "LOCKED")?;
        validate_locked(&locked, root)?;
        let pipe = next_line(&mut lines, "PIPE")?;
        let pipe_path = field(&pipe, "PIPE")?;
        let capability_line = next_line(&mut lines, "CAPABILITY")?;
        let capability = field(&capability_line, "CAPABILITY")?;
        if !is_hex_64(&capability) {
            return Err("GOGOKE_DESIGN37_HOST_CAPABILITY_INVALID".to_string());
        }
        let user_pipe_line = next_line(&mut lines, "USER_PIPE")?;
        let user_pipe = field(&user_pipe_line, "USER_PIPE")?;
        validate_private_pipe(&pipe_path, "GOGOKE_DESIGN37_SERVICE_PIPE_INVALID")?;
        validate_private_pipe(&user_pipe, "GOGOKE_DESIGN37_USER_PIPE_INVALID")?;
        if pipe_path == user_pipe {
            return Err("GOGOKE_DESIGN37_PIPE_CHANNELS_NOT_DISTINCT".to_string());
        }
        Ok(Self {
            child: Mutex::new(child),
            user_pipe,
            capability,
            root: root.to_path_buf(),
            host_pid,
        })
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn spawn(_binary: &Path, _root: &Path) -> Result<Self, String> {
        Err("GOGOKE_PRODUCT_WINDOWS_OWNER_PATH_ONLY".to_string())
    }

    pub(crate) fn user_pipe(&self) -> &str {
        &self.user_pipe
    }
    pub(crate) fn capability(&self) -> &str {
        &self.capability
    }
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }
    pub(crate) fn host_pid(&self) -> u32 {
        self.host_pid
    }

    /// Check that the retained host is still alive before a caller attempts a
    /// user operation.  The retained Child handle prevents PID reuse while the
    /// owner is alive; the native side additionally proves the pipe peer.
    pub(crate) fn is_running(&self) -> Result<bool, String> {
        let mut child = self
            .child
            .lock()
            .map_err(|_| "GOGOKE_DESIGN37_HOST_LOCK_POISONED".to_string())?;
        child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(|error| {
                format!(
                    "GOGOKE_DESIGN37_HOST_STATUS_FAILED:WIN32_{}",
                    error.raw_os_error().unwrap_or(0)
                )
            })
    }
}

impl Drop for Design37Host {
    fn drop(&mut self) {
        if let Ok(child) = self.child.get_mut() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn next_line<R: BufRead>(lines: &mut std::io::Lines<R>, expected: &str) -> Result<String, String> {
    let line = lines
        .next()
        .ok_or_else(|| format!("GOGOKE_DESIGN37_HOST_{expected}_MISSING"))?
        .map_err(|error| {
            format!(
                "GOGOKE_DESIGN37_HOST_HANDSHAKE_READ_FAILED:WIN32_{}",
                error.raw_os_error().unwrap_or(0)
            )
        })?;
    if line.len() > MAX_HANDSHAKE_LINE
        || !line.starts_with(expected)
        || !line
            .as_bytes()
            .get(expected.len())
            .is_some_and(|byte| *byte == b'\t')
    {
        return Err(format!("GOGOKE_DESIGN37_HOST_{expected}_INVALID"));
    }
    Ok(line)
}

#[cfg(target_os = "windows")]
fn field(line: &str, expected: &str) -> Result<String, String> {
    let value = line
        .strip_prefix(&format!("{expected}\t"))
        .unwrap_or_default();
    if value.is_empty() || value.contains(['\r', '\n', '\0']) {
        return Err(format!("GOGOKE_DESIGN37_HOST_{expected}_INVALID"));
    }
    Ok(value.to_string())
}

#[cfg(target_os = "windows")]
fn validate_locked(line: &str, root: &Path) -> Result<(), String> {
    let body = field(line, "LOCKED")?;
    let requested = body.split('\t').next().unwrap_or_default();
    if requested != root.to_string_lossy() {
        return Err("GOGOKE_DESIGN37_HOST_ROOT_MISMATCH".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn validate_private_pipe(value: &str, error: &str) -> Result<(), String> {
    let prefix = r"\\.\pipe\gogoke.current-user.v1.";
    if !value.starts_with(prefix)
        || value.len() > prefix.len() + 120
        || !value[prefix.len()..]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn is_hex_64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
