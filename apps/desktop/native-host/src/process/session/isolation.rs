//! Host-owned Windows AppContainer identity for an admitted v37 process.
//! The profile SID is a kernel access principal, not a permission grant by itself.

use std::ffi::c_void;
use std::fmt;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;
use crate::root::{RootIdentity, RootLock};
use super::credential_binding::{CredentialAlias, CredentialAliasScope,
    CredentialBinding, CredentialError};

#[path = "legacy_acl.rs"]
mod legacy_acl;
pub(crate) use legacy_acl::{LegacyAclInventory, LegacyHomeReceipt, LegacySourceReceipt,
    NativeCredentialAclRecoveryStep, adopt_holder_gone_source_baseline};

type Handle = *mut c_void;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_IS_APP_CONTAINER: u32 = 29;
const TOKEN_CAPABILITIES: u32 = 30;
const TOKEN_APP_CONTAINER_SID: u32 = 31;
const PROFILE_ALREADY_EXISTS: u32 = 0x8007_00b7;
const FILE_OBJECT: u32 = 1;
const DACL_SECURITY_INFORMATION: u32 = 4;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const SE_DACL_PROTECTED: u16 = 0x1000;
const GRANT_ACCESS: u32 = 1;
const DENY_ACCESS: u32 = 3;
const REVOKE_ACCESS: u32 = 4;
const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
const ACCESS_DENIED_ACE_TYPE: u8 = 1;
const ACL_SIZE_INFORMATION_CLASS: u32 = 2;
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_UNKNOWN: u32 = 0;
const OBJECT_AND_CONTAINER_INHERIT: u32 = 3;
const NO_INHERITANCE: u32 = 0;
const FILE_GENERIC_READ: u32 = 0x0012_0089;
const FILE_GENERIC_WRITE: u32 = 0x0012_0116;
const FILE_GENERIC_EXECUTE: u32 = 0x0012_00a0;
const FILE_ALL_ACCESS: u32 = 0x001f_01ff;
const CREDENTIAL_FILE_RIGHTS: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
const DELETE_ACCESS: u32 = 0x0001_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const FILE_SHARE_ALL: u32 = 7;
const OPEN_EXISTING: u32 = 3;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const FILE_ID_INFO_CLASS: i32 = 18;
const INHERITED_ACE: u32 = 0x10;
const INHERIT_ONLY_ACE: u32 = 0x08;
const SE_GROUP_ENABLED: u32 = 4;
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;

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
    fn physical_directory(&self) -> bool {
        self.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 &&
            self.attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
    }
    fn physical_file(&self) -> bool {
        self.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) == 0
            && self.links == 1
    }
}

#[repr(C)]
struct FileIdInfo { volume_serial_number: u64, file_id: [u8; 16] }

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

#[repr(C)]
struct AclSizeInformation { ace_count: u32, acl_bytes_in_use: u32, acl_bytes_free: u32 }

#[repr(C)]
struct AceHeader { ace_type: u8, ace_flags: u8, ace_size: u16 }

#[repr(C)]
struct AccessAce { header: AceHeader, mask: u32 }

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
    fn GetAclInformation(acl: *mut c_void, information: *mut c_void,
        length: u32, class: u32) -> i32;
    fn GetAce(acl: *mut c_void, index: u32, ace: *mut *mut c_void) -> i32;
    fn ConvertStringSidToSidW(text: *const u16, sid: *mut *mut c_void) -> i32;
    fn ConvertSidToStringSidW(sid: *mut c_void, text: *mut *mut u16) -> i32;
    fn GetSecurityDescriptorControl(descriptor: *mut c_void,
        control: *mut u16, revision: *mut u32) -> i32;
}

#[link(name = "OneCoreUAP")]
extern "system" {
    fn DeriveCapabilitySidsFromName(name: *const u16,
        group_sids: *mut *mut *mut c_void, group_count: *mut u32,
        capability_sids: *mut *mut *mut c_void, capability_count: *mut u32) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn CloseHandle(handle: Handle) -> i32;
    fn LocalFree(handle: Handle) -> Handle;
    fn GetCurrentProcess() -> Handle;
    fn CreateFileW(path: *const u16, access: u32, sharing: u32,
        security: *const c_void, creation: u32, flags: u32, template: Handle) -> Handle;
    fn GetFileInformationByHandle(handle: Handle, information: *mut FileInformation) -> i32;
    fn GetFileInformationByHandleEx(handle: Handle, class: i32,
        information: *mut c_void, length: u32) -> i32;
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
    AclObject { object: PathBuf, operation: &'static str, error: io::Error },
    AclWitnessMismatch,
    AclWitnessDetail { object: PathBuf, sid: String, expected: String,
        observed: Vec<(u32, u32, u32)> },
    InvalidRegistryCapability,
    InvalidIdentityServicesCapability,
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
            Self::AclObject { object, operation, error } =>
                write!(f, "AppContainer ACL {operation} for {object:?}: {error}"),
            Self::AclWitnessMismatch => write!(f, "AppContainer ACL witness does not match exact SID, rights, inheritance or object identity"),
            Self::AclWitnessDetail { object, sid, expected, observed } => {
                write!(f, "AppContainer ACL witness for {:?}, package SID {sid}: expected {expected}; observed", object)?;
                for (mode, rights, flags) in observed {
                    write!(f, " (mode={mode}, rights={rights:#x}, flags={flags:#x})")?;
                }
                if observed.is_empty() { write!(f, " no package ACE")?; }
                Ok(())
            }
            Self::InvalidRegistryCapability => write!(f, "registryRead did not derive exactly one capability SID"),
            Self::InvalidIdentityServicesCapability => write!(f, "lpacIdentityServices did not derive one distinct capability SID"),
        }
    }
}

fn acl_object(error: IsolationError, object: &Path, operation: &'static str) -> IsolationError {
    match error {
        IsolationError::Acl(error) => IsolationError::AclObject {
            object: object.to_path_buf(), operation, error },
        already @ IsolationError::AclObject { .. } => already,
        other => other,
    }
}

pub(crate) struct AppContainerProfile {
    sid: *mut c_void,
    internet_sid: Option<LocalAllocation>,
    internet_capability: Option<SidAndAttributes>,
    registry_sids: Option<DerivedCapabilitySids>,
    registry_capability: Option<SidAndAttributes>,
    identity_services_sids: Option<DerivedCapabilitySids>,
    combined_capabilities: Vec<SidAndAttributes>,
}

/// An open handle to one empty physical directory. The handle, rather than its
/// path spelling, identifies the object that may receive a package ACE.
pub(crate) struct FreshDirectory {
    path: PathBuf,
    handle: Token,
    identity: RootIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AclWitness {
    pub(crate) identity: RootIdentity,
    pub(crate) package_sid: String,
    pub(crate) rights: u32,
    pub(crate) inheritance: u32,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ExistingCredentialScopeAce { Inherited, Explicit, ExplicitAndInherited, Missing }

impl AppContainerProfile {
    /// Recover the exact historical package SID for metadata-only revocation.
    /// Derivation creates no profile and enables no capabilities or grants.
    pub(crate) fn derive_for_revocation(name: &str) -> Result<Self, IsolationError> {
        if !valid_profile_name(name) { return Err(IsolationError::InvalidProfileName); }
        let wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        let hr = unsafe { DeriveAppContainerSidFromAppContainerName(wide.as_ptr(), &mut sid) };
        if hr < 0 { return Err(IsolationError::ProfileHResult(hr)); }
        if sid.is_null() { return Err(IsolationError::MissingSid); }
        Ok(Self { sid, internet_sid: None, internet_capability: None,
            registry_sids: None, registry_capability: None, identity_services_sids: None,
            combined_capabilities: Vec::new() })
    }

    #[cfg(test)]
    pub(crate) fn derived_for_test(name: &str) -> Result<Self, IsolationError> {
        Self::derive_for_revocation(name)
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
        let mut profile = Self { sid, internet_sid: None, internet_capability: None,
            registry_sids: None,
            registry_capability: None,
            identity_services_sids: None,
            combined_capabilities: Vec::new() };
        if internet_client { profile.enable_internet_client()?; }
        profile.enable_registry_read()?;
        Ok(profile)
    }

    /// The fixed Codex CLI alone needs Windows identity services for SSPI.
    /// This named capability is separate from internetClient and all file ACEs.
    pub(crate) fn ensure_for_cli(name: &str, internet_client: bool) -> Result<Self, IsolationError> {
        let mut profile = Self::ensure(name, internet_client)?;
        profile.enable_identity_services()?;
        Ok(profile)
    }

    pub(crate) fn security_capabilities(&self) -> SecurityCapabilities {
        SecurityCapabilities { app_container_sid: self.sid,
            capabilities: if self.combined_capabilities.is_empty() { ptr::null_mut() }
                else { self.combined_capabilities.as_ptr().cast_mut().cast() },
            capability_count: self.combined_capabilities.len() as u32, reserved: 0 }
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
        self.combined_capabilities.push(SidAndAttributes { sid, attributes: SE_GROUP_ENABLED });
        Ok(())
    }

    fn enable_registry_read(&mut self) -> Result<(), IsolationError> {
        let name: Vec<u16> = std::ffi::OsStr::new("registryRead")
            .encode_wide().chain(Some(0)).collect();
        let derived = DerivedCapabilitySids::from_name(&name)?;
        if derived.capability_count != 1 || derived.capability_sids.is_null() {
            return Err(IsolationError::InvalidRegistryCapability);
        }
        let sid = unsafe { *derived.capability_sids };
        if sid.is_null() { return Err(IsolationError::InvalidRegistryCapability); }
        if self.internet_capability.as_ref().is_some_and(|internet|
            unsafe { EqualSid(internet.sid, sid) } != 0) {
            return Err(IsolationError::InvalidRegistryCapability);
        }
        let capability = SidAndAttributes { sid, attributes: SE_GROUP_ENABLED };
        self.combined_capabilities.push(SidAndAttributes { sid, attributes: SE_GROUP_ENABLED });
        self.registry_capability = Some(capability);
        self.registry_sids = Some(derived);
        Ok(())
    }

    fn enable_identity_services(&mut self) -> Result<(), IsolationError> {
        let name: Vec<u16> = std::ffi::OsStr::new("lpacIdentityServices")
            .encode_wide().chain(Some(0)).collect();
        let derived = DerivedCapabilitySids::from_name(&name)?;
        if derived.capability_count != 1 || derived.capability_sids.is_null() {
            return Err(IsolationError::InvalidIdentityServicesCapability);
        }
        let sid = unsafe { *derived.capability_sids };
        if sid.is_null() || self.combined_capabilities.iter().any(|capability|
            unsafe { EqualSid(capability.sid, sid) } != 0) {
            return Err(IsolationError::InvalidIdentityServicesCapability);
        }
        self.combined_capabilities.push(SidAndAttributes { sid, attributes: SE_GROUP_ENABLED });
        self.identity_services_sids = Some(derived);
        Ok(())
    }

    /// Hold one fresh directory by its filesystem identity before selecting a
    /// package grant. Existing content is deliberately outside this API.
    pub(crate) fn open_fresh_directory(path: &Path) -> Result<FreshDirectory, IsolationError> {
        require_fresh_physical_path(path)?;
        let handle = open_directory(path, READ_CONTROL | WRITE_DAC)?;
        if !file_information(handle.0)?.physical_directory() {
            return Err(IsolationError::DirectoryNotPhysical);
        }
        let identity = file_identity(handle.0)?;
        require_exact_fresh_path(path, &identity)?;
        Ok(FreshDirectory { path: path.to_path_buf(), handle, identity })
    }

    /// Grant read/traverse or read/write/traverse to this exact held directory.
    /// `inherit` affects only children created under this fresh target. A
    /// persistent instance ancestor must never use an inheritable package ACE.
    pub(crate) fn grant_held_fresh_directory(&self, directory: &FreshDirectory,
        writable: bool, inherit: bool) -> Result<(), IsolationError> {
        if file_identity(directory.handle.0)? != directory.identity {
            return Err(IsolationError::DirectoryNotPhysical);
        }
        require_exact_fresh_path(&directory.path, &directory.identity)?;
        let rights = directory_rights(writable);
        let inheritance = if inherit { OBJECT_AND_CONTAINER_INHERIT } else { NO_INHERITANCE };
        grant_exact_acl(directory.handle.0, self.sid, &directory.identity, rights, inheritance)?;
        require_exact_fresh_path(&directory.path, &directory.identity)?;
        Ok(())
    }

    /// Existing session callers get the same fresh, inheritable read/write ACE.
    pub(crate) fn grant_fresh_session_directory(&self, path: &Path)
        -> Result<(), IsolationError> {
        let directory = Self::open_fresh_directory(path)?;
        self.grant_held_fresh_directory(&directory, true, true)
    }

    /// Grant only the exact host-resolved instance home or host-created
    /// worktree root. The caller must supply F's previously recorded identity;
    /// neither this method nor its witness authorizes a path received on IPC.
    /// Descendants inherit from this root, never from its shared parent.
    pub(crate) fn grant_bound_tree(&self, path: &Path, expected: &RootIdentity,
        writable: bool) -> Result<AclWitness, IsolationError> {
        let root = open_bound_object(path, expected, true)?;
        let before = collect_tree(path)?;
        let rights = directory_rights(writable);
        let result = grant_exact_acl(root.0, self.sid, expected, rights,
            OBJECT_AND_CONTAINER_INHERIT)?;
        require_bound_path(path, expected, true)?;
        let after = collect_tree(path)?;
        if before != after { return Err(IsolationError::AclWitnessMismatch); }
        let witness = self.verify_bound_tree_grant(path, expected, writable)?;
        if witness.identity != result { return Err(IsolationError::AclWitnessMismatch); }
        Ok(witness)
    }

    /// Read back the root ACE and the inherited ACE on every existing child.
    /// This proves the observed tree now; the host must repeat it
    /// before use if any untrusted mutation can occur between grant and launch.
    pub(crate) fn verify_bound_tree_grant(&self, path: &Path,
        expected: &RootIdentity, writable: bool) -> Result<AclWitness, IsolationError> {
        require_bound_path(path, expected, true)?;
        let witness = self.verify_bound_directory_grant(path, expected, writable)?;
        let rights = directory_rights(writable);
        for (child, identity, directory) in collect_tree(path)? {
            let object = open_physical_object(&child, directory, READ_CONTROL)?;
            if file_identity(object.0)
                .map_err(|error| acl_object(error, &child, "read child identity"))? != identity {
                return Err(IsolationError::AclWitnessMismatch);
            }
            let entries = package_aces(object.0, self.sid)
                .map_err(|error| acl_object(error, &child, "read child ACEs"))?;
            if entries.len() != 1 || entries[0].0 != GRANT_ACCESS || entries[0].1 != rights ||
                entries[0].2 & INHERITED_ACE == 0 ||
                entries[0].2 & INHERIT_ONLY_ACE != 0 {
                return Err(IsolationError::AclWitnessDetail {
                    object: child.strip_prefix(path).map_err(|error| IsolationError::Acl(
                        io::Error::new(io::ErrorKind::InvalidData,
                            format!("ACL relative object: {error}"))))?.to_path_buf(),
                    sid: self.package_sid_string()?,
                    expected: format!("one inherited effective grant, rights={rights:#x}, inherited flag set, inherit-only flag clear"),
                    observed: entries });
            }
            require_bound_path(&child, &identity, directory)?;
        }
        require_bound_path(path, expected, true)?;
        Ok(witness)
    }

    /// Live child files can be created, renamed, and deleted by the admitted
    /// process. Only the host-bound directory root is a stable launch grant.
    pub(crate) fn verify_bound_directory_grant(&self, path: &Path,
        expected: &RootIdentity, writable: bool) -> Result<AclWitness, IsolationError> {
        let root = open_physical_object(path, true, READ_CONTROL)?;
        if &file_identity(root.0)
            .map_err(|error| acl_object(error, path, "read root identity"))? != expected {
            return Err(IsolationError::AclWitnessMismatch);
        }
        let rights = directory_rights(writable);
        let root_aces = package_aces(root.0, self.sid)
            .map_err(|error| acl_object(error, path, "read root ACEs"))?;
        if root_aces.as_slice() != &[(GRANT_ACCESS, rights, OBJECT_AND_CONTAINER_INHERIT)] {
            return Err(IsolationError::AclWitnessDetail { object: PathBuf::from("."),
                sid: self.package_sid_string()?,
                expected: format!("one explicit grant, rights={rights:#x}, flags={OBJECT_AND_CONTAINER_INHERIT:#x}"),
                observed: root_aces });
        }
        require_bound_path(path, expected, true)?;
        Ok(AclWitness { identity: expected.clone(), package_sid: self.package_sid_string()?,
            rights, inheritance: OBJECT_AND_CONTAINER_INHERIT })
    }

    /// Codex/File only: the exact F-registered auth alias is the sole
    /// multi-link exception. The ordinary tree method above stays strict.
    pub(crate) fn grant_bound_credential_tree(&self, path: &Path,
        expected: &RootIdentity, binding: &CredentialBinding,
        alias: &CredentialAlias, writable: bool) -> Result<AclWitness, CredentialError> {
        if alias.root() != path || alias.root_identity() != expected
            || alias.file_identity() != binding.identity() {
            return Err(CredentialError::IdentityChanged);
        }
        protect_credential_source_acl(binding)?;
        self.grant_credential_alias(binding, alias)?;
        self.grant_registered_credential_tree(path,expected,binding,alias,writable)
    }

    /// F has already issued or reconciled this exact profile's credential
    /// grant. Tree setup must only verify the shared source, never replace its
    /// baseline during cold recovery or change another profile's source ACE.
    pub(crate) fn grant_registered_credential_tree(&self,path:&Path,expected:&RootIdentity,
        binding:&CredentialBinding,alias:&CredentialAlias,writable:bool)
        ->Result<AclWitness,CredentialError> {
        if alias.root()!=path || alias.root_identity()!=expected || alias.file_identity()!=binding.identity() {
            return Err(CredentialError::IdentityChanged);
        }
        self.verify_credential_alias(binding,alias)?;
        let root = open_bound_object(path, expected, true)?;
        let before = collect_tree_with_credential(path, binding, alias)?;
        let result = grant_exact_acl(root.0, self.sid, expected,
            directory_rights(writable), OBJECT_AND_CONTAINER_INHERIT)?;
        require_bound_path(path, expected, true)?;
        let after = collect_tree_with_credential(path, binding, alias)?;
        if before != after { return Err(IsolationError::AclWitnessMismatch.into()); }
        // SetSecurityInfo's parent inheritance is not a sufficient witness for
        // every already-existing ordinary leaf. The fixed CLI's renamed
        // .sandbox_migration leaf was physically single-link and unprotected,
        // yet had no new SID ACE after the parent grant. Apply the same scoped
        // rights explicitly only to such missing, unprotected descendants.
        let rights = directory_rights(writable);
        for (child, identity, directory, credential) in &after {
            if *credential { continue; }
            let object = open_physical_object(child, *directory, READ_CONTROL)?;
            if &file_identity(object.0)? != identity { return Err(CredentialError::IdentityChanged); }
            if self.credential_scope_child_ace(object.0, child, *directory, rights)?
                == ExistingCredentialScopeAce::Missing {
                drop(object);
                let writable_object = open_bound_object(child, identity, *directory)?;
                if let Err(error) = grant_exact_acl(writable_object.0, self.sid, identity, rights,
                    if *directory { OBJECT_AND_CONTAINER_INHERIT } else { NO_INHERITANCE }) {
                    // SetSecurityInfo may materialize the same parent's inherited
                    // grant while adding the missing explicit leaf grant. This
                    // exact pair has the same rights; the general one-ACE writer
                    // remains strict for every other object.
                    if !matches!(&error, IsolationError::AclWitnessDetail { .. })
                        || self.credential_scope_child_ace(writable_object.0, child,
                            *directory, rights)? != ExistingCredentialScopeAce::ExplicitAndInherited {
                        return Err(error.into());
                    }
                }
                if self.credential_scope_child_ace(writable_object.0, child, *directory, rights)?
                    == ExistingCredentialScopeAce::Missing {
                    return Err(IsolationError::AclWitnessMismatch.into());
                }
            }
        }
        if after != collect_tree_with_credential(path, binding, alias)? {
            return Err(IsolationError::AclWitnessMismatch.into());
        }
        let witness = self.verify_bound_credential_tree_grant(path, expected, binding, alias, writable)?;
        if witness.identity != result { return Err(IsolationError::AclWitnessMismatch.into()); }
        Ok(witness)
    }

    fn credential_scope_child_ace(&self, handle: Handle, child: &Path,
        directory: bool, rights: u32) -> Result<ExistingCredentialScopeAce, IsolationError> {
        let observed = package_aces(handle, self.sid)?;
        if observed.is_empty() {
            if dacl_protected(handle)? {
                return Err(IsolationError::AclWitnessDetail { object: child.to_path_buf(),
                    sid: self.package_sid_string()?,
                    expected: "unprotected missing descendant eligible for exact scoped grant".into(),
                    observed });
            }
            return Ok(ExistingCredentialScopeAce::Missing);
        }
        let inherited_flags = INHERITED_ACE |
            if directory { OBJECT_AND_CONTAINER_INHERIT } else { NO_INHERITANCE };
        if observed.as_slice() == &[(GRANT_ACCESS, rights, inherited_flags)] {
            return Ok(ExistingCredentialScopeAce::Inherited);
        }
        let explicit_flags = if directory { OBJECT_AND_CONTAINER_INHERIT } else { NO_INHERITANCE };
        if observed.as_slice() == &[(GRANT_ACCESS, rights, explicit_flags)]
            && !dacl_protected(handle)? {
            return Ok(ExistingCredentialScopeAce::Explicit);
        }
        if observed.as_slice() == &[(GRANT_ACCESS, rights, explicit_flags),
            (GRANT_ACCESS, rights, inherited_flags)] && !dacl_protected(handle)? {
            return Ok(ExistingCredentialScopeAce::ExplicitAndInherited);
        }
        Err(IsolationError::AclWitnessDetail { object: child.to_path_buf(),
            sid: self.package_sid_string()?, expected: format!(
                "one exact inherited, exact explicit, or exact same-rights explicit-plus-inherited unprotected grant, rights={rights:#x}"),
            observed })
    }

    pub(crate) fn verify_bound_credential_tree_grant(&self, path: &Path,
        expected: &RootIdentity, binding: &CredentialBinding,
        alias: &CredentialAlias, writable: bool) -> Result<AclWitness, CredentialError> {
        if alias.root() != path || alias.root_identity() != expected
            || alias.file_identity() != binding.identity() {
            return Err(CredentialError::IdentityChanged);
        }
        require_bound_path(path, expected, true)?;
        let witness = self.verify_bound_directory_grant(path, expected, writable)?;
        let rights = directory_rights(writable);
        for (child, identity, directory, credential) in
            collect_tree_with_credential(path, binding, alias)? {
            if credential {
                binding.with_exact_alias(alias, |handle| {
                    if file_identity(handle)? != identity || !dacl_protected(handle)?
                        || package_aces(handle, self.sid)?.as_slice() !=
                            &[(GRANT_ACCESS, CREDENTIAL_FILE_RIGHTS, NO_INHERITANCE)] {
                        return Err(IsolationError::AclWitnessMismatch.into());
                    }
                    Ok(())
                })?;
                continue;
            }
            let object = open_physical_object(&child, directory, READ_CONTROL)?;
            if file_identity(object.0)? != identity { return Err(IsolationError::AclWitnessMismatch.into()); }
            if self.credential_scope_child_ace(object.0, &child, directory, rights)?
                == ExistingCredentialScopeAce::Missing {
                return Err(IsolationError::AclWitnessMismatch.into());
            }
            require_bound_path(&child, &identity, directory)?;
        }
        require_bound_path(path, expected, true)?;
        Ok(witness)
    }

    /// The shared auth object's DACL grants only the exact admitted SID.
    /// Rights contain no DELETE, WRITE_DAC or WRITE_OWNER bits.
    pub(crate) fn grant_credential_alias(&self, binding: &CredentialBinding,
        alias: &CredentialAlias) -> Result<(), CredentialError> {
        protect_credential_source_acl(binding)?;
        binding.with_exact_alias(alias, |handle| {
            if !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch.into()); }
            grant_exact_acl(handle, self.sid, binding.identity(),
                CREDENTIAL_FILE_RIGHTS, NO_INHERITANCE)?;
            if !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch.into()); }
            Ok(())
        })
    }

    /// Read back this exact registered alias and this profile's completed
    /// protected file grant. This does not prepare or change the DACL.
    pub(crate) fn verify_credential_alias(&self, binding: &CredentialBinding,
        alias: &CredentialAlias) -> Result<(), CredentialError> {
        binding.with_exact_alias(alias, |handle| {
            if &file_identity(handle)? != binding.identity() || !dacl_protected(handle)?
                || package_aces(handle, self.sid)?.as_slice() !=
                    &[(GRANT_ACCESS, CREDENTIAL_FILE_RIGHTS, NO_INHERITANCE)] {
                return Err(IsolationError::AclWitnessMismatch.into());
            }
            Ok(())
        })
    }

    /// Read-only completion witness for a durable pending SID revocation.
    /// F must still supply the complete alias registry to its binding.
    pub(crate) fn verify_revoked_credential_alias(&self, binding: &CredentialBinding,
        alias: &CredentialAlias) -> Result<(), CredentialError> {
        binding.with_exact_alias(alias, |handle| {
            if &file_identity(handle)? != binding.identity() || !dacl_protected(handle)?
                || !package_aces(handle, self.sid)?.is_empty() {
                return Err(IsolationError::AclWitnessMismatch.into());
            }
            Ok(())
        })
    }

    /// Read-only source witness when F has durably completed alias removal.
    /// The held metadata handle is reused; this requests no new WRITE_DAC
    /// handle, changes no ACE, and cannot read credential data.
    pub(crate) fn verify_revoked_credential_source(&self,
        binding: &CredentialBinding) -> Result<(), CredentialError> {
        binding.with_source_metadata_acl(|handle| {
            if &file_identity(handle)? != binding.identity() || !dacl_protected(handle)?
                || !package_aces(handle, self.sid)?.is_empty() {
                return Err(IsolationError::AclWitnessMismatch.into());
            }
            Ok(())
        })
    }

    /// Stopping one generation removes only its package SID from the shared
    /// object. F controls dormant alias unlink after whole-instance quiescence.
    pub(crate) fn revoke_credential_alias(&self, binding: &CredentialBinding,
        alias: &CredentialAlias) -> Result<(), CredentialError> {
        binding.with_exact_alias(alias, |handle| {
            revoke_exact_credential_ace(handle, self.sid, binding.identity())?;
            Ok(())
        })
    }

    /// F may call this only after proving whole-instance quiescence and
    /// settling every alias intent. A fresh ordinary login can then replace
    /// its source DACL before the later LPAC account/read observer is admitted.
    pub(crate) fn prepare_quiescent_owner_account_source(root: &RootLock,
        home: &Path, home_identity: &RootIdentity,
        binding: &CredentialBinding) -> Result<(), CredentialError> {
        binding.verify_registered_aliases(&[])?;
        let (observed, links) = CredentialBinding::observe_source_metadata(root,
            &home.join("auth.json"), home_identity)?;
        if &observed != binding.identity() || links != 1 {
            return Err(CredentialError::IdentityChanged);
        }
        protect_credential_source_acl(binding)
    }

    /// Fixed Codex/File account/read only. The ordinary same-user login is
    /// launched separately. This LPAC observer receives exact source read,
    /// non-inheritable HOME traverse, and its fresh runtime directory. An
    /// existing protected source DACL is required: UNKNOWN live jobs must not
    /// trigger a DACL rebuild at this entry point.
    pub(crate) fn grant_bound_owner_account_observer(&self, root: &RootLock,
        home: &Path, home_identity: &RootIdentity, runtime: &Path,
        runtime_identity: &RootIdentity, binding: &CredentialBinding,
        registered_aliases: &[CredentialAliasScope]) -> Result<(), CredentialError> {
        let source = home.join("auth.json");
        if runtime.parent() != Some(home) || runtime == source {
            return Err(CredentialError::Invalid("account observer runtime is not an exact HOME child"));
        }
        binding.verify_registered_aliases(registered_aliases)?;
        let (observed, _) = CredentialBinding::observe_source_metadata(root,
            &source, home_identity)?;
        if &observed != binding.identity() { return Err(CredentialError::IdentityChanged); }
        binding.with_source_acl(|handle, _prepared| {
            if !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch.into()); }
            let (_token, _user_buffer, user) = host_user_sid()?;
            if package_aces(handle, user)?.as_slice() !=
                &[(GRANT_ACCESS, FILE_ALL_ACCESS, NO_INHERITANCE)] {
                return Err(IsolationError::AclWitnessMismatch.into());
            }
            grant_exact_acl(handle, self.sid, binding.identity(),
                FILE_GENERIC_READ, NO_INHERITANCE)?;
            if !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch.into()); }
            Ok(())
        })?;
        let home_object = open_bound_object(home, home_identity, true)?;
        grant_exact_acl(home_object.0, self.sid, home_identity,
            FILE_GENERIC_EXECUTE, NO_INHERITANCE)?;
        let fresh_runtime = Self::open_fresh_directory(runtime)?;
        if &fresh_runtime.identity != runtime_identity {
            return Err(CredentialError::IdentityChanged);
        }
        self.grant_held_fresh_directory(&fresh_runtime, true, true)?;
        self.grant_owner_account_installation_file(home,home_identity)?;
        self.verify_bound_owner_account_observer(root, home, home_identity,
            runtime, runtime_identity, binding, registered_aliases)
    }

    pub(crate) fn verify_bound_owner_account_observer(&self, root: &RootLock,
        home: &Path, home_identity: &RootIdentity, runtime: &Path,
        runtime_identity: &RootIdentity, binding: &CredentialBinding,
        registered_aliases: &[CredentialAliasScope]) -> Result<(), CredentialError> {
        if runtime.parent() != Some(home) {
            return Err(CredentialError::Invalid("account observer runtime escaped HOME"));
        }
        binding.verify_registered_aliases(registered_aliases)?;
        let source = home.join("auth.json");
        let (source_identity, _) = CredentialBinding::observe_source_metadata(root,
            &source, home_identity)?;
        if &source_identity != binding.identity() { return Err(CredentialError::IdentityChanged); }
        let home_object = open_physical_object(home, true, READ_CONTROL)?;
        if &file_identity(home_object.0)? != home_identity ||
            package_aces(home_object.0, self.sid)?.as_slice() !=
                &[(GRANT_ACCESS, FILE_GENERIC_EXECUTE, NO_INHERITANCE)] {
            return Err(IsolationError::AclWitnessMismatch.into());
        }
        self.verify_bound_tree_grant(runtime, runtime_identity, true)?;
        self.verify_owner_account_installation_file(home,home_identity)?;
        let alias_paths: Vec<PathBuf> = registered_aliases.iter().map(|scope|
            binding.alias(scope, registered_aliases).map(|alias| alias.path()))
            .collect::<Result<_, _>>()?;
        let mut pending = vec![home.to_path_buf()];
        let mut source_found = false;
        let mut aliases_found = vec![false; alias_paths.len()];
        while let Some(parent) = pending.pop() {
            for entry in std::fs::read_dir(&parent).map_err(|error|
                CredentialError::Io { operation: "enumerate account observer HOME", source: error })? {
                let child = entry.map_err(|error| CredentialError::Io {
                    operation: "read account observer entry", source: error })?.path();
                if child == runtime { continue; }
                if child==home.join("installation_id") {continue;}
                let metadata = std::fs::symlink_metadata(&child).map_err(|error|
                    CredentialError::Io { operation: "read account observer object metadata", source: error })?;
                if child == source || alias_paths.contains(&child) {
                    if !metadata.is_file() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                        return Err(CredentialError::Invalid("registered credential name is not physical"));
                    }
                    let object = open_directory(&child, READ_CONTROL)?;
                    let info = file_information(object.0)?;
                    if info.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
                        || &file_identity(object.0)? != binding.identity()
                        || !dacl_protected(object.0)?
                        || package_aces(object.0, self.sid)?.as_slice() !=
                            &[(GRANT_ACCESS, FILE_GENERIC_READ, NO_INHERITANCE)] {
                        return Err(IsolationError::AclWitnessMismatch.into());
                    }
                    if child == source { source_found = true; }
                    if let Some(index) = alias_paths.iter().position(|path| path == &child) {
                        aliases_found[index] = true;
                    }
                    continue;
                }
                let directory = metadata.is_dir();
                let object = open_physical_object(&child, directory, READ_CONTROL)?;
                if !package_aces(object.0, self.sid)?.is_empty() {
                    return Err(IsolationError::AclWitnessDetail {
                        object: child.strip_prefix(home).unwrap_or(&child).to_path_buf(),
                        sid: self.package_sid_string()?,
                        expected: "no observer package ACE outside auth object and runtime".into(),
                        observed: package_aces(object.0, self.sid)?,
                    }.into());
                }
                if directory { pending.push(child); }
            }
        }
        if !source_found || aliases_found.iter().any(|found| !found) {
            return Err(CredentialError::Invalid("registered credential source or alias absent from HOME"));
        }
        binding.verify_registered_aliases(registered_aliases)?;
        Ok(())
    }

    /// Initial account/read with no registered auth object. Existing history
    /// is never granted to this SID; only HOME traverse and a fresh runtime
    /// are admitted. A present auth object must use the registered binding.
    pub(crate) fn grant_bound_owner_account_empty(&self, root: &RootLock,
        home: &Path, home_identity: &RootIdentity, runtime: &Path,
        runtime_identity: &RootIdentity) -> Result<(), CredentialError> {
        if runtime.parent() != Some(home) || runtime == home.join("auth.json") {
            return Err(CredentialError::Invalid("empty account runtime is not an exact HOME child"));
        }
        require_absent_owner_auth(root, home, home_identity)?;
        let home_object = open_bound_object(home, home_identity, true)?;
        grant_exact_acl(home_object.0, self.sid, home_identity,
            FILE_GENERIC_EXECUTE, NO_INHERITANCE)?;
        let fresh_runtime = Self::open_fresh_directory(runtime)?;
        if &fresh_runtime.identity != runtime_identity {
            return Err(CredentialError::IdentityChanged);
        }
        self.grant_held_fresh_directory(&fresh_runtime, true, true)?;
        self.grant_owner_account_installation_file(home,home_identity)?;
        self.verify_bound_owner_account_empty(root, home, home_identity,
            runtime, runtime_identity)
    }

    pub(crate) fn verify_bound_owner_account_empty(&self, root: &RootLock,
        home: &Path, home_identity: &RootIdentity, runtime: &Path,
        runtime_identity: &RootIdentity) -> Result<(), CredentialError> {
        if runtime.parent() != Some(home) || runtime == home.join("auth.json") {
            return Err(CredentialError::Invalid("empty account runtime escaped HOME"));
        }
        require_absent_owner_auth(root, home, home_identity)?;
        let home_object = open_physical_object(home, true, READ_CONTROL)?;
        if &file_identity(home_object.0)? != home_identity
            || package_aces(home_object.0, self.sid)?.as_slice() !=
                &[(GRANT_ACCESS, FILE_GENERIC_EXECUTE, NO_INHERITANCE)] {
            return Err(IsolationError::AclWitnessMismatch.into());
        }
        self.verify_bound_tree_grant(runtime, runtime_identity, true)?;
        self.verify_owner_account_installation_file(home,home_identity)?;
        let mut pending = vec![home.to_path_buf()];
        while let Some(parent) = pending.pop() {
            for entry in std::fs::read_dir(&parent).map_err(|error|
                CredentialError::Io { operation: "enumerate empty account HOME", source: error })? {
                let child = entry.map_err(|error| CredentialError::Io {
                    operation: "read empty account entry", source: error })?.path();
                if child == runtime { continue; }
                if child==home.join("installation_id") {continue;}
                let metadata = std::fs::symlink_metadata(&child).map_err(|error|
                    CredentialError::Io { operation: "read empty account object metadata", source: error })?;
                let directory = metadata.is_dir();
                let object = open_physical_object(&child, directory, READ_CONTROL)?;
                if !package_aces(object.0, self.sid)?.is_empty() {
                    return Err(IsolationError::AclWitnessDetail {
                        object: child.strip_prefix(home).unwrap_or(&child).to_path_buf(),
                        sid: self.package_sid_string()?,
                        expected: "no observer package ACE outside fresh runtime".into(),
                        observed: package_aces(object.0, self.sid)?,
                    }.into());
                }
                if directory { pending.push(child); }
            }
        }
        require_absent_owner_auth(root, home, home_identity)
    }

    /// F may use this only after every old OwnerLogin custody for this
    /// instance is STOPPED and no intent is UNKNOWN. The known old grant was
    /// one inheritable writable ACE on HOME. Remove that parent inheritance
    /// first, then remove only residual inherited copies for this same SID.
    /// A registered auth object is the sole multi-link exception.
    pub(crate) fn migrate_legacy_owner_login_grant(&self, root: &RootLock,
        home: &Path, home_identity: &RootIdentity,
        credential: Option<(&CredentialBinding, &[CredentialAliasScope])>)
        -> Result<(), CredentialError> {
        require_bound_path(home, home_identity, true)?;
        if let Some((binding, aliases)) = credential {
            binding.verify_registered_aliases(aliases)?;
            let (identity, _) = CredentialBinding::observe_source_metadata(root,
                &home.join("auth.json"), home_identity)?;
            if &identity != binding.identity() { return Err(CredentialError::IdentityChanged); }
        } else {
            require_absent_owner_auth(root, home, home_identity)?;
        }
        let home_object = open_bound_object(home, home_identity, true)?;
        revoke_known_legacy_owner_ace(home_object.0, self.sid, home_identity, true)?;
        let before = collect_legacy_owner_tree(home, credential)?;
        let mut credential_processed = false;
        for (path, identity, directory, registered_credential) in &before {
            if *registered_credential && credential_processed { continue; }
            if *registered_credential {
                let (binding, _) = credential.ok_or(CredentialError::Invalid(
                    "legacy credential witness missing"))?;
                binding.with_source_metadata_acl(|handle| {
                    if &file_identity(handle)? != identity {
                        return Err(CredentialError::IdentityChanged);
                    }
                    revoke_known_legacy_owner_ace(handle, self.sid, identity, false)?;
                    Ok(())
                })?;
                credential_processed = true;
            } else {
                let object = open_bound_object(path, identity, *directory)?;
                revoke_known_legacy_owner_ace(object.0, self.sid, identity, false)?;
            }
        }
        let after = collect_legacy_owner_tree(home, credential)?;
        if before != after { return Err(IsolationError::AclWitnessMismatch.into()); }
        for (path, identity, directory, registered_credential) in &after {
            if *registered_credential {
                let (binding, _) = credential.ok_or(CredentialError::Invalid(
                    "legacy credential witness missing"))?;
                binding.with_source_metadata_acl(|handle| {
                    if &file_identity(handle)? != identity || !package_aces(handle, self.sid)?.is_empty() {
                        return Err(IsolationError::AclWitnessMismatch.into());
                    }
                    Ok(())
                })?;
            } else {
                let object = open_physical_object(path, *directory, READ_CONTROL)?;
                if &file_identity(object.0)? != identity || !package_aces(object.0, self.sid)?.is_empty() {
                    return Err(IsolationError::AclWitnessMismatch.into());
                }
            }
        }
        let home_readback = open_physical_object(home, true, READ_CONTROL)?;
        if &file_identity(home_readback.0)? != home_identity
            || !package_aces(home_readback.0, self.sid)?.is_empty() {
            return Err(IsolationError::AclWitnessMismatch.into());
        }
        if let Some((binding, aliases)) = credential {
            binding.verify_registered_aliases(aliases)?;
        } else {
            require_absent_owner_auth(root, home, home_identity)?;
        }
        Ok(())
    }

    /// Only the known old whole-HOME grant or an empty root needs the legacy
    /// descendant pass. The already-installed narrow traverse grant does not.
    pub(crate) fn has_legacy_owner_login_grant(&self, home: &Path,
        home_identity: &RootIdentity) -> Result<bool, CredentialError> {
        let object = open_physical_object(home, true, READ_CONTROL)?;
        if &file_identity(object.0)? != home_identity {
            return Err(CredentialError::IdentityChanged);
        }
        let observed = package_aces(object.0, self.sid)?;
        let answer = match observed.as_slice() {
            [] => true,
            [(GRANT_ACCESS, rights, OBJECT_AND_CONTAINER_INHERIT)]
                if *rights == directory_rights(true) => true,
            [(GRANT_ACCESS, FILE_GENERIC_EXECUTE, NO_INHERITANCE)] => false,
            _ => return Err(IsolationError::AclWitnessDetail {
                object: PathBuf::from("."), sid: self.package_sid_string()?,
                expected: "empty, exact old inheritable writable grant, or exact new traverse grant".into(),
                observed,
            }.into()),
        };
        require_bound_path(home, home_identity, true)?;
        Ok(answer)
    }

    /// The fixed app-server always opens this non-secret native installation
    /// UUID read/write/create, even for account/read. Precreate only an empty
    /// ordinary file; the CLI owns its value. No directory/history grant.
    fn grant_owner_account_installation_file(&self,home:&Path,home_identity:&RootIdentity)
        ->Result<(),CredentialError> {
        require_bound_path(home,home_identity,true)?;
        let path=home.join("installation_id");
        match std::fs::symlink_metadata(&path) {
            Ok(_)=>(),
            Err(source) if source.raw_os_error()==Some(2)=>{
                std::fs::OpenOptions::new().write(true).create_new(true).open(&path)
                    .map_err(|source|CredentialError::Io {operation:"create empty CLI installation metadata",source})?;
            },
            Err(source)=>return Err(CredentialError::Io {operation:"CLI installation metadata",source}),
        }
        let object=open_physical_object(&path,false,READ_CONTROL|WRITE_DAC)?;
        let identity=file_identity(object.0)?;
        grant_exact_acl(object.0,self.sid,&identity,FILE_GENERIC_READ|FILE_GENERIC_WRITE,NO_INHERITANCE)?;
        require_bound_path(home,home_identity,true)?;
        self.verify_owner_account_installation_file(home,home_identity)
    }

    fn verify_owner_account_installation_file(&self,home:&Path,home_identity:&RootIdentity)
        ->Result<(),CredentialError> {
        require_bound_path(home,home_identity,true)?;
        let object=open_physical_object(&home.join("installation_id"),false,READ_CONTROL)?;
        if package_aces(object.0,self.sid)?.as_slice()!=
            &[(GRANT_ACCESS,FILE_GENERIC_READ|FILE_GENERIC_WRITE,NO_INHERITANCE)] {
            return Err(IsolationError::AclWitnessMismatch.into());
        }
        Ok(())
    }

    /// Single-link identity for compatibility modules and other bound assets.
    pub(crate) fn capture_program_identity(path: &Path) -> Result<RootIdentity, IsolationError> {
        let object = open_physical_object(path, false, READ_CONTROL)?;
        let identity = file_identity(object.0)?;
        require_bound_path(path, &identity, false)?;
        Ok(identity)
    }

    pub(crate) fn grant_bound_program(&self, path: &Path, expected: &RootIdentity)
        -> Result<AclWitness, IsolationError> {
        let object = open_bound_object(path, expected, false)?;
        let rights = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
        let result = grant_exact_acl(object.0, self.sid, expected, rights, NO_INHERITANCE)?;
        require_bound_path(path, expected, false)?;
        let witness = self.verify_bound_program_grant(path, expected)?;
        if witness.identity != result { return Err(IsolationError::AclWitnessMismatch); }
        Ok(witness)
    }

    pub(crate) fn verify_bound_program_grant(&self, path: &Path,
        expected: &RootIdentity) -> Result<AclWitness, IsolationError> {
        let object = open_physical_object(path, false, READ_CONTROL)?;
        let rights = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
        if &file_identity(object.0)? != expected ||
            package_aces(object.0, self.sid)?.as_slice() != &[(GRANT_ACCESS, rights, NO_INHERITANCE)] {
            return Err(IsolationError::AclWitnessMismatch);
        }
        require_bound_path(path, expected, false)?;
        Ok(AclWitness { identity: expected.clone(), package_sid: self.package_sid_string()?,
            rights, inheritance: NO_INHERITANCE })
    }

    /// Only LaunchEvidence uses this path for F's fixed catalog program after
    /// verifying its stored digest and version. Installed CLI names may be hard
    /// links to the same ordinary file; the grant follows that physical FileID.
    /// This does not apply to modules, credential aliases or writable data.
    pub(crate) fn capture_catalog_program_identity(path: &Path)
        -> Result<RootIdentity, IsolationError> {
        let object = open_catalog_program(path, READ_CONTROL)?;
        let identity = file_identity(object.0)?;
        require_catalog_program_path(path, &identity)?;
        Ok(identity)
    }

    pub(crate) fn grant_bound_catalog_program(&self, path: &Path, expected: &RootIdentity)
        -> Result<AclWitness, IsolationError> {
        let object = open_catalog_program(path, READ_CONTROL | WRITE_DAC)?;
        if &file_identity(object.0)? != expected {
            return Err(IsolationError::AclWitnessMismatch);
        }
        require_catalog_program_path(path, expected)?;
        let result = grant_exact_acl(object.0, self.sid, expected,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE, NO_INHERITANCE)?;
        require_catalog_program_path(path, expected)?;
        let witness = self.verify_bound_catalog_program_grant(path, expected)?;
        if witness.identity != result { return Err(IsolationError::AclWitnessMismatch); }
        Ok(witness)
    }

    pub(crate) fn verify_bound_catalog_program_grant(&self, path: &Path,
        expected: &RootIdentity) -> Result<AclWitness, IsolationError> {
        let object = open_catalog_program(path, READ_CONTROL)?;
        let rights = FILE_GENERIC_READ | FILE_GENERIC_EXECUTE;
        if &file_identity(object.0)? != expected ||
            package_aces(object.0, self.sid)?.as_slice() != &[(GRANT_ACCESS, rights, NO_INHERITANCE)] {
            return Err(IsolationError::AclWitnessMismatch);
        }
        require_catalog_program_path(path, expected)?;
        Ok(AclWitness { identity: expected.clone(), package_sid: self.package_sid_string()?,
            rights, inheritance: NO_INHERITANCE })
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
        let expected_count = self.combined_capabilities.len();
        if count > 32 || count != expected_count ||
            (returned as usize) < group_offset + count * size_of::<SidAndAttributes>() {
            return Err(IsolationError::WrongToken);
        }
        let actual = unsafe { std::slice::from_raw_parts(
            (groups.as_ptr() as *const u8).add(group_offset) as *const SidAndAttributes,
            count) };
        let expected = &self.combined_capabilities;
        // Windows may add mandatory/default metadata bits to token groups;
        // compare the access-effective enabled/deny-only state exactly.
        if expected.iter().any(|wanted| actual.iter().filter(|found|
            !found.sid.is_null() &&
            found.attributes & (SE_GROUP_ENABLED | SE_GROUP_USE_FOR_DENY_ONLY)
                == wanted.attributes &&
            unsafe { EqualSid(found.sid, wanted.sid) } != 0).count() != 1) {
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

fn open_directory(path: &Path, access: u32) -> Result<Token, IsolationError> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let handle = unsafe { CreateFileW(wide.as_ptr(), access, FILE_SHARE_ALL,
        ptr::null(), OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
    if handle as isize == -1 { return Err(IsolationError::AclObject {
        object: path.to_path_buf(), operation: "open object", error: io::Error::last_os_error() }); }
    Ok(Token(handle))
}

fn directory_rights(writable: bool) -> u32 {
    FILE_GENERIC_READ | FILE_GENERIC_EXECUTE |
        // SQLite and other owned runtime files need rename/delete as well as
        // data writes. Grant object DELETE only within this exact writable
        // tree; neither read-only trees nor its parent get delete-child.
        (if writable { FILE_GENERIC_WRITE | DELETE_ACCESS } else { 0 })
}

fn file_identity(handle: Handle) -> Result<RootIdentity, IsolationError> {
    let mut info = FileIdInfo { volume_serial_number: 0, file_id: [0; 16] };
    if unsafe { GetFileInformationByHandleEx(handle, FILE_ID_INFO_CLASS,
        (&mut info as *mut FileIdInfo).cast(), size_of::<FileIdInfo>() as u32) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    Ok(RootIdentity { volume_serial: info.volume_serial_number, file_id: info.file_id })
}

fn require_absent_owner_auth(root: &RootLock, home: &Path,
    home_identity: &RootIdentity) -> Result<(), CredentialError> {
    require_bound_path(home, home_identity, true)?;
    match CredentialBinding::observe_source_metadata(root, &home.join("auth.json"), home_identity) {
        Err(CredentialError::Io { operation: "observe source metadata", source })
            if source.raw_os_error() == Some(2) => {},
        Err(error) => return Err(error),
        Ok(_) => return Err(CredentialError::Invalid("empty account auth object exists")),
    }
    require_bound_path(home, home_identity, true)?;
    Ok(())
}

fn open_physical_object(path: &Path, directory: bool, access: u32)
    -> Result<Token, IsolationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| IsolationError::AclObject {
        object: path.to_path_buf(), operation: "read object metadata", error })?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 ||
        metadata.is_dir() != directory || (!directory && !metadata.is_file()) {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    let object = open_directory(path, access)?;
    let info = file_information(object.0)
        .map_err(|error| acl_object(error, path, "read physical object attributes"))?;
    if !(if directory { info.physical_directory() } else { info.physical_file() }) {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    Ok(object)
}

// A deliberately separate opener: the ordinary object and handle checks are
// retained, but a catalog program can have multiple names for one FileID.
// No generic option can relax physical_file for any other asset or data tree.
fn open_catalog_program(path: &Path, access: u32) -> Result<Token, IsolationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| IsolationError::AclObject {
        object: path.to_path_buf(), operation: "read catalog program metadata", error })?;
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !metadata.is_file() {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    let object = open_directory(path, access)?;
    let info = file_information(object.0)
        .map_err(|error| acl_object(error, path, "read catalog program attributes"))?;
    if info.attributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0 ||
        info.links == 0 {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    Ok(object)
}

fn require_catalog_program_path(path: &Path, expected: &RootIdentity)
    -> Result<(), IsolationError> {
    let object = open_catalog_program(path, READ_CONTROL)?;
    if &file_identity(object.0)
        .map_err(|error| acl_object(error, path, "read bound catalog program identity"))? != expected {
        return Err(IsolationError::AclWitnessMismatch);
    }
    Ok(())
}

fn require_bound_path(path: &Path, expected: &RootIdentity, directory: bool)
    -> Result<(), IsolationError> {
    let object = open_physical_object(path, directory, READ_CONTROL)?;
    if &file_identity(object.0)
        .map_err(|error| acl_object(error, path, "read bound object identity"))? != expected {
        return Err(IsolationError::AclWitnessMismatch);
    }
    Ok(())
}

fn open_bound_object(path: &Path, expected: &RootIdentity, directory: bool)
    -> Result<Token, IsolationError> {
    let object = open_physical_object(path, directory, READ_CONTROL | WRITE_DAC)?;
    if &file_identity(object.0)
        .map_err(|error| acl_object(error, path, "read writable bound object identity"))? != expected {
        return Err(IsolationError::AclWitnessMismatch);
    }
    require_bound_path(path, expected, directory)?;
    Ok(object)
}

fn collect_tree(root: &Path) -> Result<Vec<(PathBuf, RootIdentity, bool)>, IsolationError> {
    let mut pending = vec![root.to_path_buf()];
    let mut objects = Vec::new();
    while let Some(parent) = pending.pop() {
        for child in std::fs::read_dir(&parent).map_err(|error| IsolationError::AclObject {
            object: parent.clone(), operation: "enumerate tree directory", error })? {
            let path = child.map_err(|error| IsolationError::AclObject {
                object: parent.clone(), operation: "read tree directory entry", error })?.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| IsolationError::AclObject {
                object: path.clone(), operation: "read tree child metadata", error })?;
            let directory = metadata.is_dir();
            let object = open_physical_object(&path, directory, READ_CONTROL)?;
            let identity = file_identity(object.0)
                .map_err(|error| acl_object(error, &path, "read tree child identity"))?;
            require_bound_path(&path, &identity, directory)?;
            if directory { pending.push(path.clone()); }
            objects.push((path, identity, directory));
        }
    }
    objects.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(objects)
}

fn collect_legacy_owner_tree(home: &Path,
    credential: Option<(&CredentialBinding, &[CredentialAliasScope])>)
    -> Result<Vec<(PathBuf, RootIdentity, bool, bool)>, CredentialError> {
    let mut registered_paths = Vec::new();
    if let Some((binding, aliases)) = credential {
        binding.verify_registered_aliases(aliases)?;
        registered_paths.push(home.join("auth.json"));
        for scope in aliases {
            let alias = binding.alias(scope, aliases)?;
            registered_paths.push(alias.path());
        }
        if registered_paths.iter().enumerate().any(|(index, path)|
            !path.starts_with(home) || registered_paths[..index].contains(path)) {
            return Err(CredentialError::Invalid("registered legacy alias escaped HOME or duplicates source"));
        }
    }
    let mut found = vec![false; registered_paths.len()];
    let mut pending = vec![home.to_path_buf()];
    let mut objects = Vec::new();
    while let Some(parent) = pending.pop() {
        for child in std::fs::read_dir(&parent).map_err(|error| CredentialError::Io {
            operation: "enumerate legacy owner HOME", source: error })? {
            let path = child.map_err(|error| CredentialError::Io {
                operation: "read legacy owner entry", source: error })?.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| CredentialError::Io {
                operation: "read legacy owner object metadata", source: error })?;
            let directory = metadata.is_dir();
            let registered = registered_paths.iter().position(|name| name == &path);
            let identity = if registered.is_some() {
                if directory || !metadata.is_file()
                    || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(CredentialError::Invalid("registered legacy credential is not physical"));
                }
                let (binding, _) = credential.ok_or(CredentialError::Invalid(
                    "legacy credential witness missing"))?;
                binding.with_source_metadata_acl(|handle| {
                    let identity = file_identity(handle)?;
                    if &identity != binding.identity() { return Err(CredentialError::IdentityChanged); }
                    Ok(identity)
                })?
            } else {
                let object = open_physical_object(&path, directory, READ_CONTROL)?;
                file_identity(object.0)?
            };
            if let Some(index) = registered { found[index] = true; }
            if directory { pending.push(path.clone()); }
            objects.push((path, identity, directory, registered.is_some()));
        }
    }
    if found.iter().any(|seen| !seen) {
        return Err(CredentialError::Invalid("registered legacy credential name absent from HOME"));
    }
    objects.sort_by(|left, right| left.0.cmp(&right.0));
    if let Some((binding, aliases)) = credential { binding.verify_registered_aliases(aliases)?; }
    Ok(objects)
}

fn revoke_known_legacy_owner_ace(handle: Handle, sid: *mut c_void,
    expected: &RootIdentity, home: bool) -> Result<(), IsolationError> {
    if &file_identity(handle)? != expected { return Err(IsolationError::AclWitnessMismatch); }
    let observed = package_aces(handle, sid)?;
    if observed.is_empty() { return Ok(()); }
    let expected_rights = directory_rights(true);
    let known = if home {
        observed.as_slice() == &[(GRANT_ACCESS, expected_rights, OBJECT_AND_CONTAINER_INHERIT)]
    } else {
        observed.len() == 1 && observed[0].0 == GRANT_ACCESS
            && observed[0].1 == expected_rights
            && observed[0].2 & INHERITED_ACE != 0
            && observed[0].2 & !(INHERITED_ACE | OBJECT_AND_CONTAINER_INHERIT) == 0
    };
    if !known { return Err(IsolationError::AclWitnessMismatch); }
    let mut old_acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut old_acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if old_acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let mut entry = ExplicitAccessW { permissions: 0, access_mode: REVOKE_ACCESS,
        inheritance: NO_INHERITANCE, trustee: TrusteeW { multiple: ptr::null_mut(),
            multiple_operation: 0, form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN,
            name: sid.cast() } };
    let mut new_acl = ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &mut entry, old_acl, &mut new_acl) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if new_acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let new_acl = LocalAllocation(new_acl);
    let status = unsafe { SetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), new_acl.0, ptr::null_mut()) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if &file_identity(handle)? != expected || !package_aces(handle, sid)?.is_empty() {
        return Err(IsolationError::AclWitnessMismatch);
    }
    Ok(())
}

fn collect_tree_with_credential(root: &Path, binding: &CredentialBinding,
    alias: &CredentialAlias) -> Result<Vec<(PathBuf, RootIdentity, bool, bool)>, CredentialError> {
    if alias.root() != root { return Err(CredentialError::IdentityChanged); }
    let exact_alias = alias.path();
    let mut pending = vec![root.to_path_buf()];
    let mut objects = Vec::new();
    let mut found_alias = false;
    while let Some(parent) = pending.pop() {
        for child in std::fs::read_dir(&parent)
            .map_err(|error| CredentialError::Io { operation: "enumerate scope tree", source: error })? {
            let path = child.map_err(|error| CredentialError::Io {
                operation: "read scope entry", source: error })?.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error|
                CredentialError::Io { operation: "read scope object metadata", source: error })?;
            let directory = metadata.is_dir();
            if path == exact_alias {
                if found_alias || directory || !metadata.is_file()
                    || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(CredentialError::Invalid("registered alias is not one physical leaf"));
                }
                binding.with_exact_alias(alias, |handle| {
                    if file_identity(handle)? != *alias.file_identity() {
                        return Err(IsolationError::AclWitnessMismatch.into());
                    }
                    Ok(())
                })?;
                objects.push((path, alias.file_identity().clone(), false, true));
                found_alias = true;
                continue;
            }
            let object = open_physical_object(&path, directory, READ_CONTROL)?;
            let identity = file_identity(object.0)?;
            require_bound_path(&path, &identity, directory)?;
            if directory { pending.push(path.clone()); }
            objects.push((path, identity, directory, false));
        }
    }
    if !found_alias { return Err(CredentialError::Invalid("registered alias missing from exact scope")); }
    objects.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(objects)
}

fn package_aces(handle: Handle, sid: *mut c_void)
    -> Result<Vec<(u32, u32, u32)>, IsolationError> {
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    // Read ACE headers directly. EXPLICIT_ACCESS is a reconstructed description
    // of an ACL, while the inherited flag we witness lives on the actual ACE.
    let mut size = AclSizeInformation { ace_count: 0, acl_bytes_in_use: 0,
        acl_bytes_free: 0 };
    if unsafe { GetAclInformation(acl, (&mut size as *mut AclSizeInformation).cast(),
        size_of::<AclSizeInformation>() as u32, ACL_SIZE_INFORMATION_CLASS) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    let mut matched = Vec::new();
    for index in 0..size.ace_count {
        let mut ace = ptr::null_mut();
        if unsafe { GetAce(acl, index, &mut ace) } == 0 {
            return Err(IsolationError::Acl(io::Error::last_os_error()));
        }
        if ace.is_null() { return Err(IsolationError::AclWitnessMismatch); }
        let header = unsafe { &*ace.cast::<AceHeader>() };
        let mode = match header.ace_type {
            ACCESS_ALLOWED_ACE_TYPE => GRANT_ACCESS,
            ACCESS_DENIED_ACE_TYPE => DENY_ACCESS,
            other => return Err(IsolationError::Acl(io::Error::new(io::ErrorKind::InvalidData,
                format!("unsupported DACL ACE type {other:#x} at index {index}")))),
        };
        if header.ace_size < 16 { return Err(IsolationError::AclWitnessMismatch); }
        let access = unsafe { &*ace.cast::<AccessAce>() };
        let ace_sid = unsafe { ace.cast::<u8>().add(8).cast() };
        if unsafe { EqualSid(ace_sid, sid) } != 0 {
            matched.push((mode, access.mask, u32::from(header.ace_flags)));
        }
    }
    Ok(matched)
}

fn grant_exact_acl(handle: Handle, sid: *mut c_void, expected: &RootIdentity,
    rights: u32, inheritance: u32) -> Result<RootIdentity, IsolationError> {
    if &file_identity(handle)? != expected { return Err(IsolationError::AclWitnessMismatch); }
    let before = package_aces(handle, sid)?;
    if before.as_slice() == &[(GRANT_ACCESS, rights, inheritance)] {
        return Ok(expected.clone());
    }
    if !before.is_empty() { return Err(IsolationError::AclWitnessMismatch); }
    let mut old_acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut old_acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if old_acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let mut entry = ExplicitAccessW { permissions: rights, access_mode: GRANT_ACCESS,
        inheritance, trustee: TrusteeW { multiple: ptr::null_mut(), multiple_operation: 0,
            form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN, name: sid.cast() } };
    let mut new_acl = ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &mut entry, old_acl, &mut new_acl) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let new_acl = LocalAllocation(new_acl);
    let status = unsafe { SetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), new_acl.0, ptr::null_mut()) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if &file_identity(handle)? != expected {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let observed = package_aces(handle, sid)?;
    if observed.as_slice() != &[(GRANT_ACCESS, rights, inheritance)] {
        return Err(IsolationError::AclWitnessDetail { object: PathBuf::from("."),
            sid: "bound package SID".to_owned(),
            expected: format!("post-write explicit grant, rights={rights:#x}, flags={inheritance:#x}"),
            observed });
    }
    Ok(expected.clone())
}

fn dacl_protected(handle: Handle) -> Result<bool, IsolationError> {
    let mut acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if descriptor.is_null() || acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let mut control = 0u16;
    let mut revision = 0u32;
    if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    Ok(control & SE_DACL_PROTECTED != 0)
}

fn host_user_sid() -> Result<(Token, Vec<usize>, *mut c_void), IsolationError> {
    let mut raw = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(IsolationError::Token(io::Error::last_os_error()));
    }
    let token = Token(raw);
    let mut size = 0u32;
    unsafe { GetTokenInformation(token.0, 1, ptr::null_mut(), 0, &mut size); }
    if size < size_of::<SidAndAttributes>() as u32 || size > 4096 {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let mut words = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    let mut returned = 0u32;
    if unsafe { GetTokenInformation(token.0, 1, words.as_mut_ptr().cast(),
        size, &mut returned) } == 0 {
        return Err(IsolationError::Token(io::Error::last_os_error()));
    }
    if returned < size_of::<SidAndAttributes>() as u32 {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let sid = unsafe { (*(words.as_ptr().cast::<SidAndAttributes>())).sid };
    if sid.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    Ok((token, words, sid))
}

fn well_known_sid(value: &str) -> Result<LocalAllocation, IsolationError> {
    let wide: Vec<u16> = std::ffi::OsStr::new(value).encode_wide().chain(Some(0)).collect();
    let mut raw = ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut raw) } == 0 {
        return Err(IsolationError::Acl(io::Error::last_os_error()));
    }
    if raw.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    Ok(LocalAllocation(raw))
}

fn protect_credential_source_acl(binding: &CredentialBinding) -> Result<(), CredentialError> {
    binding.with_source_acl(|handle, prepared| {
        if !prepared {
            let (_token, _user_buffer, user) = host_user_sid()?;
            let system = well_known_sid("S-1-5-18")?;
            let administrators = well_known_sid("S-1-5-32-544")?;
            let mut entries = [user, system.0, administrators.0].map(|sid|
                ExplicitAccessW { permissions: FILE_ALL_ACCESS, access_mode: GRANT_ACCESS,
                    inheritance: NO_INHERITANCE, trustee: TrusteeW {
                        multiple: ptr::null_mut(), multiple_operation: 0,
                        form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN,
                        name: sid.cast() } });
            let mut raw_acl = ptr::null_mut();
            let status = unsafe { SetEntriesInAclW(entries.len() as u32,
                entries.as_mut_ptr(), ptr::null_mut(), &mut raw_acl) };
            if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)).into()); }
            if raw_acl.is_null() { return Err(IsolationError::AclWitnessMismatch.into()); }
            let acl = LocalAllocation(raw_acl);
            let status = unsafe { SetSecurityInfo(handle, FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(), ptr::null_mut(), acl.0, ptr::null_mut()) };
            if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32)).into()); }
        }
        if !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch.into()); }
        let (_token, _user_buffer, user) = host_user_sid()?;
        if package_aces(handle, user)?.as_slice() !=
            &[(GRANT_ACCESS, FILE_ALL_ACCESS, NO_INHERITANCE)] {
            return Err(IsolationError::AclWitnessMismatch.into());
        }
        Ok(())
    })
}

fn revoke_exact_credential_ace(handle: Handle, sid: *mut c_void,
    expected: &RootIdentity) -> Result<(), IsolationError> {
    if &file_identity(handle)? != expected { return Err(IsolationError::AclWitnessMismatch); }
    let observed = package_aces(handle, sid)?;
    if observed.is_empty() { return Ok(()); }
    if observed.as_slice() != &[(GRANT_ACCESS, CREDENTIAL_FILE_RIGHTS, NO_INHERITANCE)] {
        return Err(IsolationError::AclWitnessMismatch);
    }
    let mut old_acl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe { GetSecurityInfo(handle, FILE_OBJECT, DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), &mut old_acl, ptr::null_mut(), &mut descriptor) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    let _descriptor = LocalAllocation(descriptor);
    if old_acl.is_null() || !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch); }
    let mut entry = ExplicitAccessW { permissions: 0, access_mode: REVOKE_ACCESS,
        inheritance: NO_INHERITANCE, trustee: TrusteeW { multiple: ptr::null_mut(),
            multiple_operation: 0, form: TRUSTEE_IS_SID, kind: TRUSTEE_IS_UNKNOWN,
            name: sid.cast() } };
    let mut new_acl = ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(1, &mut entry, old_acl, &mut new_acl) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if new_acl.is_null() { return Err(IsolationError::AclWitnessMismatch); }
    let new_acl = LocalAllocation(new_acl);
    let status = unsafe { SetSecurityInfo(handle, FILE_OBJECT,
        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
        ptr::null_mut(), ptr::null_mut(), new_acl.0, ptr::null_mut()) };
    if status != 0 { return Err(IsolationError::Acl(io::Error::from_raw_os_error(status as i32))); }
    if &file_identity(handle)? != expected || !package_aces(handle, sid)?.is_empty()
        || !dacl_protected(handle)? { return Err(IsolationError::AclWitnessMismatch); }
    Ok(())
}

fn require_fresh_physical_path(path: &Path) -> Result<(), IsolationError> {
    let metadata = std::fs::symlink_metadata(path).map_err(IsolationError::Acl)?;
    if !metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(IsolationError::DirectoryNotPhysical);
    }
    if std::fs::read_dir(path).map_err(IsolationError::Acl)?.next().is_some() {
        return Err(IsolationError::DirectoryNotFresh);
    }
    Ok(())
}

fn require_exact_fresh_path(path: &Path, identity: &RootIdentity)
    -> Result<(), IsolationError> {
    require_fresh_physical_path(path)?;
    require_bound_path(path, identity, true)?;
    require_fresh_physical_path(path)
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

struct DerivedCapabilitySids {
    group_sids: *mut *mut c_void,
    group_count: u32,
    capability_sids: *mut *mut c_void,
    capability_count: u32,
}

impl DerivedCapabilitySids {
    fn from_name(name: &[u16]) -> Result<Self, IsolationError> {
        let mut derived = Self { group_sids: ptr::null_mut(), group_count: 0,
            capability_sids: ptr::null_mut(), capability_count: 0 };
        if unsafe { DeriveCapabilitySidsFromName(name.as_ptr(),
            &mut derived.group_sids, &mut derived.group_count,
            &mut derived.capability_sids, &mut derived.capability_count) } == 0 {
            return Err(IsolationError::Token(io::Error::last_os_error()));
        }
        Ok(derived)
    }
}

impl Drop for DerivedCapabilitySids {
    fn drop(&mut self) {
        unsafe {
            if !self.group_sids.is_null() {
                for sid in std::slice::from_raw_parts(self.group_sids, self.group_count as usize) {
                    if !sid.is_null() { LocalFree(*sid); }
                }
                LocalFree(self.group_sids.cast());
            }
            if !self.capability_sids.is_null() {
                for sid in std::slice::from_raw_parts(self.capability_sids,
                    self.capability_count as usize) {
                    if !sid.is_null() { LocalFree(*sid); }
                }
                LocalFree(self.capability_sids.cast());
            }
        }
    }
}

fn valid_profile_name(name: &str) -> bool {
    name.len() <= 64 && name.starts_with("Gogoke37.") && name[9..].bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')) && name.len() > 9
}

#[cfg(test)]
#[path = "catalog_program_tests.rs"]
mod catalog_program_tests;

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
        let profile = AppContainerProfile { sid, internet_sid: None, internet_capability: None,
            registry_sids: None, registry_capability: None, identity_services_sids: None,
            combined_capabilities: Vec::new() };
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
    fn held_fresh_directory_can_grant_read_without_inheritance() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.ReadOnlyAcl").unwrap();
        let path = std::env::temp_dir().join(format!("gogoke-v37-read-acl-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let held = AppContainerProfile::open_fresh_directory(&path).unwrap();
        profile.grant_held_fresh_directory(&held, false, false).unwrap();
        let mut acl = ptr::null_mut();
        let mut descriptor = ptr::null_mut();
        assert_eq!(unsafe { GetSecurityInfo(held.handle.0, FILE_OBJECT, DACL_SECURITY_INFORMATION,
            ptr::null_mut(), ptr::null_mut(), &mut acl, ptr::null_mut(), &mut descriptor) }, 0);
        let _descriptor = LocalAllocation(descriptor);
        let mut count = 0;
        let mut raw_entries = ptr::null_mut();
        assert_eq!(unsafe { GetExplicitEntriesFromAclW(acl, &mut count, &mut raw_entries) }, 0);
        let _entries = LocalAllocation(raw_entries.cast());
        assert!(count > 0 && !raw_entries.is_null());
        let observed = unsafe { std::slice::from_raw_parts(raw_entries, count as usize) };
        assert!(observed.iter().any(|entry| entry.access_mode == GRANT_ACCESS &&
            entry.inheritance == NO_INHERITANCE &&
            entry.permissions & 0x0001 != 0 && entry.permissions & 0x0002 == 0 &&
            entry.trustee.form == TRUSTEE_IS_SID &&
            unsafe { EqualSid(entry.trustee.name.cast(), profile.sid) } != 0));
        drop(held);
        std::fs::remove_dir(path).unwrap();
    }

    #[test]
    fn bound_nonempty_tree_grants_only_its_recorded_instance_identity() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("gogoke-v37-bound-acl-{}-{nonce}", std::process::id()));
        let home = base.join("instanceA");
        let sibling = base.join("instanceB");
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        std::fs::write(home.join("gogoke-instance.marker"), b"registered").unwrap();
        std::fs::write(home.join("sessions").join("state"), b"existing").unwrap();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.BoundInstanceAcl").unwrap();
        let identity = crate::root::inspect_root(&home).unwrap().identity;
        let other = crate::root::inspect_root(&sibling).unwrap().identity;
        assert!(matches!(profile.grant_bound_tree(&home, &other, true),
            Err(IsolationError::AclWitnessMismatch)));
        let witness = profile.grant_bound_tree(&home, &identity, true).unwrap();
        assert_eq!(witness.identity, identity);
        assert_eq!(witness.package_sid, profile.sid_identity().unwrap());
        assert_eq!(witness.rights, directory_rights(true));
        assert_eq!(witness.inheritance, OBJECT_AND_CONTAINER_INHERIT);
        assert_eq!(profile.verify_bound_tree_grant(&home, &identity, true).unwrap(), witness);
        assert_eq!(profile.grant_bound_tree(&home, &identity, true).unwrap(), witness);
        let sibling_handle = open_physical_object(&sibling, true, READ_CONTROL).unwrap();
        assert!(package_aces(sibling_handle.0, profile.sid).unwrap().is_empty());
        drop(sibling_handle);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn active_root_grant_survives_delete_pending_child_without_accepting_wrong_root() {
        use std::time::{SystemTime, UNIX_EPOCH};
        #[link(name = "kernel32")]
        extern "system" {
            fn SetFileInformationByHandle(handle: Handle, class: i32,
                information: *const c_void, length: u32) -> i32;
        }
        #[repr(C)]
        struct FileDispositionInfo { delete_file: u8 }
        #[repr(C)]
        #[derive(Default)]
        struct FileStandardInfo { allocation_size: i64, end_of_file: i64,
            links: u32, delete_pending: u8, directory: u8 }

        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("gogoke-v37-active-root-{}-{nonce}", std::process::id()));
        let home = base.join("instanceA");
        let other = base.join("instanceB");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir(&other).unwrap();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.ActiveRootAcl").unwrap();
        let identity = crate::root::inspect_root(&home).unwrap().identity;
        let other_identity = crate::root::inspect_root(&other).unwrap().identity;
        let witness = profile.grant_bound_tree(&home, &identity, true).unwrap();
        assert_eq!(profile.verify_bound_directory_grant(&home, &identity, true).unwrap(), witness);
        assert!(matches!(profile.verify_bound_directory_grant(&home, &other_identity, true),
            Err(IsolationError::AclWitnessMismatch)), "another physical root cannot inherit this witness");
        assert!(matches!(profile.verify_bound_directory_grant(&home, &identity, false),
            Err(IsolationError::AclWitnessDetail { .. })), "root ACE rights cannot change with the claimed tier");

        let pending = home.join("dynamic-child.tmp");
        std::fs::write(&pending, b"owned runtime data").unwrap();
        let held = open_directory(&pending, DELETE_ACCESS | 0x0080).unwrap();
        let disposition = FileDispositionInfo { delete_file: 1 };
        let marked = unsafe { SetFileInformationByHandle(held.0, 4,
            (&disposition as *const FileDispositionInfo).cast(), size_of::<FileDispositionInfo>() as u32) };
        let original_error = io::Error::last_os_error();
        assert_ne!(marked, 0, "actual file disposition failed: {original_error}");
        let mut standard = FileStandardInfo::default();
        let observed = unsafe { GetFileInformationByHandleEx(held.0, 1,
            (&mut standard as *mut FileStandardInfo).cast(), size_of::<FileStandardInfo>() as u32) };
        let original_error = io::Error::last_os_error();
        assert_ne!(observed, 0, "actual held file state failed: {original_error}");
        assert_ne!(standard.delete_pending, 0, "held file must actually be delete-pending");
        assert_eq!(profile.verify_bound_directory_grant(&home, &identity, true).unwrap(), witness,
            "active verification still reads the exact bound root while the child is delete-pending");
        drop(held);
        // A stable hard-linked leaf remains enumerable and must still be
        // rejected by the unchanged pre-activation physical-file guard.
        let outside = other.join("owned-instrument-file");
        let alias = home.join("hard-linked-child");
        std::fs::write(&outside, b"instrument-only bytes").unwrap();
        std::fs::hard_link(&outside, &alias).unwrap();
        assert!(matches!(profile.verify_bound_tree_grant(&home, &identity, true),
            Err(IsolationError::DirectoryNotPhysical)), "strict admission still rejects aliased descendants");
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn inherited_package_ace_from_shared_parent_is_not_a_bound_home_grant() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("gogoke-v37-parent-acl-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.ParentAcl").unwrap();
        profile.grant_fresh_session_directory(&base).unwrap();
        let home = base.join("instanceA");
        std::fs::create_dir(&home).unwrap();
        std::fs::write(home.join("gogoke-instance.marker"), b"registered").unwrap();
        let identity = crate::root::inspect_root(&home).unwrap().identity;
        assert!(matches!(profile.grant_bound_tree(&home, &identity, true),
            Err(IsolationError::AclWitnessMismatch)));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn bound_program_grant_reads_back_exact_file_and_does_not_inherit() {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir().join(format!("gogoke-v37-program-acl-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&base).unwrap();
        let program = base.join("catalog.exe");
        let other = base.join("other.exe");
        std::fs::write(&program, b"catalog object").unwrap();
        std::fs::write(&other, b"other object").unwrap();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.ProgramAcl").unwrap();
        let identity = AppContainerProfile::capture_program_identity(&program).unwrap();
        let other_identity = AppContainerProfile::capture_program_identity(&other).unwrap();
        assert!(matches!(profile.grant_bound_program(&program, &other_identity),
            Err(IsolationError::AclWitnessMismatch)));
        let witness = profile.grant_bound_program(&program, &identity).unwrap();
        assert_eq!(witness.identity, identity);
        assert_eq!(witness.inheritance, NO_INHERITANCE);
        assert_eq!(profile.verify_bound_program_grant(&program, &identity).unwrap(), witness);
        let other_handle = open_physical_object(&other, false, READ_CONTROL).unwrap();
        assert!(package_aces(other_handle.0, profile.sid).unwrap().is_empty());
        drop(other_handle);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn outbound_network_capability_is_explicit_and_exact() {
        let name: Vec<u16> = std::ffi::OsStr::new("Gogoke37.capabilitytest")
            .encode_wide().chain(Some(0)).collect();
        let mut sid = ptr::null_mut();
        assert!(unsafe { DeriveAppContainerSidFromAppContainerName(name.as_ptr(), &mut sid) } >= 0);
        let mut profile = AppContainerProfile { sid, internet_sid: None, internet_capability: None,
            registry_sids: None, registry_capability: None, identity_services_sids: None,
            combined_capabilities: Vec::new() };
        assert_eq!(profile.security_capabilities().capability_count, 0);
        profile.enable_internet_client().unwrap();
        let capabilities = profile.security_capabilities();
        assert_eq!(capabilities.capability_count, 1);
        let actual = unsafe { &*(capabilities.capabilities as *const SidAndAttributes) };
        assert_eq!(actual.attributes, SE_GROUP_ENABLED);
        assert_eq!(actual.sid, profile.internet_sid.as_ref().unwrap().0);
        profile.enable_registry_read().unwrap();
        let capabilities = profile.security_capabilities();
        assert_eq!(capabilities.capability_count, 2);
        let actual = unsafe { std::slice::from_raw_parts(
            capabilities.capabilities as *const SidAndAttributes, 2) };
        assert_eq!(actual[0].sid, profile.internet_sid.as_ref().unwrap().0);
        assert_eq!(actual[1].sid, profile.registry_capability.as_ref().unwrap().sid);
        assert!(actual.iter().all(|capability| capability.attributes == SE_GROUP_ENABLED));
        let production = AppContainerProfile::ensure("Gogoke37.ProductionCapability", false).unwrap();
        assert_eq!(production.security_capabilities().capability_count, 1);
        assert!(production.registry_capability.is_some());
        let cli = AppContainerProfile::ensure_for_cli("Gogoke37.CliCapability", false).unwrap();
        let cli_capabilities = cli.security_capabilities();
        assert_eq!(cli_capabilities.capability_count, 2);
        let cli_actual = unsafe { std::slice::from_raw_parts(
            cli_capabilities.capabilities as *const SidAndAttributes, 2) };
        assert_eq!(cli_actual[0].sid, cli.registry_capability.as_ref().unwrap().sid);
        assert_eq!(cli_actual[1].sid, unsafe { *cli.identity_services_sids.as_ref().unwrap().capability_sids });
        assert!(cli_actual.iter().all(|capability| capability.attributes == SE_GROUP_ENABLED));
        let cli_networked = AppContainerProfile::ensure_for_cli("Gogoke37.CliNetworkCapability", true).unwrap();
        assert_eq!(cli_networked.security_capabilities().capability_count, 3);
        assert_eq!(cli_networked.combined_capabilities[0].sid,
            cli_networked.internet_sid.as_ref().unwrap().0);
    }
}
