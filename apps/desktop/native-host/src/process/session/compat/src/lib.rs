//! Fixed x64 Codex 0.160 LPAC path-API compatibility module.
//! The host owns launch authorization, verified image identity, Job custody,
//! exact home mapping, module file custody, and thread resume.

#![cfg(windows)]

use std::ffi::{c_void, CStr, CString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

pub const MODULE_BYTES: &[u8] = include_bytes!(env!("GOGOKE_LPAC_PATH_MODULE"));
pub const MODULE_SHA256: &str = env!("GOGOKE_LPAC_PATH_MODULE_SHA256");
pub const SHIM_SOURCE_SHA256: &str = env!("GOGOKE_LPAC_PATH_SHIM_SHA256");
pub const DETOURS_COMMIT: &str = "e4bfd6b03e50de46b47abfbd1e46b384f0c5f833";
pub const DETOURS_LICENSE: &str = "MIT";
pub const SUPPORTED_CLI_VERSION: &str = "0.160.0-win32-x64";
pub const OBSERVED_CLI_SHA256: &str =
    "fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d";
pub const OBSERVED_CLAUDE_SHA256: &str =
    "180d7b279455e8b89d4353a5146447be2f80b80fb0db14bdc6dd9cb98c0aef09";

const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const WC_NO_BEST_FIT_CHARS: u32 = 0x400;

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut c_void;
    fn GetACP() -> u32;
    fn IsWow64Process2(process: *mut c_void, process_machine: *mut u16,
        native_machine: *mut u16) -> i32;
    fn WideCharToMultiByte(code_page: u32, flags: u32, wide: *const u16,
        wide_len: i32, bytes: *mut i8, byte_len: i32, default_char: *const i8,
        used_default: *mut i32) -> i32;
}

#[link(name = "gogoke_detours", kind = "static")]
extern "system" {
    fn DetourUpdateProcessWithDll(process: *mut c_void, names: *const *const i8,
        count: u32) -> i32;
}

fn x64_process(process: *mut c_void) -> io::Result<bool> {
    let mut process_machine = 0;
    let mut native_machine = 0;
    if unsafe { IsWow64Process2(process, &mut process_machine, &mut native_machine) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(process_machine == 0 && native_machine == IMAGE_FILE_MACHINE_AMD64)
}

/// Convert an absolute module path for Detours' ANSI API without best-fit or
/// replacement characters. The host must keep the exact file open and verify
/// its embedded bytes, identity and LPAC read/execute grant through resume.
pub fn module_path_ansi(path: &Path) -> io::Result<CString> {
    path_for_code_page(path, unsafe { GetACP() })
}

fn path_for_code_page(path: &Path, code_page: u32) -> io::Result<CString> {
    if !path.is_absolute() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "module path is not absolute"));
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.is_empty() || wide.len() > i32::MAX as usize || wide.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid module path"));
    }
    let mut used_default = 0;
    // UTF-8 ACP forbids lpUsedDefaultChar and legacy conversion flags.
    // Reject malformed UTF-16 there; legacy ACP still rejects replacement.
    let utf8 = code_page == 65001;
    let flags = if utf8 { 0x80 } else { WC_NO_BEST_FIT_CHARS };
    let default_pointer = if utf8 { std::ptr::null_mut() } else { &mut used_default as *mut i32 };
    let needed = unsafe { WideCharToMultiByte(code_page, flags,
        wide.as_ptr(), wide.len() as i32, std::ptr::null_mut(), 0,
        std::ptr::null(), default_pointer) };
    if needed == 0 { return Err(io::Error::last_os_error()); }
    let mut bytes = vec![0u8; needed as usize];
    used_default = 0;
    let written = unsafe { WideCharToMultiByte(code_page, flags,
        wide.as_ptr(), wide.len() as i32, bytes.as_mut_ptr().cast(), needed,
        std::ptr::null(), default_pointer) };
    if written != needed || used_default != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "module path is not losslessly representable in the Windows ANSI code page"));
    }
    CString::new(bytes).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput,
        "module path contains NUL"))
}

/// Add the one fixed DLL to an already-created, host-verified suspended x64
/// process. Caller must verify the exact pinned Codex image, LPAC token, Job,
/// module file and mapping before this call. This never creates or resumes a
/// process and does not select a cross-bitness helper.
///
/// # Safety
/// `process` must be a live PROCESS_VM_* capable handle to the exact suspended
/// child owned by the caller. `module_path` must identify the retained fixed
/// module file, not an IPC-selected path.
pub unsafe fn update_suspended(process: *mut c_void, module_path: &CStr) -> io::Result<()> {
    if process.is_null() || !x64_process(GetCurrentProcess())? || !x64_process(process)? {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "Detours path compatibility requires same-bitness native x64 processes"));
    }
    let bytes = module_path.to_bytes();
    if !(bytes.len() >= 3 && bytes[1] == b':' && bytes[2] == b'\\'
        || bytes.starts_with(b"\\\\")) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput,
            "Detours module path must be absolute"));
    }
    let names = [module_path.as_ptr()];
    if DetourUpdateProcessWithDll(process, names.as_ptr(), 1) == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn module_path_conversion_handles_utf8_and_rejects_legacy_loss() {
        let path = std::path::Path::new("C:\\tmp\\兼容\\module.dll");
        let utf8 = super::path_for_code_page(path, 65001).unwrap();
        assert_eq!(utf8.to_bytes(), path.to_str().unwrap().as_bytes());
        assert!(super::path_for_code_page(path, 1252).is_err());
        assert!(super::path_for_code_page(std::path::Path::new("module.dll"), 65001).is_err());
    }
    #[test]
    fn production_shim_buffer_mapping_passthrough_and_iat_guards() {
        let status = std::process::Command::new(env!("GOGOKE_LPAC_PATH_TEST_EXE"))
            .status().expect("run cloud-built same-source C++ shim test");
        assert!(status.success(), "same-source C++ shim test failed: {status}");
    }
}
