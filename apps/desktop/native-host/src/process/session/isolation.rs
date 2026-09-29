//! Host-owned Windows AppContainer identity for an admitted v37 process.
//! The profile SID is a kernel access principal, not a permission grant by itself.

use std::ffi::c_void;
use std::fmt;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::ptr;

type Handle = *mut c_void;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_IS_APP_CONTAINER: u32 = 29;
const TOKEN_APP_CONTAINER_SID: u32 = 31;
const PROFILE_ALREADY_EXISTS: u32 = 0x8007_00b7;

#[link(name = "userenv")]
extern "system" {
    fn CreateAppContainerProfile(name: *const u16, display: *const u16,
        description: *const u16, capabilities: *const c_void, count: u32,
        sid: *mut *mut c_void) -> i32;
    fn DeriveAppContainerSidFromAppContainerName(name: *const u16,
        sid: *mut *mut c_void) -> i32;
}

#[link(name = "advapi32")]
extern "system" {
    fn FreeSid(sid: *mut c_void) -> *mut c_void;
    fn EqualSid(left: *mut c_void, right: *mut c_void) -> i32;
    fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
    fn GetTokenInformation(token: Handle, class: u32, output: *mut c_void,
        length: u32, returned: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: Handle) -> i32;
}

#[repr(C)]
pub(crate) struct SecurityCapabilities {
    pub(crate) app_container_sid: *mut c_void,
    pub(crate) capabilities: *mut c_void,
    pub(crate) capability_count: u32,
    pub(crate) reserved: u32,
}

#[derive(Debug)]
pub(crate) enum IsolationError {
    InvalidProfileName,
    ProfileHResult(i32),
    MissingSid,
    Token(io::Error),
    WrongToken,
}

impl fmt::Display for IsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfileName => write!(f, "invalid AppContainer profile name"),
            Self::ProfileHResult(hr) => write!(f, "AppContainer profile HRESULT=0x{:08x}", *hr as u32),
            Self::MissingSid => write!(f, "AppContainer profile returned no SID"),
            Self::Token(error) => write!(f, "AppContainer token query: {error}"),
            Self::WrongToken => write!(f, "suspended process has a different AppContainer token"),
        }
    }
}

pub(crate) struct AppContainerProfile {
    sid: *mut c_void,
}

impl AppContainerProfile {
    pub(crate) fn ensure(name: &str) -> Result<Self, IsolationError> {
        if !valid_profile_name(name) { return Err(IsolationError::InvalidProfileName); }
        let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        let hr = unsafe { CreateAppContainerProfile(wide.as_ptr(), wide.as_ptr(), wide.as_ptr(),
            ptr::null(), 0, &mut sid) };
        if hr as u32 == PROFILE_ALREADY_EXISTS {
            let derived = unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &mut sid) };
            if derived < 0 { return Err(IsolationError::ProfileHResult(derived)); }
        } else if hr < 0 {
            return Err(IsolationError::ProfileHResult(hr));
        }
        if sid.is_null() { return Err(IsolationError::MissingSid); }
        Ok(Self { sid })
    }

    pub(crate) fn security_capabilities(&self) -> SecurityCapabilities {
        SecurityCapabilities { app_container_sid: self.sid, capabilities: ptr::null_mut(),
            capability_count: 0, reserved: 0 }
    }

    /// Inspect the exact suspended process handle before durable admission.
    pub(crate) fn verify_suspended_process(&self, process: Handle) -> Result<(), IsolationError> {
        let mut raw = ptr::null_mut();
        if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut raw) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        let token = Token(raw);
        let mut is_container = 0u32;
        let mut returned = 0u32;
        if unsafe { GetTokenInformation(token.0, TOKEN_IS_APP_CONTAINER,
            (&mut is_container as *mut u32).cast(), size_of::<u32>() as u32, &mut returned) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        if is_container == 0 || returned != size_of::<u32>() as u32 {
            return Err(IsolationError::WrongToken);
        }
        let mut size = 0u32;
        unsafe { GetTokenInformation(token.0, TOKEN_APP_CONTAINER_SID,
            ptr::null_mut(), 0, &mut size); }
        if size < size_of::<*mut c_void>() as u32 || size > 4096 {
            return Err(IsolationError::WrongToken);
        }
        let mut words = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
        if unsafe { GetTokenInformation(token.0, TOKEN_APP_CONTAINER_SID,
            words.as_mut_ptr().cast(), size, &mut returned) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        if returned < size_of::<*mut c_void>() as u32 {
            return Err(IsolationError::WrongToken);
        }
        let observed = words[0] as *mut c_void;
        if observed.is_null() || unsafe { EqualSid(observed, self.sid) } == 0 {
            return Err(IsolationError::WrongToken);
        }
        Ok(())
    }
}

impl Drop for AppContainerProfile {
    fn drop(&mut self) { unsafe { FreeSid(self.sid); } }
}

struct Token(Handle);
impl Drop for Token {
    fn drop(&mut self) { unsafe { CloseHandle(self.0); } }
}

fn valid_profile_name(name: &str) -> bool {
    name.len() <= 64 && name.starts_with("Gogoke37.") && name[9..].bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) && name.len() > 9
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profile_name_is_bounded_and_not_a_path() {
        assert!(valid_profile_name("Gogoke37.instanceA"));
        assert!(!valid_profile_name("Gogoke37."));
        assert!(!valid_profile_name("Gogoke37.a\\b"));
        assert!(!valid_profile_name(&format!("Gogoke37.{}", "a".repeat(70))));
    }
}
