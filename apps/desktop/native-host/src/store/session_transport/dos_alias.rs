//! Private Windows DOS-device aliases used only by the pinned OpenCode H path.
//!
//! A DOS alias is a protocol compatibility spelling, never an authorization
//! root. Callers must retain and re-check the original physical F identity.

use crate::root::RootIdentity;
use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

const DDD_REMOVE_DEFINITION: u32 = 0x0000_0002;
const DDD_EXACT_MATCH_ON_REMOVE: u32 = 0x0000_0004;
const DDD_NO_BROADCAST_SYSTEM: u32 = 0x0000_0008;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_STATISTICS: u32 = 10;
const TOKEN_SESSION_ID: u32 = 12;
const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_NO_TOKEN: i32 = 1008;

type Handle = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Luid {
    low_part: u32,
    high_part: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TokenStatistics {
    token_id: Luid,
    authentication_id: Luid,
    expiration_time: i64,
    token_type: u32,
    impersonation_level: u32,
    dynamic_charged: u32,
    dynamic_available: u32,
    group_count: u32,
    privilege_count: u32,
    modified_id: Luid,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct FileTime {
    low: u32,
    high: u32,
}

impl FileTime {
    fn value(self) -> u64 {
        (u64::from(self.high) << 32) | u64::from(self.low)
    }
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn DefineDosDeviceW(flags: u32, device_name: *const u16, target_path: *const u16) -> i32;
    fn QueryDosDeviceW(device_name: *const u16, target_path: *mut u16, max: u32) -> u32;
    fn GetCurrentProcess() -> Handle;
    fn GetCurrentThread() -> Handle;
    fn GetCurrentProcessId() -> u32;
    fn GetProcessTimes(
        process: Handle,
        creation: *mut FileTime,
        exit: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}

#[link(name = "advapi32")]
unsafe extern "system" {
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn OpenThreadToken(thread: Handle, access: u32, open_as_self: i32, token: *mut Handle) -> i32;
    fn GetTokenInformation(
        token: Handle,
        class: u32,
        information: *mut c_void,
        information_len: u32,
        returned_len: *mut u32,
    ) -> i32;
}

struct OwnedHandle(Handle);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn last_error(context: &'static str) -> String {
    format!("OpenCode DOS alias {context}: {:?}", io::Error::last_os_error())
}

/// Authenticated logon LUID plus Windows session. A PID by itself is not a
/// namespace identity and cannot authorize removal in another logon.
pub(super) fn current_namespace_id() -> Result<String, String> {
    let mut raw = std::ptr::null_mut();
    if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut raw) } == 0 {
        let error=io::Error::last_os_error();
        if error.raw_os_error()!=Some(ERROR_NO_TOKEN) {
            return Err(format!("OpenCode DOS alias open effective thread token: {error:?}"));
        }
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
            return Err(last_error("open current process token"));
        }
    }
    if raw.is_null() {
        return Err("OpenCode DOS alias: token handle is null".into());
    }
    let token = OwnedHandle(raw);
    let mut statistics = TokenStatistics::default();
    let mut returned = 0u32;
    if unsafe {
        GetTokenInformation(
            token.0,
            TOKEN_STATISTICS,
            (&mut statistics as *mut TokenStatistics).cast(),
            std::mem::size_of::<TokenStatistics>() as u32,
            &mut returned,
        )
    } == 0
        || returned != std::mem::size_of::<TokenStatistics>() as u32
    {
        return Err(last_error("read token statistics"));
    }
    let mut session_id = 0u32;
    returned = 0;
    if unsafe {
        GetTokenInformation(
            token.0,
            TOKEN_SESSION_ID,
            (&mut session_id as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
            &mut returned,
        )
    } == 0
        || returned != std::mem::size_of::<u32>() as u32
    {
        return Err(last_error("read token session"));
    }
    if statistics.authentication_id.low_part == 0 && statistics.authentication_id.high_part == 0 {
        return Err("OpenCode DOS alias: zero authentication LUID".into());
    }
    Ok(format!(
        "auth={:08x}{:08x};session={session_id}",
        statistics.authentication_id.high_part as u32,
        statistics.authentication_id.low_part
    ))
}

/// Exact identity of the host process that will own the local DOS mapping.
pub(super) fn current_host_identity() -> Result<(u32, u64), String> {
    let pid = unsafe { GetCurrentProcessId() };
    if pid == 0 {
        return Err("OpenCode DOS alias: current host PID is zero".into());
    }
    let mut creation = FileTime::default();
    let mut exit = FileTime::default();
    let mut kernel = FileTime::default();
    let mut user = FileTime::default();
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(last_error("read current host creation time"));
    }
    let creation = creation.value();
    if creation == 0 {
        return Err("OpenCode DOS alias: current host creation time is zero".into());
    }
    Ok((pid, creation))
}

/// DOS-device mappings are a finite namespace. No mapping is overwritten and
/// only a single exact mapping is ever accepted for an owned alias.
pub(super) fn query(name: &str) -> Result<Vec<String>, String> {
    if !valid_name(name) {
        return Err("OpenCode DOS alias: invalid device name".into());
    }
    let name_wide = wide(OsStr::new(name));
    let mut buffer = vec![0u16; 32_768];
    let written = unsafe {
        QueryDosDeviceW(name_wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32)
    };
    if written == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) {
            return Ok(Vec::new());
        }
        return Err(format!("OpenCode DOS alias QueryDosDeviceW: {error:?}"));
    }
    if written as usize > buffer.len() {
        return Err("OpenCode DOS alias: QueryDosDeviceW length exceeds buffer".into());
    }
    if buffer[written as usize-1]!=0 {
        return Err("OpenCode DOS alias: QueryDosDeviceW result is not terminated".into());
    }
    let mut values = Vec::new();
    let mut start = 0usize;
    for cursor in 0..written as usize {
        if buffer[cursor] == 0 {
            if cursor == start {
                break;
            }
            values.push(OsString::from_wide(&buffer[start..cursor])
                .into_string()
                .map_err(|_| "OpenCode DOS alias: non-Unicode QueryDosDeviceW result")?);
            start = cursor + 1;
        }
    }
    if values.is_empty() || start > written as usize {
        return Err("OpenCode DOS alias: malformed QueryDosDeviceW MULTI_SZ".into());
    }
    Ok(values)
}

fn valid_name(name: &str) -> bool {
    name.len() == 2
        && name.as_bytes()[0].is_ascii_uppercase()
        && name.as_bytes()[1] == b':'
}

pub(super) fn free_candidates() -> impl Iterator<Item = &'static str> {
    [
        "Z:", "Y:", "X:", "W:", "V:", "U:", "T:", "S:", "R:", "Q:",
        "P:", "O:", "N:", "M:", "L:", "K:", "J:", "I:", "H:", "G:",
        "F:", "E:", "D:",
    ]
    .into_iter()
}

/// Define only after a durable intent exists. The returned target is the exact
/// DOS target spelling supplied to DefineDosDeviceW and later used for exact
/// removal; the observed mapping is captured separately by `verify_mapping`.
pub(super) fn define(name: &str, physical_f: &Path) -> Result<String, String> {
    if !valid_name(name) || !physical_f.is_absolute() {
        return Err("OpenCode DOS alias: invalid definition input".into());
    }
    let target = physical_f
        .to_str()
        .filter(|path| !path.is_empty() && !path.contains('\0'))
        .ok_or("OpenCode DOS alias: non-Unicode F target")?;
    if !query(name)?.is_empty() {
        return Err("OpenCode DOS alias: refused to overwrite existing mapping".into());
    }
    let name_wide = wide(OsStr::new(name));
    let target_wide = wide(OsStr::new(target));
    if unsafe {
        DefineDosDeviceW(
            DDD_NO_BROADCAST_SYSTEM,
            name_wide.as_ptr(),
            target_wide.as_ptr(),
        )
    } == 0
    {
        return Err(last_error("DefineDosDeviceW"));
    }
    Ok(target.to_owned())
}

/// Require exactly one mapping and prove its root is the original physical F.
pub(super) fn verify_mapping(
    name: &str,
    observed_target: Option<&str>,
    expected_f: &RootIdentity,
) -> Result<String, String> {
    let mappings = query(name)?;
    if mappings.len() != 1 {
        return Err("OpenCode DOS alias: mapping is absent or ambiguous".into());
    }
    if observed_target.is_some_and(|expected| mappings[0] != expected) {
        return Err("OpenCode DOS alias: observed target changed".into());
    }
    let alias_root = PathBuf::from(format!(r"{}\", name));
    let resolved = std::fs::canonicalize(&alias_root)
        .map_err(|error| format!("OpenCode DOS alias canonicalize: {error:?}"))?;
    let identity = crate::root::inspect_root(&resolved)
        .map_err(|error| format!("OpenCode DOS alias physical identity: {error:?}"))?
        .identity;
    if &identity != expected_f {
        return Err("OpenCode DOS alias: physical F FileID changed".into());
    }
    Ok(mappings[0].clone())
}

/// Delete only the exact mapping that H originally created. Caller separately
/// persists the returned removal/readback result.
pub(super) fn remove_exact(name: &str, requested_target: &str) -> Result<(), String> {
    if !valid_name(name) || requested_target.is_empty() || requested_target.contains('\0') {
        return Err("OpenCode DOS alias: invalid exact removal input".into());
    }
    let name_wide = wide(OsStr::new(name));
    let target_wide = wide(OsStr::new(requested_target));
    if unsafe {
        DefineDosDeviceW(
            DDD_REMOVE_DEFINITION | DDD_EXACT_MATCH_ON_REMOVE | DDD_NO_BROADCAST_SYSTEM,
            name_wide.as_ptr(),
            target_wide.as_ptr(),
        )
    } == 0
    {
        return Err(last_error("exact DefineDosDeviceW removal"));
    }
    if !query(name)?.is_empty() {
        return Err("OpenCode DOS alias: mapping remains after exact removal".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_single_letter_dos_names() {
        assert!(valid_name("Z:"));
        for invalid in ["", "Z", "ZZ:", "z:", "C:\\", "\\\\.\\Z:"] {
            assert!(!valid_name(invalid), "accepted {invalid:?}");
        }
    }

    #[test]
    fn candidate_space_is_finite_and_never_uses_system_or_boot_letters() {
        let candidates = free_candidates().collect::<Vec<_>>();
        assert_eq!(candidates.len(), 23);
        assert_eq!(candidates[0], "Z:");
        assert_eq!(candidates[22], "D:");
        assert!(!candidates.contains(&"C:"));
        assert_eq!(candidates.iter().collect::<std::collections::HashSet<_>>().len(), 23);
    }

    #[test]
    fn exact_mapping_readback_requires_the_original_physical_f_identity() {
        use crate::root::RootLock;
        use std::time::{SystemTime,UNIX_EPOCH};
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!(
            "gogoke-opencode-alias-boundary-{}-{stamp}",std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let identity=root.canonical_root().identity.clone();
        let name=free_candidates().find(|name|query(name).is_ok_and(|rows|rows.is_empty()))
            .expect("a free local test drive alias");
        let target=define(name,root.canonical_root().canonical_path.as_path()).unwrap();
        let cleanup=TestAlias{name,target:target.clone()};
        let observed=verify_mapping(name,None,&identity).unwrap();
        assert_eq!(query(name).unwrap(),vec![observed.clone()]);
        assert!(verify_mapping(name,Some("deliberately-different-target"),&identity).is_err());
        let foreign=crate::root::RootIdentity{volume_serial:identity.volume_serial,
            file_id:[0x5a;16]};
        assert!(verify_mapping(name,Some(&observed),&foreign).is_err());
        remove_exact(name,&target).unwrap();
        assert!(query(name).unwrap().is_empty());
        drop(cleanup);
        drop(root);
        std::fs::remove_dir_all(folder).unwrap();
    }

    struct TestAlias { name:&'static str, target:String }
    impl Drop for TestAlias {
        fn drop(&mut self) {let _=remove_exact(self.name,&self.target);}
    }
}
