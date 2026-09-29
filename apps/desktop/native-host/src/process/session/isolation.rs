//! Host-owned Windows AppContainer identity for an admitted v37 process.
//! The profile SID is a kernel access principal, not a permission grant by itself.

use std::ffi::c_void;
use std::fmt;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::ptr;

type Handle = *mut c_void;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_IS_APP_CONTAINER: u32 = 29;
const TOKEN_CAPABILITIES: u32 = 30;
const TOKEN_APP_CONTAINER_SID: u32 = 31;
const PROFILE_ALREADY_EXISTS: u32 = 0x8007_00b7;
const FILE_OBJECT: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 4;
const GRANT_ACCESS: u32 = 1;
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_UNKNOWN: u32 = 0;
const OBJECT_AND_CONTAINER_INHERIT: u32 = 3;
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const FILE_GENERIC_EXECUTE: u32 = 0x0012_00a0;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const FILE_SHARE_ALL: u32 = 7;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const SE_GROUP_ENABLED: u32 = 4;

#[repr(C)]
struct SidAndAttributes { sid: *mut c_void, attributes: u32 }

#[repr(C)]
#[derive(Clone, Copy, Eq, PartialEq)]
struct FileTime { low: u32, high: u32 }

#[repr(C)]
struct FileInformation {
    attributes: u32,
    created: FileTime,
    accessed: FileTime,
    modified: FileTime,
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

impl FileInformation {
    fn same_object(&self, other: &Self) -> bool {
        self.volume_serial == other.volume_serial &&
            self.index_high == other.index_high && self.index_low == other.index_low
    }
    fn physical_directory(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 &&
            self.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
}

#[repr(C)]
struct TrusteeW {
    multiple: *mut TrusteeW,
    multiple_operation: u32,
    form: u32,
    kind: u32,
    name: *mut u16,
}

#[repr(C)]
struct ExplicitAccessW {
    permissions: u32,
    access_mode: u32,
    inheritance: u32,
    trustee: TrusteeW,
}

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
    fn GetSecurityInfo(handle: Handle, object_type: u32, information: u32,
        owner: *mut *mut c_void, group: *mut *mut c_void, dacl: *mut *mut c_void,
        sacl: *mut *mut c_void, descriptor: *mut *mut c_void) -> u32;
    fn SetEntriesInAclW(count: u32, entries: *mut ExplicitAccessW,
        old_acl: *mut c_void, new_acl: *mut *mut c_void) -> u32;
    fn SetSecurityInfo(handle: Handle, object_type: u32, information: u32,
        owner: *mut c_void, group: *mut c_void, dacl: *mut c_void,
        sacl: *mut c_void) -> u32;
    fn GetExplicitEntriesFromAclW(acl: *mut c_void, count: *mut u32,
        entries: *mut *mut ExplicitAccessW) -> u32;
    fn ConvertStringSidToSidW(text: *const u16, sid: *mut *mut c_void) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: Handle) -> i32;
    fn LocalFree(handle: Handle) -> Handle;
    fn CreateFileW(path: *const u16, access: u32, sharing: u32,
        security: *const c_void, creation: u32, flags: u32, template: Handle) -> Handle;
    fn GetFileInformationByHandle(handle: Handle, information: *mut FileInformation) -> i32;
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
    DirectoryNotFresh,
    DirectoryNotPhysical,
    Acl(io::Error),
}

impl fmt::Display for IsolationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProfileName => write!(f, "invalid AppContainer profile name"),
            Self::ProfileHResult(hr) => write!(f, "AppContainer profile HRESULT=0x{:08x}", *hr as u32),
            Self::MissingSid => write!(f, "AppContainer profile returned no SID"),
            Self::Token(error) => write!(f, "AppContainer token query: {error}"),
            Self::WrongToken => write!(f, "suspended process has a different AppContainer token"),
            Self::DirectoryNotFresh => write!(f, "AppContainer directory must be empty before ACL grant"),
            Self::DirectoryNotPhysical => write!(f, "AppContainer directory must be a physical directory"),
            Self::Acl(error) => write!(f, "AppContainer ACL: {error}"),
        }
    }
}

pub(crate) struct AppContainerProfile {
    sid: *mut c_void,
    internet_sid: Option<LocalAllocation>,
    internet_capability: Option<SidAndAttributes>,
}

impl AppContainerProfile {
    #[cfg(test)]
    pub(crate) fn derived_for_test(name: &str) -> Result<Self, IsolationError> {
        if !valid_profile_name(name) { return Err(IsolationError::InvalidProfileName); }
        let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        let hr = unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &mut sid) };
        if hr < 0 { return Err(IsolationError::ProfileHResult(hr)); }
        if sid.is_null() { return Err(IsolationError::MissingSid); }
        Ok(Self { sid, internet_sid: None, internet_capability: None })
    }

    pub(crate) fn ensure(name: &str, internet_client: bool) -> Result<Self, IsolationError> {
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
        let mut profile = Self { sid, internet_sid: None, internet_capability: None };
        if internet_client { profile.enable_internet_client()?; }
        Ok(profile)
    }

    pub(crate) fn security_capabilities(&self) -> SecurityCapabilities {
        SecurityCapabilities { app_container_sid: self.sid,
            capabilities: self.internet_capability.as_ref().map_or(ptr::null_mut(), |capability|
                (capability as *const SidAndAttributes).cast_mut().cast()),
            capability_count: u32::from(self.internet_capability.is_some()), reserved: 0 }
    }

    pub(crate) fn package_sid_string(&self) -> Result<String, IsolationError> {
        let mut raw = ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(self.sid, &mut raw) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        let allocation = LocalAllocation(raw.cast());
        let mut len = 0usize;
        while unsafe { *raw.add(len) } != 0 {
            if len >= 180 { return Err(IsolationError::WrongToken); }
            len += 1;
        }
        let value = std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(raw, len) })
            .to_string_lossy().into_owned();
        drop(allocation);
        Ok(value)
    }

    pub(crate) fn sid_identity(&self) -> Result<String, IsolationError> {
        self.package_sid_string()
    }

    fn enable_internet_client(&mut self) -> Result<(), IsolationError> {
        // Microsoft documents S-1-15-3-1 as the internetClient capability SID.
        // An absent network grant stays absent; there is no broad network fallback.
        let text: Vec<u16> = std::ffi::OsStr::new("S-1-15-3-1")
            .encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        if unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        if sid.is_null() { return Err(IsolationError::MissingSid); }
        self.internet_capability = Some(SidAndAttributes { sid, attributes: SE_GROUP_ENABLED });
        self.internet_sid = Some(LocalAllocation(sid));
        Ok(())
    }

    /// Grant only a newly created, empty session directory to this package SID.
    /// The ACE inherits to contents created after the grant. The ordinary user
    /// ACE remains; the AppContainer token additionally requires its SID.
    /// The caller must hold the directory's native custody and check its
    /// physical identity before and after this operation.
    pub(crate) fn grant_fresh_session_directory(&self, path: &Path)
        -> Result<(), IsolationError> {
        let before = std::fs::symlink_metadata(path).map_err(IsolationError::Acl)?;
        if !before.is_dir() || before.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(IsolationError::DirectoryNotPhysical);
        }
        if std::fs::read_dir(path).map_err(IsolationError::Acl)?.next().is_some() {
            return Err(IsolationError::DirectoryNotFresh);
        }
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe { CreateFileW(wide.as_ptr(), READ_CONTROL | WRITE_DAC, FILE_SHARE_ALL,
            ptr::null(), OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
        if handle as isize == -1 { return Err(IsolationError::Acl(io::Error::last_os_error())); }
        let handle = Token(handle);
        let physical = file_information(handle.0)?;
        if !physical.physical_directory() { return Err(IsolationError::DirectoryNotPhysical); }
        if std::fs::read_dir(path).map_err(IsolationError::Acl)?.next().is_some() {
            return Err(IsolationError::DirectoryNotFresh);
        }
        let mut old_acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        let status = unsafe { GetSecurityInfo(handle.0, FILE_OBJECT, DACL_SECURITY_INFORMATION,
            ptr::null_mut(), ptr::null_mut(), &mut old_acl, ptr::null_mut(), &mut descriptor) };
        if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
        let descriptor = LocalAllocation(descriptor);
        if old_acl.is_null() { return Err(IsolationError::DirectoryNotPhysical); }
        let mut entry = ExplicitAccessW {
            permissions: FILE_GENERIC_READ | FILE_GENERIC_WRITE | FILE_GENERIC_EXECUTE,
            access_mode: GRANT_ACCESS,
            inheritance: OBJECT_AND_CONTAINER_INHERIT,
            trustee: TrusteeW { multiple: ptr::null_mut(), multiple_operation: 0,
                form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN,
                name: self.sid.cast() },
        };
        let mut new_acl = ptr::null_mut();
        let status = unsafe { SetEntriesInAclW(1, &mut entry, old_acl, &mut new_acl) };
        if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
        let new_acl = LocalAllocation(new_acl);
        let status = unsafe { SetSecurityInfo(handle.0, FILE_OBJECT, DACL_SECURITY_INFORMATION,
            ptr::null_mut(), ptr::null_mut(), new_acl.0, ptr::null_mut()) };
        if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
        drop(descriptor);
        let after = std::fs::symlink_metadata(path).map_err(IsolationError::Acl)?;
        let observed = unsafe { CreateFileW(wide.as_ptr(), READ_CONTROL, FILE_SHARE_ALL,
            ptr::null(), OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
        if observed as isize == -1 { return Err(IsolationError::Acl(io::Error::last_os_error())); }
        let observed = Token(observed);
        let observed_info = file_information(observed.0)?;
        if !after.is_dir() || after.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || !physical.same_object(&observed_info) || !observed_info.physical_directory() {
            return Err(IsolationError::DirectoryNotPhysical);
        }
        Ok(())
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
        let mut capability_bytes = 0u32;
        unsafe { GetTokenInformation(token.0, TOKEN_CAPABILITIES,
            ptr::null_mut(), 0, &mut capability_bytes); }
        let alignment = std::mem::align_of::<SidAndAttributes>();
        let group_offset = (size_of::<u32>() + alignment - 1) & !(alignment - 1);
        if capability_bytes < group_offset as u32 || capability_bytes > 4096 {
            return Err(IsolationError::WrongToken);
        }
        let mut groups = vec![0usize; (capability_bytes as usize).div_ceil(size_of::<usize>())];
        if unsafe { GetTokenInformation(token.0, TOKEN_CAPABILITIES,
            groups.as_mut_ptr().cast(), capability_bytes, &mut returned) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        if returned < group_offset as u32 { return Err(IsolationError::WrongToken); }
        let count = unsafe { *(groups.as_ptr() as *const u32) } as usize;
        if count > 32 || count != usize::from(self.internet_capability.is_some()) ||
            (returned as usize) < group_offset + count * size_of::<SidAndAttributes>() {
            return Err(IsolationError::WrongToken);
        }
        if let Some(expected) = &self.internet_capability {
            let actual = unsafe { &*((groups.as_ptr() as *const u8).add(group_offset)
                as *const SidAndAttributes) };
            if actual.sid.is_null() || unsafe { EqualSid(actual.sid, expected.sid) } == 0 ||
                actual.attributes & SE_GROUP_ENABLED == 0 {
                return Err(IsolationError::WrongToken);
            }
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

fn file_information(handle: Handle) -> Result<FileInformation, IsolationError> {
    let mut information = std::mem::MaybeUninit::<FileInformation>::uninit();
    if unsafe { GetFileInformationByHandle(handle, information.as_mut_ptr()) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    Ok(unsafe { information.assume_init() })
}

struct LocalAllocation(Handle);
impl Drop for LocalAllocation {
    fn drop(&mut self) { unsafe { LocalFree(self.0); } }
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

    #[test]
    fn fresh_directory_acl_binds_exact_package_sid_and_rejects_existing_content() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let name = format!("Gogoke37.acltest{}", std::process::id());
        let wide: Vec<u16> = std::ffi::OsStr::new(&name).encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        let hr = unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &mut sid) };
        assert!(hr >= 0 && !sid.is_null(), "derive test package SID HRESULT={hr:#x}");
        let profile = AppContainerProfile { sid, internet_sid: None, internet_capability: None };
        let path = std::env::temp_dir().join(format!("gogoke-v37-acl-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        profile.grant_fresh_session_directory(&path).unwrap();
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let handle = unsafe { CreateFileW(path_wide.as_ptr(), READ_CONTROL, FILE_SHARE_ALL,
            ptr::null(), OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
        assert_ne!(handle as isize, -1);
        let handle = Token(handle);
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        assert_eq!(unsafe { GetSecurityInfo(handle.0, FILE_OBJECT, DACL_SECURITY_INFORMATION,
            ptr::null_mut(), ptr::null_mut(), &mut acl, ptr::null_mut(), &mut descriptor) }, 0);
        let _descriptor = LocalAllocation(descriptor);
        let mut count = 0;
        let mut raw_entries = ptr::null_mut();
        assert_eq!(unsafe { GetExplicitEntriesFromAclW(acl, &mut count, &mut raw_entries) }, 0);
        let _entries = LocalAllocation(raw_entries.cast());
        assert!(count > 0 && !raw_entries.is_null());
        let observed = unsafe { std::slice::from_raw_parts(raw_entries, count as usize) };
        assert!(observed.iter().any(|entry| entry.access_mode == GRANT_ACCESS &&
            entry.inheritance == OBJECT_AND_CONTAINER_INHERIT &&
            entry.trustee.form == TRUSTEE_IS_SID &&
            unsafe { EqualSid(entry.trustee.name.cast(), profile.sid) } != 0));
        std::fs::write(path.join("present"), b"fixture").unwrap();
        assert!(matches!(profile.grant_fresh_session_directory(&path),
            Err(IsolationError::DirectoryNotFresh)));
        std::fs::remove_file(path.join("present")).unwrap();
        drop(handle);
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn outbound_network_capability_is_explicit_and_exact() {
        let name: Vec<u16> = std::ffi::OsStr::new("Gogoke37.capabilitytest")
            .encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        assert!(unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) } >= 0);
        let mut profile = AppContainerProfile { sid, internet_sid: None, internet_capability: None };
        assert_eq!(profile.security_capabilities().capability_count, 0);
        profile.enable_internet_client().unwrap();
        let capabilities = profile.security_capabilities();
        assert_eq!(capabilities.capability_count, 1);
        let actual = unsafe { &*(capabilities.capabilities as *const SidAndAttributes) };
        assert_eq!(actual.attributes, SE_GROUP_ENABLED);
        assert_eq!(actual.sid, profile.internet_sid.as_ref().unwrap().0);
    }
}
