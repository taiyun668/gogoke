//! Tauri-owned native-host lifecycle for the Design 37 desktop path.
//!
//! The existing R2 service still owns its short-lived native host.  This module
//! is the long-lived owner used when the service is switched to `connectExisting`.
//! Keeping the owner here makes the process relationship explicit: Tauri starts
//! the exact host, keeps its process handle, and never accepts a host endpoint
//! from Node or another caller.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const MAX_HANDSHAKE_LINE: usize = 16 * 1024;

#[derive(Debug)]
pub(crate) struct Design37Host {
    child: Mutex<Child>,
    service_pipe: String,
    user_pipe: String,
    capability: String,
    root: PathBuf,
    host_pid: u32,
    user_connection: Mutex<Option<std::fs::File>>,
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
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            // The host remains long-lived. Drain stderr so a diagnostic cannot
            // block it; the process exit path still reports the exit status.
            let stderr_tail_writer = Arc::clone(&stderr_tail);
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
                        if let Ok(mut shared) = stderr_tail_writer.lock() {
                            *shared = tail.clone();
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
        let handshake = (|| -> Result<(String, String, String), String> {
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
            validate_service_pipe(&pipe_path)?;
            validate_user_pipe(&user_pipe)?;
            if pipe_path == user_pipe {
                return Err("GOGOKE_DESIGN37_PIPE_CHANNELS_NOT_DISTINCT".to_string());
            }
            Ok((pipe_path, capability, user_pipe))
        })()
        .map_err(|error| with_stderr_tail(error, &stderr_tail))?;
        let (service_pipe, capability, user_pipe) = handshake;
        let user_connection = connect_and_verify_user_pipe(&user_pipe, &child)
            .map_err(|error| with_stderr_tail(error, &stderr_tail))?;
        Ok(Self {
            child: Mutex::new(child),
            service_pipe,
            user_pipe,
            capability,
            root: root.to_path_buf(),
            host_pid,
            user_connection: Mutex::new(Some(user_connection)),
        })
    }

    #[cfg(not(target_os = "windows"))]
    pub(crate) fn spawn(_binary: &Path, _root: &Path) -> Result<Self, String> {
        Err("GOGOKE_PRODUCT_WINDOWS_OWNER_PATH_ONLY".to_string())
    }

    pub(crate) fn user_pipe(&self) -> &str {
        &self.user_pipe
    }
    pub(crate) fn service_pipe(&self) -> &str {
        &self.service_pipe
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

    /// Send one User frame over the retained, object-verified connection.
    /// The peer process object is checked immediately before every request.
    #[cfg(target_os = "windows")]
    pub(crate) fn request_user(&self, frame: &[u8]) -> Result<Vec<u8>, String> {
        if frame.len() > 4 * 1024 * 1024 {
            return Err("GOGOKE_DESIGN37_USER_FRAME_TOO_LARGE".to_string());
        }
        let mut child = self
            .child
            .lock()
            .map_err(|_| "GOGOKE_DESIGN37_HOST_LOCK_POISONED".to_string())?;
        verify_user_server_object(&mut child, &self.user_connection)?;
        let mut connection = self
            .user_connection
            .lock()
            .map_err(|_| "GOGOKE_DESIGN37_USER_PIPE_LOCK_POISONED".to_string())?;
        let pipe = connection
            .as_mut()
            .ok_or_else(|| "GOGOKE_DESIGN37_USER_PIPE_CLOSED".to_string())?;
        pipe.write_all(&(frame.len() as u32).to_le_bytes())
            .map_err(io_error)?;
        pipe.write_all(frame).map_err(io_error)?;
        pipe.flush().map_err(io_error)?;
        let mut header = [0u8; 4];
        pipe.read_exact(&mut header).map_err(io_error)?;
        let length = u32::from_le_bytes(header) as usize;
        if length > 4 * 1024 * 1024 {
            return Err("GOGOKE_DESIGN37_USER_REPLY_TOO_LARGE".to_string());
        }
        let mut reply = vec![0u8; length];
        pipe.read_exact(&mut reply).map_err(io_error)?;
        Ok(reply)
    }
}

impl Drop for Design37Host {
    fn drop(&mut self) {
        if let Ok(connection) = self.user_connection.get_mut() {
            let _ = connection.take();
        }
        if let Ok(child) = self.child.get_mut() {
            for _ in 0..50 {
                if child.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
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
fn is_hex_64(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(target_os = "windows")]
fn validate_service_pipe(value: &str) -> Result<(), String> {
    validate_pipe(
        value,
        r"\\.\pipe\gogoke.current-user.v1.",
        "GOGOKE_DESIGN37_SERVICE_PIPE_INVALID",
    )
}

#[cfg(target_os = "windows")]
fn validate_user_pipe(value: &str) -> Result<(), String> {
    validate_pipe(
        value,
        r"\\.\pipe\gogoke.user.v1.",
        "GOGOKE_DESIGN37_USER_PIPE_INVALID",
    )
}

#[cfg(target_os = "windows")]
fn validate_pipe(value: &str, prefix: &str, error: &str) -> Result<(), String> {
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
fn with_stderr_tail(error: String, tail: &Arc<Mutex<Vec<u8>>>) -> String {
    let bytes = tail
        .lock()
        .ok()
        .map(|value| value.clone())
        .unwrap_or_default();
    if bytes.is_empty() {
        return error;
    }
    format!("{error}:STDERR_TAIL:{}", String::from_utf8_lossy(&bytes))
}

#[cfg(target_os = "windows")]
fn io_error(error: std::io::Error) -> String {
    format!(
        "GOGOKE_DESIGN37_USER_PIPE_IO:WIN32_{}",
        error.raw_os_error().unwrap_or(0)
    )
}

#[cfg(target_os = "windows")]
fn connect_and_verify_user_pipe(path: &str, child: &Child) -> Result<std::fs::File, String> {
    use std::fs::OpenOptions;
    let pipe = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    verify_pipe_server_object(child, &pipe)?;
    let mut preface = [0x47u8];
    pipe.try_clone()
        .map_err(io_error)?
        .write_all(&mut preface)
        .map_err(io_error)?;
    Ok(pipe)
}

#[cfg(target_os = "windows")]
fn verify_user_server_object(
    child: &mut Child,
    pipe: &Mutex<Option<std::fs::File>>,
) -> Result<(), String> {
    let connection = pipe
        .lock()
        .map_err(|_| "GOGOKE_DESIGN37_USER_PIPE_LOCK_POISONED".to_string())?;
    let connection = connection
        .as_ref()
        .ok_or_else(|| "GOGOKE_DESIGN37_USER_PIPE_CLOSED".to_string())?;
    verify_pipe_server_object(child, connection)
}

#[cfg(target_os = "windows")]
fn verify_pipe_server_object(child: &Child, pipe: &std::fs::File) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let mut server_pid = 0u32;
    let pipe_handle = pipe.as_raw_handle() as HANDLE;
    if unsafe { GetNamedPipeServerProcessId(pipe_handle, &mut server_pid) } == 0 || server_pid == 0
    {
        return Err(format!(
            "GOGOKE_DESIGN37_USER_SERVER_PID_FAILED:WIN32_{}",
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        ));
    }
    let server = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
            0,
            server_pid,
        )
    };
    if server.is_null() {
        return Err(format!(
            "GOGOKE_DESIGN37_USER_SERVER_OPEN_FAILED:WIN32_{}",
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        ));
    }
    let same = unsafe { CompareObjectHandles(child.as_raw_handle() as HANDLE, server) } != 0;
    unsafe {
        CloseHandle(server);
    }
    if !same {
        return Err("GOGOKE_DESIGN37_USER_SERVER_OBJECT_MISMATCH".to_string());
    }
    Ok(())
}

#[cfg(target_os = "windows")]
use std::os::windows::io::AsRawHandle;

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetNamedPipeServerProcessId(
        pipe: windows_sys::Win32::Foundation::HANDLE,
        pid: *mut u32,
    ) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> windows_sys::Win32::Foundation::HANDLE;
    fn CloseHandle(handle: windows_sys::Win32::Foundation::HANDLE) -> i32;
}

#[cfg(target_os = "windows")]
#[link(name = "KernelBase")]
unsafe extern "system" {
    fn CompareObjectHandles(
        first: windows_sys::Win32::Foundation::HANDLE,
        second: windows_sys::Win32::Foundation::HANDLE,
    ) -> i32;
}
