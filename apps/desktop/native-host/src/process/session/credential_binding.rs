//! Metadata-only custody for the fixed Codex File backend's one auth object.
//! F supplies the durable source identity and complete alias set. This module
//! never opens a credential stream for data and never treats an Arc as a row.

use super::DirectoryRoots;
use super::isolation::IsolationError;
use crate::root::{RootIdentity, RootLock};
use std::ffi::{c_void, OsStr};
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

const READ_ATTRIBUTES: u32 = 0x80;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const DELETE_ACCESS: u32 = 0x0001_0000;
const SHARE_READ_WRITE: u32 = 3;
const SHARE_ALL: u32 = 7;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const REPARSE: u32 = 0x400;
const DIRECTORY: u32 = 0x10;
const AUTH_NAME: &str = "auth.json";

#[repr(C)]
struct FileIdInfo { volume_serial: u64, file_id: [u8; 16] }

#[repr(C)]
struct FileInfo {
    attributes: u32,
    times: [u32; 6],
    volume_serial: u32,
    size_high: u32,
    size_low: u32,
    links: u32,
    index_high: u32,
    index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetFileInformationByHandleEx(handle: *mut c_void, class: i32,
        output: *mut c_void, length: u32) -> i32;
    fn GetFileInformationByHandle(handle: *mut c_void, output: *mut FileInfo) -> i32;
    fn CreateHardLinkW(new_name: *const u16, existing_name: *const u16,
        security: *const c_void) -> i32;
}

#[derive(Debug)]
pub(crate) enum CredentialError {
    Io { operation: &'static str, source: io::Error },
    Invalid(&'static str),
    IdentityChanged,
    LinkCount { expected: u32, observed: u32 },
    CustodyPoisoned,
    Isolation(Box<IsolationError>),
}

impl std::fmt::Display for CredentialError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { operation, source } => write!(f, "credential {operation}: {source}"),
            Self::Invalid(reason) => write!(f, "credential metadata: {reason}"),
            Self::IdentityChanged => write!(f, "credential physical identity changed"),
            Self::LinkCount { expected, observed } =>
                write!(f, "credential registered link count expected={expected} observed={observed}"),
            Self::CustodyPoisoned => write!(f, "credential object custody poisoned"),
            Self::Isolation(error) => write!(f, "credential ACL: {error}"),
        }
    }
}

impl From<IsolationError> for CredentialError {
    fn from(error: IsolationError) -> Self { Self::Isolation(Box::new(error)) }
}

fn io_error(operation: &'static str, source: io::Error) -> CredentialError {
    CredentialError::Io { operation, source }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialAliasScope {
    pub(crate) root: PathBuf,
    pub(crate) root_identity: RootIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialAlias {
    scope: CredentialAliasScope,
    file_identity: RootIdentity,
}

impl CredentialAlias {
    pub(crate) fn path(&self) -> PathBuf { self.scope.root.join(AUTH_NAME) }
    pub(crate) fn root(&self) -> &Path { &self.scope.root }
    pub(crate) fn root_identity(&self) -> &RootIdentity { &self.scope.root_identity }
    pub(crate) fn file_identity(&self) -> &RootIdentity { &self.file_identity }
}

struct HeldAlias {
    witness: CredentialAlias,
    parents: DirectoryRoots,
}

struct BindingState {
    aliases: Vec<HeldAlias>,
    acl_prepared: bool,
}

pub(crate) struct CredentialBinding {
    source: PathBuf,
    source_parent_identity: RootIdentity,
    identity: RootIdentity,
    file: File,
    source_parents: DirectoryRoots,
    state: Mutex<BindingState>,
}

struct CachedBinding { identity: RootIdentity, binding: Weak<CredentialBinding> }
static CREDENTIAL_CUSTODY: OnceLock<Mutex<Vec<CachedBinding>>> = OnceLock::new();
fn custody() -> &'static Mutex<Vec<CachedBinding>> {
    CREDENTIAL_CUSTODY.get_or_init(|| Mutex::new(Vec::new()))
}

fn physical_info(file: &File, operation: &'static str) -> Result<(RootIdentity, u32), CredentialError> {
    let metadata = file.metadata().map_err(|error| io_error(operation, error))?;
    if metadata.file_attributes() & (REPARSE | DIRECTORY) != 0 || !metadata.is_file() {
        return Err(CredentialError::Invalid("not an ordinary non-reparse file"));
    }
    let mut id = FileIdInfo { volume_serial: 0, file_id: [0; 16] };
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), 18,
        (&mut id as *mut FileIdInfo).cast(), std::mem::size_of::<FileIdInfo>() as u32) } == 0 {
        return Err(io_error(operation, io::Error::last_os_error()));
    }
    let mut info: FileInfo = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
        return Err(io_error(operation, io::Error::last_os_error()));
    }
    if info.attributes & (REPARSE | DIRECTORY) != 0 || info.links == 0 {
        return Err(CredentialError::Invalid("credential file attributes or links"));
    }
    Ok((RootIdentity { volume_serial: id.volume_serial, file_id: id.file_id }, info.links))
}

fn open_metadata(path: &Path, operation: &'static str) -> Result<File, CredentialError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| io_error(operation, error))?;
    if metadata.file_attributes() & (REPARSE | DIRECTORY) != 0 || !metadata.is_file() {
        return Err(CredentialError::Invalid("credential name is not an ordinary file"));
    }
    OpenOptions::new().access_mode(READ_ATTRIBUTES | READ_CONTROL)
        .share_mode(SHARE_ALL).custom_flags(OPEN_REPARSE_POINT)
        .open(path).map_err(|error| io_error(operation, error))
}

fn alias_path(scope: &CredentialAliasScope) -> Result<PathBuf, CredentialError> {
    if !scope.root.is_absolute() { return Err(CredentialError::Invalid("alias root is not absolute")); }
    Ok(scope.root.join(AUTH_NAME))
}

impl CredentialBinding {
    /// F's initial observation can persist a physical identity without
    /// opening or hashing credential contents. The same entry also works
    /// after restart when the object legitimately has registered aliases.
    pub(crate) fn observe_source_metadata(root: &RootLock, source: &Path,
        source_parent_identity: &RootIdentity) -> Result<(RootIdentity, u32), CredentialError> {
        if source.file_name() != Some(OsStr::new(AUTH_NAME)) {
            return Err(CredentialError::Invalid("source is not fixed Codex/File auth name"));
        }
        let parent = source.parent().ok_or(CredentialError::Invalid("source parent absent"))?;
        let held = DirectoryRoots::prepare(root,
            &[(parent.to_path_buf(), source_parent_identity.clone())])
            .map_err(|error| io_error("hold observed source parent", error))?;
        let file = open_metadata(source, "observe source metadata")?;
        let observed = physical_info(&file, "observe source file ID and links")?;
        held.verify().map_err(|error| io_error("reverify observed source parent", error))?;
        Ok(observed)
    }

    /// Only F's registered Codex/File source and its complete recorded aliases
    /// may enter. Every call rechecks the names, physical object and link count.
    pub(crate) fn open_registered(root: &RootLock, source: &Path,
        source_parent_identity: &RootIdentity, expected: &RootIdentity,
        registered_aliases: &[CredentialAliasScope]) -> Result<Arc<Self>, CredentialError> {
        if source.file_name() != Some(OsStr::new(AUTH_NAME)) {
            return Err(CredentialError::Invalid("source is not fixed Codex/File auth name"));
        }
        let parent = source.parent().ok_or(CredentialError::Invalid("source parent absent"))?;
        let source_parents = DirectoryRoots::prepare(root,
            &[(parent.to_path_buf(), source_parent_identity.clone())])
            .map_err(|error| io_error("hold registered parent", error))?;
        let observed = open_metadata(source, "open registered source metadata")?;
        let (identity, _) = physical_info(&observed, "registered source identity")?;
        if &identity != expected { return Err(CredentialError::IdentityChanged); }
        let mut cache = custody().lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        cache.retain(|entry| entry.binding.strong_count() != 0);
        if let Some(entry) = cache.iter().find(|entry| entry.identity == identity) {
            let binding = entry.binding.upgrade().ok_or(CredentialError::CustodyPoisoned)?;
            if binding.source != source || binding.source_parent_identity != *source_parent_identity {
                return Err(CredentialError::IdentityChanged);
            }
            binding.verify_registered_aliases(registered_aliases)?;
            return Ok(binding);
        }
        // One DELETE-owning/no-share-delete file handle pins the registered
        // object across all H generations. It requests no credential data bits.
        let file = OpenOptions::new()
            .access_mode(READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | DELETE_ACCESS)
            .share_mode(SHARE_READ_WRITE).custom_flags(OPEN_REPARSE_POINT)
            .open(source).map_err(|error| io_error("hold registered source namespace", error))?;
        let (held_identity, _) = physical_info(&file, "held source identity")?;
        if held_identity != identity { return Err(CredentialError::IdentityChanged); }
        let binding = Arc::new(Self { source: source.to_path_buf(),
            source_parent_identity: source_parent_identity.clone(), identity,
            file, source_parents, state: Mutex::new(BindingState {
                aliases: Vec::new(), acl_prepared: false }) });
        binding.install_registered_aliases(root, registered_aliases)?;
        cache.push(CachedBinding { identity: binding.identity.clone(), binding: Arc::downgrade(&binding) });
        Ok(binding)
    }

    fn install_registered_aliases(&self, root: &RootLock,
        aliases: &[CredentialAliasScope]) -> Result<(), CredentialError> {
        let mut state = self.state.lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        if !state.aliases.is_empty() { return Err(CredentialError::Invalid("alias custody already installed")); }
        let mut held = Vec::with_capacity(aliases.len());
        for scope in aliases {
            if held.iter().any(|entry: &HeldAlias| entry.witness.scope == *scope) {
                return Err(CredentialError::Invalid("duplicate registered alias"));
            }
            let parents = DirectoryRoots::prepare(root,
                &[(scope.root.clone(), scope.root_identity.clone())])
                .map_err(|error| io_error("hold registered alias parent", error))?;
            let path = alias_path(scope)?;
            let file = open_metadata(&path, "open registered alias metadata")?;
            let (identity, _) = physical_info(&file, "registered alias identity")?;
            if identity != self.identity { return Err(CredentialError::IdentityChanged); }
            held.push(HeldAlias { witness: CredentialAlias {
                scope: scope.clone(), file_identity: identity }, parents });
        }
        self.check_link_count(aliases.len())?;
        state.aliases = held;
        Ok(())
    }

    fn check_link_count(&self, aliases: usize) -> Result<(), CredentialError> {
        let (identity, links) = physical_info(&self.file, "held source link count")?;
        if identity != self.identity { return Err(CredentialError::IdentityChanged); }
        let expected = u32::try_from(aliases).ok().and_then(|value| value.checked_add(1))
            .ok_or(CredentialError::Invalid("registered alias count overflow"))?;
        if links != expected { return Err(CredentialError::LinkCount { expected, observed: links }); }
        Ok(())
    }

    fn verify_locked(&self, state: &BindingState,
        aliases: &[CredentialAliasScope]) -> Result<(), CredentialError> {
        if aliases.iter().enumerate().any(|(index, scope)| aliases[..index].contains(scope)) {
            return Err(CredentialError::Invalid("duplicate alias in complete registry"));
        }
        if state.aliases.len() != aliases.len() || aliases.iter().any(|scope|
            !state.aliases.iter().any(|entry| &entry.witness.scope == scope)) {
            return Err(CredentialError::Invalid("incomplete registered alias set"));
        }
        self.source_parents.verify().map_err(|error| io_error("verify source parent", error))?;
        let source = open_metadata(&self.source, "reopen registered source metadata")?;
        if physical_info(&source, "recheck registered source identity")?.0 != self.identity {
            return Err(CredentialError::IdentityChanged);
        }
        for entry in &state.aliases {
            entry.parents.verify().map_err(|error| io_error("verify alias parent", error))?;
            let file = open_metadata(&entry.witness.path(), "reopen registered alias metadata")?;
            if physical_info(&file, "recheck registered alias identity")?.0 != self.identity {
                return Err(CredentialError::IdentityChanged);
            }
        }
        self.check_link_count(aliases.len())
    }

    pub(crate) fn verify_registered_aliases(&self,
        aliases: &[CredentialAliasScope]) -> Result<(), CredentialError> {
        let state = self.state.lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        self.verify_locked(&state, aliases)
    }

    /// F must persist its pending intent before this call. A failed post-create
    /// check leaves the new name in place for F's UNKNOWN reconciliation.
    pub(crate) fn create_alias(&self, root: &RootLock, scope: CredentialAliasScope,
        registered_aliases: &[CredentialAliasScope]) -> Result<CredentialAlias, CredentialError> {
        let mut state = self.state.lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        self.verify_locked(&state, registered_aliases)?;
        if state.aliases.iter().any(|entry| entry.witness.scope == scope) {
            return Err(CredentialError::Invalid("alias already registered"));
        }
        let parents = DirectoryRoots::prepare(root,
            &[(scope.root.clone(), scope.root_identity.clone())])
            .map_err(|error| io_error("hold new alias parent", error))?;
        let path = alias_path(&scope)?;
        match std::fs::symlink_metadata(&path) {
            Ok(_) => return Err(CredentialError::Invalid("alias name already exists")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(io_error("check absent alias name", error)),
        }
        let new_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let source_wide: Vec<u16> = self.source.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe { CreateHardLinkW(new_wide.as_ptr(), source_wide.as_ptr(),
            std::ptr::null()) } == 0 {
            return Err(io_error("CreateHardLinkW exact alias", io::Error::last_os_error()));
        }
        let new_file = open_metadata(&path, "open created alias metadata")?;
        let (identity, _) = physical_info(&new_file, "created alias identity")?;
        if identity != self.identity { return Err(CredentialError::IdentityChanged); }
        self.check_link_count(state.aliases.len() + 1)?;
        let witness = CredentialAlias { scope, file_identity: identity };
        state.aliases.push(HeldAlias { witness: witness.clone(), parents });
        Ok(witness)
    }

    pub(crate) fn alias(&self, scope: &CredentialAliasScope,
        registered_aliases: &[CredentialAliasScope]) -> Result<CredentialAlias, CredentialError> {
        let state = self.state.lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        self.verify_locked(&state, registered_aliases)?;
        state.aliases.iter().find(|entry| &entry.witness.scope == scope)
            .map(|entry| entry.witness.clone())
            .ok_or(CredentialError::Invalid("alias not in complete registry"))
    }

    pub(super) fn with_exact_alias<T>(&self, alias: &CredentialAlias,
        action: impl FnOnce(*mut c_void) -> Result<T, CredentialError>) -> Result<T, CredentialError> {
        let state = self.state.lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        let scopes: Vec<_> = state.aliases.iter().map(|entry| entry.witness.scope.clone()).collect();
        self.verify_locked(&state, &scopes)?;
        if alias.file_identity != self.identity || !state.aliases.iter().any(|entry|
            entry.witness == *alias) { return Err(CredentialError::IdentityChanged); }
        let result = action(self.file.as_raw_handle())?;
        self.verify_locked(&state, &scopes)?;
        Ok(result)
    }

    pub(super) fn with_source_acl<T>(&self,
        action: impl FnOnce(*mut c_void, bool) -> Result<T, CredentialError>) -> Result<T, CredentialError> {
        let mut state = self.state.lock().map_err(|_| CredentialError::CustodyPoisoned)?;
        let scopes: Vec<_> = state.aliases.iter().map(|entry| entry.witness.scope.clone()).collect();
        self.verify_locked(&state, &scopes)?;
        let result = action(self.file.as_raw_handle(), state.acl_prepared)?;
        state.acl_prepared = true;
        self.verify_locked(&state, &scopes)?;
        Ok(result)
    }

    pub(crate) fn identity(&self) -> &RootIdentity { &self.identity }

    #[cfg(test)]
    fn denied_data_read_for_test(&self) -> io::Result<()> {
        #[link(name = "kernel32")]
        extern "system" {
            fn ReadFile(handle: *mut c_void, buffer: *mut c_void, length: u32,
                read: *mut u32, overlapped: *mut c_void) -> i32;
        }
        let mut byte = 0u8;
        let mut read = 0u32;
        if unsafe { ReadFile(self.file.as_raw_handle(), (&mut byte as *mut u8).cast(),
            1, &mut read, std::ptr::null_mut()) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Err(io::Error::new(io::ErrorKind::Other,
            format!("metadata-only credential handle unexpectedly read {read} bytes")))
    }
}

#[cfg(test)]
#[path = "credential_binding_tests.rs"]
mod tests;
