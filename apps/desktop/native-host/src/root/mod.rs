//! Windows physical-root identity and process exclusion.
//!
//! The directory handle is the source of truth. Path text is retained only for
//! diagnostics; aliases and junctions are collapsed by `FileIdInfo` before the
//! named OS mutex is selected.

use std::fmt;
use std::io;
use std::path::PathBuf;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RootIdentity {
    pub volume_serial: u64,
    pub file_id: [u8; 16],
}

static POISONED_ROOTS: OnceLock<Mutex<HashSet<RootIdentity>>> = OnceLock::new();

fn poisoned_roots() -> &'static Mutex<HashSet<RootIdentity>> {
    POISONED_ROOTS.get_or_init(|| Mutex::new(HashSet::new()))
}

fn root_is_poisoned(identity: &RootIdentity) -> bool {
    poisoned_roots()
        .lock()
        .expect("poisoned-root registry")
        .contains(identity)
}

fn mark_root_poisoned(identity: &RootIdentity) {
    poisoned_roots()
        .lock()
        .expect("poisoned-root registry")
        .insert(identity.clone());
}

impl RootIdentity {
    pub fn opaque(&self) -> String {
        format!(
            "volume:{:016x}/file:{}",
            self.volume_serial,
            hex_bytes(&self.file_id)
        )
    }

    pub fn lock_name(&self) -> String {
        format!(
            "Global\\Gogoke.Root.v1.{:016x}.{}",
            self.volume_serial,
            hex_bytes(&self.file_id)
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalRoot {
    pub requested_path: PathBuf,
    pub canonical_path: PathBuf,
    pub identity: RootIdentity,
}

#[derive(Debug)]
pub enum RootLockError {
    MissingRoot,
    RootMustBeAbsolute(PathBuf),
    UnsupportedRoot {
        path: PathBuf,
        reason: &'static str,
    },
    OpenRoot {
        path: PathBuf,
        source: io::Error,
    },
    RootNotDirectory(PathBuf),
    InspectRoot {
        operation: &'static str,
        source: io::Error,
    },
    AlreadyLocked {
        identity: RootIdentity,
    },
    Poisoned {
        identity: RootIdentity,
    },
    RootIdentityChanged {
        observed: RootIdentity,
        bound: RootIdentity,
    },
    RootPathChanged,
    CreateLock {
        name: String,
        source: io::Error,
    },
    HandlePolicy {
        operation: &'static str,
        source: io::Error,
    },
    LockThreadStopped,
}

#[derive(Debug)]
pub enum DatabasePinError {
    InvalidPath(RootLockError),
    ParentRequired(PathBuf),
    UnsafeChildName(PathBuf),
    OutsideRoot {
        path: PathBuf,
        expected: RootIdentity,
        observed: RootIdentity,
    },
    OpenFile {
        path: PathBuf,
        operation: &'static str,
        source: io::Error,
    },
    AlreadyExists(PathBuf),
    ReparsePoint(PathBuf),
    NotRegularFile(PathBuf),
    InspectFile {
        operation: &'static str,
        source: io::Error,
    },
    HandlePolicy {
        source: io::Error,
    },
    UnsupportedPlatform,
}

impl fmt::Display for DatabasePinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath(error) => write!(f, "DATABASE_PATH_INVALID: {error}"),
            Self::ParentRequired(path) => {
                write!(f, "DATABASE_PARENT_REQUIRED: {}", path.display())
            }
            Self::UnsafeChildName(path) => {
                write!(f, "DATABASE_CHILD_NAME_UNSAFE: {}", path.display())
            }
            Self::OutsideRoot {
                path,
                expected,
                observed,
            } => write!(
                f,
                "DATABASE_OUTSIDE_ROOT: {} parent is {}, expected {}",
                path.display(),
                observed.opaque(),
                expected.opaque()
            ),
            Self::OpenFile {
                path,
                operation,
                source,
            } => write!(
                f,
                "DATABASE_OPEN_FAILED: {operation} {}: {source}",
                path.display()
            ),
            Self::AlreadyExists(path) => {
                write!(f, "DATABASE_ALREADY_EXISTS: {}", path.display())
            }
            Self::ReparsePoint(path) => {
                write!(f, "DATABASE_REPARSE_REJECTED: {}", path.display())
            }
            Self::NotRegularFile(path) => {
                write!(f, "DATABASE_NOT_REGULAR_FILE: {}", path.display())
            }
            Self::InspectFile { operation, source } => {
                write!(f, "DATABASE_IDENTITY_FAILED: {operation}: {source}")
            }
            Self::HandlePolicy { source } => {
                write!(f, "DATABASE_HANDLE_POLICY_FAILED: {source}")
            }
            Self::UnsupportedPlatform => write!(f, "DATABASE_PIN_UNSUPPORTED_PLATFORM"),
        }
    }
}

/// SQLite sidecar next to a main database. Inspected separately from the
/// main-file pin: pinning `core.db` never qualifies `-wal` or `-shm`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DatabaseSidecarKind {
    Wal,
    Shm,
}

impl DatabaseSidecarKind {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Wal => "-wal",
            Self::Shm => "-shm",
        }
    }

    fn header_bytes(self) -> u64 {
        match self {
            Self::Wal => 32,
            Self::Shm => 32_768,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DatabaseSidecarState {
    Absent,
    Present {
        identity: RootIdentity,
        size: u64,
        truncated_header: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatabaseSidecarReport {
    pub main_identity: RootIdentity,
    pub wal: DatabaseSidecarState,
    pub shm: DatabaseSidecarState,
}

impl std::error::Error for DatabasePinError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidPath(error) => Some(error),
            Self::OpenFile { source, .. }
            | Self::InspectFile { source, .. }
            | Self::HandlePolicy { source } => Some(source),
            _ => None,
        }
    }
}

impl fmt::Display for RootLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRoot => write!(f, "ROOT_PATH_REQUIRED: an explicit root path is required"),
            Self::RootMustBeAbsolute(path) => write!(
                f,
                "ROOT_PATH_NOT_ABSOLUTE: refusing cwd-relative root {}",
                path.display()
            ),
            Self::UnsupportedRoot { path, reason } => {
                write!(f, "ROOT_UNSUPPORTED: {} ({reason})", path.display())
            }
            Self::OpenRoot { path, source } => {
                write!(f, "ROOT_OPEN_FAILED: {}: {source}", path.display())
            }
            Self::RootNotDirectory(path) => {
                write!(f, "ROOT_NOT_DIRECTORY: {}", path.display())
            }
            Self::InspectRoot { operation, source } => {
                write!(f, "ROOT_IDENTITY_FAILED: {operation}: {source}")
            }
            Self::AlreadyLocked { identity } => write!(
                f,
                "ROOT_ALREADY_LOCKED: {} already has a writer",
                identity.opaque()
            ),
            Self::Poisoned { identity } => write!(
                f,
                "ROOT_POISONED: {} has unresolved native custody; refusing reuse",
                identity.opaque()
            ),
            Self::RootIdentityChanged { observed, bound } => write!(
                f,
                "ROOT_IDENTITY_CHANGED: observed {} but bound {}; refusing write authority",
                observed.opaque(),
                bound.opaque()
            ),
            Self::RootPathChanged => write!(
                f,
                "ROOT_PATH_CHANGED: root path kind changed while binding; refusing write authority"
            ),
            Self::CreateLock { name, source } => {
                write!(f, "ROOT_LOCK_FAILED: {name}: {source}")
            }
            Self::HandlePolicy { operation, source } => {
                write!(f, "ROOT_HANDLE_POLICY_FAILED: {operation}: {source}")
            }
            Self::LockThreadStopped => write!(f, "ROOT_LOCK_THREAD_STOPPED"),
        }
    }
}

impl std::error::Error for RootLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OpenRoot { source, .. }
            | Self::InspectRoot { source, .. }
            | Self::CreateLock { source, .. }
            | Self::HandlePolicy { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{
        mark_root_poisoned, root_is_poisoned, CanonicalRoot, DatabasePinError, DatabaseSidecarKind,
        DatabaseSidecarReport, DatabaseSidecarState, RootIdentity, RootLockError,
    };
    #[cfg(test)]
    use std::cell::Cell;
    use std::ffi::{c_void, OsStr, OsString};
    use std::io;
    use std::marker::PhantomData;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Component, Path, PathBuf};
    use std::ptr;
    use std::sync::mpsc;
    use std::thread;

    type Handle = *mut c_void;

    #[cfg(test)]
    thread_local! {
        static WIDE_CALL_COUNT: Cell<usize> = const { Cell::new(0) };
    }

    const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;
    const GENERIC_READ: u32 = 0x8000_0000;
    const GENERIC_WRITE: u32 = 0x4000_0000;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const DELETE_ACCESS: u32 = 0x0001_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const CREATE_NEW: u32 = 1;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_STANDARD_INFO_CLASS: i32 = 1;
    const FILE_ATTRIBUTE_TAG_INFO_CLASS: i32 = 9;
    const FILE_ID_INFO_CLASS: i32 = 18;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;
    const ERROR_ALREADY_EXISTS: i32 = 183;
    const ERROR_FILE_EXISTS: i32 = 80;
    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_ABANDONED: u32 = 0x0000_0080;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;
    const WAIT_FAILED: u32 = 0xffff_ffff;
    const DRIVE_UNKNOWN: u32 = 0;
    const DRIVE_NO_ROOT_DIR: u32 = 1;
    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;
    const DRIVE_RAMDISK: u32 = 6;

    #[repr(C)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }

    #[repr(C)]
    struct FileId128 {
        identifier: [u8; 16],
    }

    #[repr(C)]
    struct FileIdInfo {
        volume_serial_number: u64,
        file_id: FileId128,
    }

    #[repr(C)]
    struct FileAttributeTagInfo {
        file_attributes: u32,
        reparse_tag: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            file_name: *const u16,
            desired_access: u32,
            share_mode: u32,
            security_attributes: *const c_void,
            creation_disposition: u32,
            flags_and_attributes: u32,
            template_file: Handle,
        ) -> Handle;
        fn GetFileInformationByHandleEx(
            file: Handle,
            info_class: i32,
            info: *mut c_void,
            size: u32,
        ) -> i32;
        fn GetFinalPathNameByHandleW(
            file: Handle,
            path: *mut u16,
            path_len: u32,
            flags: u32,
        ) -> u32;
        fn GetVolumePathNameW(
            file_name: *const u16,
            volume_path_name: *mut u16,
            buffer_len: u32,
        ) -> i32;
        fn GetDriveTypeW(root_path_name: *const u16) -> u32;
        fn CreateMutexW(
            mutex_attributes: *const c_void,
            initial_owner: i32,
            name: *const u16,
        ) -> Handle;
        fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        fn ReleaseMutex(mutex: Handle) -> i32;
        fn SetHandleInformation(object: Handle, mask: u32, flags: u32) -> i32;
        fn GetHandleInformation(object: Handle, flags: *mut u32) -> i32;
        fn CloseHandle(object: Handle) -> i32;
        fn GetLastError() -> u32;
    }

    struct OwnedHandle(Handle);

    impl OwnedHandle {
        fn new(handle: Handle) -> Option<Self> {
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                None
            } else {
                Some(Self(handle))
            }
        }

        fn raw(&self) -> Handle {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    impl OwnedHandle {
        fn disarm(&mut self) {
            self.0 = INVALID_HANDLE_VALUE;
        }
    }

    // A HANDLE is an opaque kernel reference. This wrapper owns one reference,
    // and CloseHandle is valid from any thread.
    unsafe impl Send for OwnedHandle {}

    struct OpenRoot {
        canonical: CanonicalRoot,
        identity_handle: OwnedHandle,
        path_binding_handle: Option<OwnedHandle>,
    }

    enum LockThreadCommand {
        Release,
    }

    struct LockThreadReady {
        handle_inheritable: bool,
    }

    pub struct RootLock {
        canonical: CanonicalRoot,
        identity_handle: OwnedHandle,
        path_binding_handle: Option<OwnedHandle>,
        lock_handle_inheritable: bool,
        release_tx: Option<mpsc::SyncSender<LockThreadCommand>>,
        lock_thread: Option<thread::JoinHandle<()>>,
        // Root authority is thread-affine.  The native SQLite handoff is
        // bound and consumed by the same custodian thread; keeping a raw
        // marker here makes RootLock (and handles borrowing it) !Send/!Sync,
        // so a foreign thread cannot steal the root or release it through a
        // safe Rust move/drop path.
        _thread_affine: PhantomData<*mut ()>,
    }

    /// Pins one direct-child database object against rename, deletion, and
    /// replacement. Concurrent database/VFS handles must include
    /// FILE_SHARE_DELETE because this pin intentionally owns DELETE access;
    /// private IPC wiring must verify that handshake before opening SQLite.
    pub struct DatabaseFilePin<'root> {
        authoritative_path: PathBuf,
        identity: RootIdentity,
        root_identity: RootIdentity,
        created_new: bool,
        handle: OwnedHandle,
        _root_lock: PhantomData<&'root RootLock>,
    }

    /// A validated direct-child database handle prepared for the Route-B
    /// same-open handoff.  The native SQLite VFS consumes the OS handle only
    /// after the caller has bound the exact canonical path and root identity.
    pub(crate) struct SqliteMainHandle<'root> {
        authoritative_path: PathBuf,
        identity: RootIdentity,
        root_identity: RootIdentity,
        created_new: bool,
        handle: OwnedHandle,
        _root_lock: PhantomData<&'root RootLock>,
    }

    impl SqliteMainHandle<'_> {
        pub(crate) fn raw(&self) -> *mut c_void {
            self.handle.raw()
        }

        pub(crate) fn path(&self) -> &Path {
            &self.authoritative_path
        }

        pub(crate) fn identity(&self) -> &RootIdentity {
            &self.identity
        }

        pub(crate) fn root_identity(&self) -> &RootIdentity {
            &self.root_identity
        }

        pub(crate) fn created_new(&self) -> bool {
            self.created_new
        }

        /// Disarm the Rust close path after the C VFS has adopted this exact
        /// handle.  The root-lock lifetime marker remains held by the store.
        pub(crate) fn mark_transferred(&mut self) {
            self.handle.disarm();
        }
    }

    impl<'root> DatabaseFilePin<'root> {
        pub fn path(&self) -> &Path {
            &self.authoritative_path
        }

        pub fn identity(&self) -> &RootIdentity {
            &self.identity
        }

        pub fn created_new(&self) -> bool {
            self.created_new
        }

        pub fn handle_inheritable(&self) -> Result<bool, DatabasePinError> {
            database_handle_is_inheritable(self.handle.raw())
        }

        pub fn verify_path_identity(&self) -> Result<bool, DatabasePinError> {
            let verification = open_file_handle(
                &self.authoritative_path,
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                "verify pinned database path",
            )?;
            let observed = inspect_database_file(&self.authoritative_path, &verification)?;
            Ok(observed == self.identity)
        }

        pub(crate) fn into_sqlite_main_handle(self) -> SqliteMainHandle<'root> {
            let DatabaseFilePin {
                authoritative_path,
                identity,
                root_identity,
                created_new,
                handle,
                _root_lock,
            } = self;
            SqliteMainHandle {
                authoritative_path,
                identity,
                root_identity,
                created_new,
                handle,
                _root_lock,
            }
        }
    }

    impl RootLock {
        pub fn acquire(path: &Path) -> Result<Self, RootLockError> {
            Self::acquire_after_precheck(path, |_| {})
        }

        pub(super) fn acquire_after_precheck(
            path: &Path,
            after_precheck: impl FnOnce(&RootIdentity),
        ) -> Result<Self, RootLockError> {
            // The shared preflight lets a contender identify an already-held
            // physical root. The mutex is acquired before the exclusive path
            // binding, then the identity is read again from the pinned handle.
            // Any retarget in that interval fails closed instead of changing
            // which object the mutex protects.
            let observed = inspect_root(path)?;
            if root_is_poisoned(&observed.identity) {
                return Err(RootLockError::Poisoned {
                    identity: observed.identity,
                });
            }
            after_precheck(&observed.identity);
            let identity = observed.identity.clone();
            let name = identity.lock_name();
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let (release_tx, release_rx) = mpsc::sync_channel(1);
            let lock_thread = thread::Builder::new()
                .name("gogoke-root-lock".to_owned())
                .spawn(move || {
                    let result = acquire_named_mutex(&name, &identity);
                    match result {
                        Ok((mutex, ready)) => {
                            if ready_tx.send(Ok(ready)).is_ok() {
                                let _ = release_rx.recv();
                            }
                            unsafe {
                                ReleaseMutex(mutex.raw());
                            }
                        }
                        Err(error) => {
                            let _ = ready_tx.send(Err(error));
                        }
                    }
                })
                .map_err(|source| RootLockError::CreateLock {
                    name: observed.identity.lock_name(),
                    source,
                })?;

            let ready = match ready_rx.recv() {
                Ok(Ok(ready)) => ready,
                Ok(Err(error)) => {
                    let _ = lock_thread.join();
                    return Err(error);
                }
                Err(_) => {
                    let _ = lock_thread.join();
                    return Err(RootLockError::LockThreadStopped);
                }
            };

            let open = match open_root(path) {
                Ok(open) => open,
                Err(error) => {
                    let _ = release_tx.send(LockThreadCommand::Release);
                    let _ = lock_thread.join();
                    return Err(error);
                }
            };
            if open.canonical.identity != observed.identity {
                let error = RootLockError::RootIdentityChanged {
                    observed: observed.identity,
                    bound: open.canonical.identity,
                };
                let _ = release_tx.send(LockThreadCommand::Release);
                let _ = lock_thread.join();
                return Err(error);
            }

            // The first check is only a fast rejection. A prior holder can
            // publish poison while this contender waits for the OS mutex;
            // recheck the bound physical identity before publishing the lock.
            if root_is_poisoned(&open.canonical.identity) {
                let identity = open.canonical.identity.clone();
                let _ = release_tx.send(LockThreadCommand::Release);
                let _ = lock_thread.join();
                return Err(RootLockError::Poisoned { identity });
            }

            Ok(Self {
                canonical: open.canonical,
                identity_handle: open.identity_handle,
                path_binding_handle: open.path_binding_handle,
                lock_handle_inheritable: ready.handle_inheritable,
                release_tx: Some(release_tx),
                lock_thread: Some(lock_thread),
                _thread_affine: PhantomData,
            })
        }

        pub fn canonical_root(&self) -> &CanonicalRoot {
            &self.canonical
        }

        /// Permanently reject this physical root for the lifetime of the
        /// process after a native close result becomes UNKNOWN or otherwise
        /// fails. There is intentionally no in-process clear operation: the
        /// unresolved handle ledger must be resolved by process restart or an
        /// explicitly reviewed recovery path.
        pub(crate) fn poison_identity(identity: &RootIdentity) {
            mark_root_poisoned(identity);
        }

        pub fn handles_are_non_inheritable(&self) -> Result<bool, RootLockError> {
            Ok(!self.root_handle_inheritable()? && !self.lock_handle_inheritable())
        }

        pub fn root_handle_inheritable(&self) -> Result<bool, RootLockError> {
            if handle_is_inheritable(self.identity_handle.raw())? {
                return Ok(true);
            }
            match &self.path_binding_handle {
                Some(handle) => handle_is_inheritable(handle.raw()),
                None => Ok(false),
            }
        }

        pub fn lock_handle_inheritable(&self) -> bool {
            self.lock_handle_inheritable
        }

        pub fn pin_existing_database<'root>(
            &'root self,
            path: &Path,
        ) -> Result<DatabaseFilePin<'root>, DatabasePinError> {
            pin_database_file(self, path, OPEN_EXISTING, false)
        }

        pub fn create_and_pin_database<'root>(
            &'root self,
            path: &Path,
        ) -> Result<DatabaseFilePin<'root>, DatabasePinError> {
            pin_database_file(self, path, CREATE_NEW, true)
        }

        /// Inspect WAL/SHM next to a pinned or unpinned main file.
        /// Does not pin, delete, or repair sidecars. Crash residue is reported
        /// as Present and left on disk.
        pub fn inspect_database_sidecars(
            &self,
            main_path: &Path,
        ) -> Result<DatabaseSidecarReport, DatabasePinError> {
            let authoritative = prepare_database_path(self, main_path)?;
            let main = open_file_handle(
                &authoritative,
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                "inspect main before sidecar report",
            )?;
            let main_identity = inspect_database_file(&authoritative, &main)?;
            let wal = inspect_one_sidecar(&authoritative, DatabaseSidecarKind::Wal)?;
            let shm = inspect_one_sidecar(&authoritative, DatabaseSidecarKind::Shm)?;
            if let DatabaseSidecarState::Present { identity, .. } = &wal {
                if *identity == main_identity {
                    return Err(DatabasePinError::NotRegularFile(sidecar_path(
                        &authoritative,
                        DatabaseSidecarKind::Wal,
                    )));
                }
            }
            if let DatabaseSidecarState::Present { identity, .. } = &shm {
                if *identity == main_identity {
                    return Err(DatabasePinError::NotRegularFile(sidecar_path(
                        &authoritative,
                        DatabaseSidecarKind::Shm,
                    )));
                }
            }
            Ok(DatabaseSidecarReport {
                main_identity,
                wal,
                shm,
            })
        }
    }

    impl Drop for RootLock {
        fn drop(&mut self) {
            if let Some(release_tx) = self.release_tx.take() {
                let _ = release_tx.send(LockThreadCommand::Release);
            }
            if let Some(lock_thread) = self.lock_thread.take() {
                let _ = lock_thread.join();
            }
        }
    }

    pub fn inspect_root(path: &Path) -> Result<CanonicalRoot, RootLockError> {
        validate_explicit_path(path)?;
        reject_unsupported_volume(path)?;
        let handle = open_directory_handle(
            path,
            FILE_FLAG_BACKUP_SEMANTICS,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            "root inspection handle",
        )?;
        canonical_root_from_handle(path, &handle)
    }

    fn open_root(path: &Path) -> Result<OpenRoot, RootLockError> {
        validate_explicit_path(path)?;
        reject_unsupported_volume(path)?;

        // DELETE access plus no FILE_SHARE_DELETE turns the namespace handle
        // into the path binding. Probe narrowly first: a reparse entry keeps
        // this narrow guard, while an ordinary directory is reopened with
        // FILE_SHARE_WRITE so child atomic-write workflows remain available.
        let narrow_binding = open_directory_handle(
            path,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            FILE_READ_ATTRIBUTES | DELETE_ACCESS,
            FILE_SHARE_READ,
            "root path binding handle",
        )?;
        let (identity_handle, path_binding_handle) = if handle_is_reparse(&narrow_binding)? {
            (
                open_directory_handle(
                    path,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    FILE_READ_ATTRIBUTES | DELETE_ACCESS,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    "root identity handle",
                )?,
                Some(narrow_binding),
            )
        } else {
            drop(narrow_binding);
            let ordinary = open_directory_handle(
                path,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                FILE_READ_ATTRIBUTES | DELETE_ACCESS,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                "root identity and path binding handle",
            )?;
            if handle_is_reparse(&ordinary)? {
                return Err(RootLockError::RootPathChanged);
            }
            (ordinary, None)
        };

        let canonical = canonical_root_from_handle(path, &identity_handle)?;

        Ok(OpenRoot {
            canonical,
            identity_handle,
            path_binding_handle,
        })
    }

    fn canonical_root_from_handle(
        requested_path: &Path,
        identity_handle: &OwnedHandle,
    ) -> Result<CanonicalRoot, RootLockError> {
        let mut standard = FileStandardInfo {
            allocation_size: 0,
            end_of_file: 0,
            number_of_links: 0,
            delete_pending: 0,
            directory: 0,
        };
        let standard_ok = unsafe {
            GetFileInformationByHandleEx(
                identity_handle.raw(),
                FILE_STANDARD_INFO_CLASS,
                (&mut standard as *mut FileStandardInfo).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            )
        };
        if standard_ok == 0 {
            return Err(RootLockError::InspectRoot {
                operation: "FileStandardInfo",
                source: io::Error::last_os_error(),
            });
        }
        if standard.directory == 0 {
            return Err(RootLockError::RootNotDirectory(
                requested_path.to_path_buf(),
            ));
        }

        let canonical_path = final_path(identity_handle.raw())?;
        validate_explicit_path(&canonical_path)?;
        reject_unsupported_volume(&canonical_path)?;

        let mut id = FileIdInfo {
            volume_serial_number: 0,
            file_id: FileId128 {
                identifier: [0; 16],
            },
        };
        let id_ok = unsafe {
            GetFileInformationByHandleEx(
                identity_handle.raw(),
                FILE_ID_INFO_CLASS,
                (&mut id as *mut FileIdInfo).cast(),
                std::mem::size_of::<FileIdInfo>() as u32,
            )
        };
        if id_ok == 0 {
            return Err(RootLockError::InspectRoot {
                operation: "FileIdInfo",
                source: io::Error::last_os_error(),
            });
        }

        Ok(CanonicalRoot {
            requested_path: requested_path.to_path_buf(),
            canonical_path,
            identity: RootIdentity {
                volume_serial: id.volume_serial_number,
                file_id: id.file_id.identifier,
            },
        })
    }

    fn handle_is_reparse(handle: &OwnedHandle) -> Result<bool, RootLockError> {
        let mut info = FileAttributeTagInfo {
            file_attributes: 0,
            reparse_tag: 0,
        };
        let ok = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_ATTRIBUTE_TAG_INFO_CLASS,
                (&mut info as *mut FileAttributeTagInfo).cast(),
                std::mem::size_of::<FileAttributeTagInfo>() as u32,
            )
        };
        if ok == 0 {
            return Err(RootLockError::InspectRoot {
                operation: "FileAttributeTagInfo",
                source: io::Error::last_os_error(),
            });
        }
        Ok(info.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }

    fn open_directory_handle(
        path: &Path,
        flags: u32,
        desired_access: u32,
        share_mode: u32,
        operation: &'static str,
    ) -> Result<OwnedHandle, RootLockError> {
        let wide_path = wide(path.as_os_str());
        let raw = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                desired_access,
                share_mode,
                ptr::null(),
                OPEN_EXISTING,
                flags,
                ptr::null_mut(),
            )
        };
        let handle = OwnedHandle::new(raw).ok_or_else(|| RootLockError::OpenRoot {
            path: path.to_path_buf(),
            source: io::Error::last_os_error(),
        })?;
        clear_inheritance(handle.raw(), operation)?;
        Ok(handle)
    }

    fn pin_database_file<'root>(
        root_lock: &'root RootLock,
        requested_path: &Path,
        creation_disposition: u32,
        created_new: bool,
    ) -> Result<DatabaseFilePin<'root>, DatabasePinError> {
        let authoritative_path = prepare_database_path(root_lock, requested_path)?;
        let handle = match open_file_handle(
            &authoritative_path,
            GENERIC_READ | GENERIC_WRITE | DELETE_ACCESS,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            creation_disposition,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            if created_new {
                "atomically create and pin database"
            } else {
                "open and pin existing database"
            },
        ) {
            Err(DatabasePinError::OpenFile { source, .. })
                if created_new
                    && matches!(
                        source.raw_os_error(),
                        Some(ERROR_FILE_EXISTS) | Some(ERROR_ALREADY_EXISTS)
                    ) =>
            {
                return Err(DatabasePinError::AlreadyExists(authoritative_path));
            }
            result => result?,
        };

        let identity = match inspect_database_file(&authoritative_path, &handle) {
            Ok(identity) => identity,
            Err(error) => {
                // Never unlink after releasing the only pinned handle: a
                // replacement could land in that gap. A failed CREATE_NEW
                // therefore leaves its explicit empty artifact for recovery.
                drop(handle);
                return Err(error);
            }
        };

        Ok(DatabaseFilePin {
            authoritative_path,
            identity,
            root_identity: root_lock.canonical.identity.clone(),
            created_new,
            handle,
            _root_lock: PhantomData,
        })
    }

    fn prepare_database_path(
        root_lock: &RootLock,
        requested_path: &Path,
    ) -> Result<PathBuf, DatabasePinError> {
        validate_explicit_path(requested_path).map_err(DatabasePinError::InvalidPath)?;
        let parent = requested_path
            .parent()
            .ok_or_else(|| DatabasePinError::ParentRequired(requested_path.to_path_buf()))?;
        let parent_root = inspect_root(parent).map_err(DatabasePinError::InvalidPath)?;
        if parent_root.identity != root_lock.canonical.identity {
            return Err(DatabasePinError::OutsideRoot {
                path: requested_path.to_path_buf(),
                expected: root_lock.canonical.identity.clone(),
                observed: parent_root.identity,
            });
        }

        let file_name = requested_path
            .components()
            .next_back()
            .and_then(|component| match component {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .filter(|name| safe_database_file_name(name))
            .ok_or_else(|| DatabasePinError::UnsafeChildName(requested_path.to_path_buf()))?;

        Ok(root_lock.canonical.canonical_path.join(file_name))
    }

    fn safe_database_file_name(name: &OsStr) -> bool {
        if has_interior_nul(name) {
            return false;
        }
        let Some(name) = name.to_str() else {
            return false;
        };
        if name.is_empty()
            || name.contains(':')
            || name.ends_with(['.', ' '])
            || name.contains(['\\', '/'])
        {
            return false;
        }
        let device_stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        !matches!(device_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !matches!(
                device_stem.as_str(),
                "COM1"
                    | "COM2"
                    | "COM3"
                    | "COM4"
                    | "COM5"
                    | "COM6"
                    | "COM7"
                    | "COM8"
                    | "COM9"
                    | "LPT1"
                    | "LPT2"
                    | "LPT3"
                    | "LPT4"
                    | "LPT5"
                    | "LPT6"
                    | "LPT7"
                    | "LPT8"
                    | "LPT9"
            )
            && !matches!(
                device_stem.as_str(),
                "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³"
            )
    }

    fn open_file_handle(
        path: &Path,
        desired_access: u32,
        share_mode: u32,
        creation_disposition: u32,
        flags: u32,
        operation: &'static str,
    ) -> Result<OwnedHandle, DatabasePinError> {
        let wide_path = wide(path.as_os_str());
        let raw = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                desired_access,
                share_mode,
                ptr::null(),
                creation_disposition,
                flags,
                ptr::null_mut(),
            )
        };
        let handle = OwnedHandle::new(raw).ok_or_else(|| DatabasePinError::OpenFile {
            path: path.to_path_buf(),
            operation,
            source: io::Error::last_os_error(),
        })?;
        let ok = unsafe { SetHandleInformation(handle.raw(), HANDLE_FLAG_INHERIT, 0) };
        if ok == 0 {
            return Err(DatabasePinError::HandlePolicy {
                source: io::Error::last_os_error(),
            });
        }
        Ok(handle)
    }

    fn inspect_database_file(
        path: &Path,
        handle: &OwnedHandle,
    ) -> Result<RootIdentity, DatabasePinError> {
        let mut attributes = FileAttributeTagInfo {
            file_attributes: 0,
            reparse_tag: 0,
        };
        let attributes_ok = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_ATTRIBUTE_TAG_INFO_CLASS,
                (&mut attributes as *mut FileAttributeTagInfo).cast(),
                std::mem::size_of::<FileAttributeTagInfo>() as u32,
            )
        };
        if attributes_ok == 0 {
            return Err(DatabasePinError::InspectFile {
                operation: "FileAttributeTagInfo",
                source: io::Error::last_os_error(),
            });
        }
        if attributes.file_attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(DatabasePinError::ReparsePoint(path.to_path_buf()));
        }

        let mut standard = FileStandardInfo {
            allocation_size: 0,
            end_of_file: 0,
            number_of_links: 0,
            delete_pending: 0,
            directory: 0,
        };
        let standard_ok = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_STANDARD_INFO_CLASS,
                (&mut standard as *mut FileStandardInfo).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            )
        };
        if standard_ok == 0 {
            return Err(DatabasePinError::InspectFile {
                operation: "FileStandardInfo",
                source: io::Error::last_os_error(),
            });
        }
        if standard.directory != 0 {
            return Err(DatabasePinError::NotRegularFile(path.to_path_buf()));
        }

        let mut id = FileIdInfo {
            volume_serial_number: 0,
            file_id: FileId128 {
                identifier: [0; 16],
            },
        };
        let id_ok = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_ID_INFO_CLASS,
                (&mut id as *mut FileIdInfo).cast(),
                std::mem::size_of::<FileIdInfo>() as u32,
            )
        };
        if id_ok == 0 {
            return Err(DatabasePinError::InspectFile {
                operation: "FileIdInfo",
                source: io::Error::last_os_error(),
            });
        }
        Ok(RootIdentity {
            volume_serial: id.volume_serial_number,
            file_id: id.file_id.identifier,
        })
    }

    fn sidecar_path(main: &Path, kind: DatabaseSidecarKind) -> PathBuf {
        let mut name = main
            .file_name()
            .map(|value| value.to_os_string())
            .unwrap_or_default();
        name.push(kind.suffix());
        main.with_file_name(name)
    }

    fn inspect_one_sidecar(
        main: &Path,
        kind: DatabaseSidecarKind,
    ) -> Result<DatabaseSidecarState, DatabasePinError> {
        let path = sidecar_path(main, kind);
        let handle = match open_file_handle(
            &path,
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            "inspect database sidecar",
        ) {
            Err(DatabasePinError::OpenFile { source, .. })
                if matches!(source.raw_os_error(), Some(2) | Some(3)) =>
            {
                return Ok(DatabaseSidecarState::Absent);
            }
            result => result?,
        };
        let identity = inspect_database_file(&path, &handle)?;
        let size = file_end_of_file(&handle)?;
        Ok(DatabaseSidecarState::Present {
            identity,
            size,
            truncated_header: size > 0 && size < kind.header_bytes(),
        })
    }

    fn file_end_of_file(handle: &OwnedHandle) -> Result<u64, DatabasePinError> {
        let mut standard = FileStandardInfo {
            allocation_size: 0,
            end_of_file: 0,
            number_of_links: 0,
            delete_pending: 0,
            directory: 0,
        };
        let standard_ok = unsafe {
            GetFileInformationByHandleEx(
                handle.raw(),
                FILE_STANDARD_INFO_CLASS,
                (&mut standard as *mut FileStandardInfo).cast(),
                std::mem::size_of::<FileStandardInfo>() as u32,
            )
        };
        if standard_ok == 0 {
            return Err(DatabasePinError::InspectFile {
                operation: "FileStandardInfo",
                source: io::Error::last_os_error(),
            });
        }
        if standard.end_of_file < 0 {
            return Err(DatabasePinError::InspectFile {
                operation: "FileStandardInfo",
                source: io::Error::other("negative end of file"),
            });
        }
        Ok(standard.end_of_file as u64)
    }

    fn database_handle_is_inheritable(handle: Handle) -> Result<bool, DatabasePinError> {
        let mut flags = 0u32;
        let ok = unsafe { GetHandleInformation(handle, &mut flags) };
        if ok == 0 {
            return Err(DatabasePinError::HandlePolicy {
                source: io::Error::last_os_error(),
            });
        }
        Ok(flags & HANDLE_FLAG_INHERIT != 0)
    }

    fn acquire_named_mutex(
        name: &str,
        identity: &RootIdentity,
    ) -> Result<(OwnedHandle, LockThreadReady), RootLockError> {
        let wide_name = wide(OsStr::new(name));
        let raw = unsafe { CreateMutexW(ptr::null(), 1, wide_name.as_ptr()) };
        let mutex = OwnedHandle::new(raw).ok_or_else(|| RootLockError::CreateLock {
            name: name.to_owned(),
            source: io::Error::last_os_error(),
        })?;
        let already_existed = unsafe { GetLastError() as i32 } == ERROR_ALREADY_EXISTS;
        if already_existed {
            let wait = unsafe { WaitForSingleObject(mutex.raw(), 0) };
            match wait {
                WAIT_OBJECT_0 | WAIT_ABANDONED => {}
                WAIT_TIMEOUT => {
                    return Err(RootLockError::AlreadyLocked {
                        identity: identity.clone(),
                    })
                }
                WAIT_FAILED => {
                    return Err(RootLockError::CreateLock {
                        name: name.to_owned(),
                        source: io::Error::last_os_error(),
                    })
                }
                _ => {
                    return Err(RootLockError::CreateLock {
                        name: name.to_owned(),
                        source: io::Error::other(format!("unexpected mutex wait result {wait}")),
                    })
                }
            }
        }
        clear_inheritance(mutex.raw(), "root mutex handle")?;
        let handle_inheritable = handle_is_inheritable(mutex.raw())?;
        Ok((mutex, LockThreadReady { handle_inheritable }))
    }

    fn validate_explicit_path(path: &Path) -> Result<(), RootLockError> {
        if path.as_os_str().is_empty() {
            return Err(RootLockError::MissingRoot);
        }
        if has_interior_nul(path.as_os_str()) {
            return Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "interior NUL is not admitted",
            });
        }
        let normalized = path.as_os_str().to_string_lossy().replace('/', "\\");
        if is_drive_absolute(&normalized)
            || is_extended_drive_absolute(&normalized)
            || is_volume_guid_absolute(&normalized)
        {
            return Ok(());
        }
        if path.is_absolute() || normalized.starts_with('\\') {
            return Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "only drive-absolute and well-formed Volume GUID roots are admitted",
            });
        }
        Err(RootLockError::RootMustBeAbsolute(path.to_path_buf()))
    }

    fn reject_unsupported_volume(path: &Path) -> Result<(), RootLockError> {
        let wide_path = wide(path.as_os_str());
        let mut volume = vec![0u16; 32_768];
        let ok = unsafe {
            GetVolumePathNameW(wide_path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32)
        };
        if ok == 0 {
            return Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "volume kind could not be established",
            });
        }
        let kind = unsafe { GetDriveTypeW(volume.as_ptr()) };
        match kind {
            DRIVE_FIXED | DRIVE_REMOVABLE | DRIVE_RAMDISK => Ok(()),
            DRIVE_REMOTE => Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "remote volumes are not admitted",
            }),
            DRIVE_UNKNOWN | DRIVE_NO_ROOT_DIR => Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "volume kind is unknown",
            }),
            DRIVE_CDROM => Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "read-only optical volumes cannot be product roots",
            }),
            _ => Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "unrecognized volume kind",
            }),
        }
    }

    fn final_path(handle: Handle) -> Result<PathBuf, RootLockError> {
        let required = unsafe { GetFinalPathNameByHandleW(handle, ptr::null_mut(), 0, 0) };
        if required == 0 {
            return Err(RootLockError::InspectRoot {
                operation: "GetFinalPathNameByHandleW(size)",
                source: io::Error::last_os_error(),
            });
        }
        let mut buffer = vec![0u16; required as usize + 1];
        let written = unsafe {
            GetFinalPathNameByHandleW(handle, buffer.as_mut_ptr(), buffer.len() as u32, 0)
        };
        if written == 0 || written as usize >= buffer.len() {
            return Err(RootLockError::InspectRoot {
                operation: "GetFinalPathNameByHandleW(path)",
                source: io::Error::last_os_error(),
            });
        }
        buffer.truncate(written as usize);
        Ok(PathBuf::from(OsString::from_wide(&buffer)))
    }

    fn clear_inheritance(handle: Handle, operation: &'static str) -> Result<(), RootLockError> {
        let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
        if ok == 0 {
            return Err(RootLockError::HandlePolicy {
                operation,
                source: io::Error::last_os_error(),
            });
        }
        Ok(())
    }

    fn handle_is_inheritable(handle: Handle) -> Result<bool, RootLockError> {
        let mut flags = 0u32;
        let ok = unsafe { GetHandleInformation(handle, &mut flags) };
        if ok == 0 {
            return Err(RootLockError::HandlePolicy {
                operation: "GetHandleInformation",
                source: io::Error::last_os_error(),
            });
        }
        Ok(flags & HANDLE_FLAG_INHERIT != 0)
    }

    fn is_drive_absolute(path: &str) -> bool {
        let bytes = path.as_bytes();
        bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'\\'
    }

    fn is_extended_drive_absolute(path: &str) -> bool {
        path.strip_prefix(r"\\?\").is_some_and(is_drive_absolute)
    }

    fn is_volume_guid_absolute(path: &str) -> bool {
        const PREFIX: &str = r"\\?\Volume{";
        let Some(rest) = path.get(PREFIX.len()..) else {
            return false;
        };
        if !path[..PREFIX.len()].eq_ignore_ascii_case(PREFIX) {
            return false;
        }
        let bytes = rest.as_bytes();
        if bytes.len() < 38 || bytes[36] != b'}' || bytes[37] != b'\\' {
            return false;
        }
        bytes[..36].iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
    }

    fn has_interior_nul(value: &OsStr) -> bool {
        value.encode_wide().any(|unit| unit == 0)
    }

    fn wide(value: &OsStr) -> Vec<u16> {
        #[cfg(test)]
        WIDE_CALL_COUNT.with(|count| count.set(count.get() + 1));
        let mut encoded: Vec<u16> = value.encode_wide().collect();
        assert!(
            !encoded.contains(&0),
            "interior NUL must be rejected before Windows FFI conversion"
        );
        encoded.push(0);
        encoded
    }

    #[cfg(test)]
    pub(super) fn wide_call_count() -> usize {
        WIDE_CALL_COUNT.with(Cell::get)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{
        mark_root_poisoned, CanonicalRoot, DatabasePinError, DatabaseSidecarReport, RootIdentity,
        RootLockError,
    };
    use std::marker::PhantomData;
    use std::path::Path;

    pub struct RootLock;
    pub struct DatabaseFilePin<'root> {
        _root: PhantomData<&'root RootLock>,
    }

    impl RootLock {
        pub fn acquire(path: &Path) -> Result<Self, RootLockError> {
            Err(RootLockError::UnsupportedRoot {
                path: path.to_path_buf(),
                reason: "the native root lock requires a Windows implementation",
            })
        }

        pub fn canonical_root(&self) -> &CanonicalRoot {
            unreachable!("a non-Windows RootLock cannot be acquired")
        }

        pub(crate) fn poison_identity(identity: &RootIdentity) {
            mark_root_poisoned(identity);
        }

        pub fn handles_are_non_inheritable(&self) -> Result<bool, RootLockError> {
            Err(RootLockError::UnsupportedRoot {
                path: PathBuf::new(),
                reason: "the native root lock requires a Windows implementation",
            })
        }

        pub fn root_handle_inheritable(&self) -> Result<bool, RootLockError> {
            self.handles_are_non_inheritable().map(|value| !value)
        }

        pub fn lock_handle_inheritable(&self) -> bool {
            unreachable!("a non-Windows RootLock cannot be acquired")
        }

        pub fn pin_existing_database<'root>(
            &'root self,
            _path: &Path,
        ) -> Result<DatabaseFilePin<'root>, DatabasePinError> {
            Err(DatabasePinError::UnsupportedPlatform)
        }

        pub fn create_and_pin_database<'root>(
            &'root self,
            _path: &Path,
        ) -> Result<DatabaseFilePin<'root>, DatabasePinError> {
            Err(DatabasePinError::UnsupportedPlatform)
        }

        pub fn inspect_database_sidecars(
            &self,
            _main_path: &Path,
        ) -> Result<DatabaseSidecarReport, DatabasePinError> {
            Err(DatabasePinError::UnsupportedPlatform)
        }
    }

    impl DatabaseFilePin<'_> {
        pub fn path(&self) -> &Path {
            unreachable!("a non-Windows DatabaseFilePin cannot be acquired")
        }

        pub fn identity(&self) -> &RootIdentity {
            unreachable!("a non-Windows DatabaseFilePin cannot be acquired")
        }

        pub fn created_new(&self) -> bool {
            unreachable!("a non-Windows DatabaseFilePin cannot be acquired")
        }

        pub fn handle_inheritable(&self) -> Result<bool, DatabasePinError> {
            Err(DatabasePinError::UnsupportedPlatform)
        }

        pub fn verify_path_identity(&self) -> Result<bool, DatabasePinError> {
            Err(DatabasePinError::UnsupportedPlatform)
        }
    }

    pub fn inspect_root(path: &Path) -> Result<CanonicalRoot, RootLockError> {
        Err(RootLockError::UnsupportedRoot {
            path: path.to_path_buf(),
            reason: "the native root identity requires a Windows implementation",
        })
    }

    use std::path::PathBuf;
}

#[cfg(windows)]
pub(crate) use platform::SqliteMainHandle;
pub use platform::{inspect_root, DatabaseFilePin, RootLock};

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(all(test, windows))]
mod tests {
    use super::platform::wide_call_count;
    use super::{inspect_root, DatabasePinError, DatabaseSidecarState, RootLock, RootLockError};
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, OpenOptions};
    use std::io::{Seek, SeekFrom, Write};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    #[link(name = "kernel32")]
    extern "system" {
        fn QueryDosDeviceW(device_name: *const u16, target_path: *mut u16, max: u32) -> u32;
    }

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gogoke-root-{label}-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).expect("create isolated test root");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn poison_published_after_precheck_blocks_same_physical_root_before_lock_publication() {
        use std::sync::mpsc::sync_channel;
        let root = TempRoot::new("poison-after-precheck");
        let first = RootLock::acquire(root.path()).expect("first physical root owner");
        let identity = first.canonical_root().identity.clone();
        let path = root.path().to_path_buf();
        let (checked_tx, checked_rx) = sync_channel(0);
        let (resume_tx, resume_rx) = sync_channel(0);
        let contender = std::thread::spawn(move || {
            RootLock::acquire_after_precheck(&path, |_| {
                checked_tx.send(()).expect("precheck observed");
                resume_rx.recv().expect("prior holder settled");
            }).map(drop)
        });
        checked_rx.recv().expect("contender completed unpoisoned precheck");
        RootLock::poison_identity(&identity);
        drop(first); // Production close failure publishes poison before dropping this lock.
        resume_tx.send(()).expect("release contender to acquire OS mutex");
        assert!(matches!(contender.join().expect("contender settled"),
            Err(RootLockError::Poisoned { identity: found }) if found == identity));
        assert!(matches!(RootLock::acquire(root.path()),
            Err(RootLockError::Poisoned { identity: found }) if found == identity));
        let unrelated = TempRoot::new("unpoisoned-control");
        let other = RootLock::acquire(unrelated.path()).expect("unrelated root remains eligible");
        drop(other);
    }

    fn create_junction(alias: &Path, target: &Path) {
        let output = Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(alias)
            .arg(target)
            .output()
            .expect("launch mklink");
        assert!(
            output.status.success(),
            "mklink failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn valid_globalroot_alias(path: &Path) -> PathBuf {
        let requested = path.to_string_lossy();
        assert!(requested.len() >= 3 && requested.as_bytes()[1] == b':');
        let drive = &requested[..2];
        let wide_drive: Vec<u16> = OsStr::new(drive)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut target = vec![0u16; 32_768];
        let written = unsafe {
            QueryDosDeviceW(
                wide_drive.as_ptr(),
                target.as_mut_ptr(),
                target.len() as u32,
            )
        };
        assert!(
            written > 0,
            "QueryDosDeviceW failed: {}",
            std::io::Error::last_os_error()
        );
        let end = target
            .iter()
            .position(|unit| *unit == 0)
            .expect("DOS device terminator");
        let device = OsString::from_wide(&target[..end])
            .to_string_lossy()
            .into_owned();
        PathBuf::from(format!(r"\\?\GLOBALROOT{}{}", device, &requested[2..]))
    }

    fn assert_child_atomic_write_lifecycle(root: &Path, label: &str) {
        let temporary = root.join(format!("{label}.tmp"));
        let final_path = root.join(format!("{label}.final"));
        let payload = format!("gogoke-root-write-{label}").into_bytes();

        fs::write(&temporary, &payload).expect("create and write temporary child");
        fs::rename(&temporary, &final_path).expect("atomically rename child into place");
        assert_eq!(
            fs::read(&final_path).expect("reopen and read final child"),
            payload
        );
        fs::remove_file(&final_path).expect("delete final child");
    }

    #[test]
    fn requires_an_explicit_absolute_root_without_cwd_fallback() {
        assert!(matches!(
            RootLock::acquire(Path::new("")),
            Err(RootLockError::MissingRoot)
        ));
        assert!(matches!(
            RootLock::acquire(Path::new("relative-root")),
            Err(RootLockError::RootMustBeAbsolute(_))
        ));
    }

    #[test]
    fn rejects_network_and_device_namespaces_before_opening_them() {
        for path in [
            r"\\server\share\root",
            r"\\?\UNC\server\share\root",
            r"\\.\C:\root",
            r"\\?\Device\HarddiskVolume1\root",
            r"\??\C:\root",
        ] {
            assert!(matches!(
                RootLock::acquire(Path::new(path)),
                Err(RootLockError::UnsupportedRoot { .. })
            ));
        }
    }

    #[test]
    fn lock_uses_physical_identity_and_creates_no_lock_file() {
        let parent = TempRoot::new("junction");
        let target = parent.path().join("target");
        let alias = parent.path().join("alias");
        fs::create_dir(&target).expect("create target");
        create_junction(&alias, &target);

        let direct = inspect_root(&target).expect("inspect target");
        let through_alias = inspect_root(&alias).expect("inspect junction");
        assert_eq!(direct.identity, through_alias.identity);

        let lock = RootLock::acquire(&target).expect("first writer acquires lock");
        assert!(lock.handles_are_non_inheritable().expect("inspect handles"));
        assert!(matches!(
            RootLock::acquire(&alias),
            Err(RootLockError::AlreadyLocked { .. })
        ));
        assert_eq!(
            fs::read_dir(&target).expect("read target").count(),
            0,
            "OS exclusion must not create a replaceable lock file"
        );
        drop(lock);
        fs::remove_dir(&alias).expect("remove test junction");
    }

    #[test]
    fn rejects_missing_roots_and_regular_files_without_creating_state() {
        let parent = TempRoot::new("invalid");
        let missing = parent.path().join("missing");
        assert!(matches!(
            RootLock::acquire(&missing),
            Err(RootLockError::OpenRoot { .. })
        ));
        assert!(!missing.exists());

        let file = parent.path().join("file");
        fs::write(&file, b"not a directory").expect("create test file");
        assert!(matches!(
            RootLock::acquire(&file),
            Err(RootLockError::RootNotDirectory(_))
        ));
        assert_eq!(
            fs::read(&file).expect("file remains intact"),
            b"not a directory"
        );
    }

    #[test]
    fn held_root_path_cannot_be_renamed_deleted_or_replaced() {
        let parent = TempRoot::new("path-binding");
        let root = parent.path().join("root");
        let renamed = parent.path().join("renamed");
        fs::create_dir(&root).expect("create root");

        let lock = RootLock::acquire(&root).expect("acquire bound root");
        let original = lock.canonical_root().identity.clone();
        assert_child_atomic_write_lifecycle(&root, "direct-root");
        assert!(
            fs::rename(&root, &renamed).is_err(),
            "rename must be denied while held"
        );
        assert!(
            fs::remove_dir(&root).is_err(),
            "delete must be denied while held"
        );
        assert_eq!(
            inspect_root(&root)
                .expect("root remains resolvable")
                .identity,
            original
        );

        drop(lock);
        fs::rename(&root, &renamed).expect("rename proceeds after release");
        assert_eq!(
            inspect_root(&renamed)
                .expect("inspect renamed root")
                .identity,
            original
        );
    }

    #[test]
    fn held_junction_and_target_cannot_be_retargeted() {
        let parent = TempRoot::new("junction-binding");
        let target_one = parent.path().join("target-one");
        let target_two = parent.path().join("target-two");
        let target_one_moved = parent.path().join("target-one-moved");
        let alias = parent.path().join("alias");
        let alias_moved = parent.path().join("alias-moved");
        fs::create_dir(&target_one).expect("create first target");
        fs::create_dir(&target_two).expect("create replacement target");
        create_junction(&alias, &target_one);

        let lock = RootLock::acquire(&alias).expect("acquire junction root");
        let original = lock.canonical_root().identity.clone();
        assert_child_atomic_write_lifecycle(&alias, "junction-target");
        assert!(
            fs::rename(&alias, &alias_moved).is_err(),
            "junction rename must be denied"
        );
        assert!(
            fs::remove_dir(&alias).is_err(),
            "junction deletion must be denied"
        );
        assert!(
            fs::rename(&target_one, &target_one_moved).is_err(),
            "physical target rename must be denied"
        );
        assert_eq!(
            inspect_root(&alias)
                .expect("junction remains bound")
                .identity,
            original
        );

        drop(lock);
        fs::remove_dir(&alias).expect("junction removal proceeds after release");
        create_junction(&alias, &target_two);
        let replacement = inspect_root(&alias)
            .expect("inspect replacement target")
            .identity;
        assert_ne!(
            replacement, original,
            "test replacement must be a distinct object"
        );
        fs::remove_dir(&alias).expect("remove replacement junction");
    }

    #[test]
    fn rejects_a_valid_globalroot_alias_before_opening_it() {
        let root = TempRoot::new("globalroot");
        let globalroot = valid_globalroot_alias(root.path());
        assert!(fs::metadata(&globalroot)
            .expect("GLOBALROOT regression path must be valid")
            .is_dir());
        assert!(matches!(
            RootLock::acquire(&globalroot),
            Err(RootLockError::UnsupportedRoot { .. })
        ));
    }

    #[test]
    fn existing_database_pin_blocks_preflight_swap_but_allows_database_io() {
        let root = TempRoot::new("database-existing");
        let database = root.path().join("core.db");
        let replacement = root.path().join("replacement.db");
        let backup = root.path().join("core.db.backup");
        let wal = root.path().join("core.db-wal");
        fs::write(&database, b"original").expect("seed database");
        fs::write(&replacement, b"replacement").expect("seed replacement");

        let root_lock = RootLock::acquire(root.path()).expect("acquire root");
        let pin = root_lock
            .pin_existing_database(&database)
            .expect("pin existing database");
        assert!(!pin.created_new());
        assert!(!pin.handle_inheritable().expect("inspect database handle"));
        assert!(pin.verify_path_identity().expect("verify pinned path"));

        let mut writable = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&database)
            .expect("open writable database while pinned");
        writable.seek(SeekFrom::End(0)).expect("seek database");
        writable.write_all(b"-write").expect("write database");
        writable.sync_all().expect("sync database");
        drop(writable);
        assert_eq!(
            fs::read(&database).expect("reopen database"),
            b"original-write"
        );

        fs::write(&wal, b"wal-frame").expect("create WAL sidecar");
        assert_eq!(fs::read(&wal).expect("read WAL sidecar"), b"wal-frame");
        let sidecars = root_lock
            .inspect_database_sidecars(&database)
            .expect("inspect sidecars independently of main pin");
        assert_eq!(sidecars.main_identity, *pin.identity());
        assert!(
            matches!(
                sidecars.wal,
                DatabaseSidecarState::Present {
                    truncated_header: true,
                    size: 9,
                    ..
                }
            ),
            "short WAL is truncated, not qualified by the main pin: {:?}",
            sidecars.wal
        );
        assert_eq!(sidecars.shm, DatabaseSidecarState::Absent);
        fs::remove_file(&wal).expect("delete WAL sidecar");

        assert!(
            fs::rename(&database, &backup).is_err(),
            "pinned DB rename must fail"
        );
        assert!(
            fs::remove_file(&database).is_err(),
            "pinned DB delete must fail"
        );
        assert!(
            fs::rename(&replacement, &database).is_err(),
            "replacement over pinned DB must fail"
        );
        assert!(pin.verify_path_identity().expect("identity remains stable"));

        drop(pin);
        fs::rename(&database, &backup).expect("rename allowed after pin release");
        fs::rename(&replacement, &database).expect("swap allowed after pin release");
        assert_eq!(
            fs::read(&database).expect("read swapped database"),
            b"replacement"
        );
        fs::remove_file(&database).expect("remove swapped database");
        fs::rename(&backup, &database).expect("swap back original database");
        assert_eq!(
            fs::read(&database).expect("read restored database"),
            b"original-write"
        );
    }

    #[test]
    fn create_new_database_pin_is_atomic_and_never_truncates_existing() {
        let root = TempRoot::new("database-create");
        let database = root.path().join("core.db");
        let renamed = root.path().join("renamed.db");
        let root_lock = RootLock::acquire(root.path()).expect("acquire root");

        let pin = root_lock
            .create_and_pin_database(&database)
            .expect("atomically create and pin database");
        assert!(pin.created_new());
        assert!(!pin.handle_inheritable().expect("inspect database handle"));
        assert!(pin.verify_path_identity().expect("verify created database"));
        fs::write(&database, b"created-content").expect("write created database");
        assert_eq!(
            fs::read(&database).expect("read created database"),
            b"created-content"
        );
        assert!(
            fs::rename(&database, &renamed).is_err(),
            "created pinned database rename must fail"
        );
        assert!(
            fs::remove_file(&database).is_err(),
            "created pinned database delete must fail"
        );
        drop(pin);

        assert!(matches!(
            root_lock.create_and_pin_database(&database),
            Err(DatabasePinError::AlreadyExists(_))
        ));
        assert_eq!(
            fs::read(&database).expect("existing database was not truncated"),
            b"created-content"
        );
        fs::rename(&database, &renamed).expect("rename allowed after release");
    }

    #[test]
    fn database_pin_rejects_outside_nested_reparse_directory_and_device_paths() {
        let root = TempRoot::new("database-reject");
        let outside = TempRoot::new("database-outside");
        let root_lock = RootLock::acquire(root.path()).expect("acquire root");

        let outside_database = outside.path().join("outside.db");
        fs::write(&outside_database, b"outside").expect("seed outside database");
        assert!(matches!(
            root_lock.pin_existing_database(&outside_database),
            Err(DatabasePinError::OutsideRoot { .. })
        ));

        let outside_alias = root.path().join("outside-alias");
        create_junction(&outside_alias, outside.path());
        assert!(matches!(
            root_lock.pin_existing_database(&outside_alias.join("outside.db")),
            Err(DatabasePinError::OutsideRoot { .. })
        ));
        fs::remove_dir(&outside_alias).expect("remove outside-parent junction");

        let nested = root.path().join("nested");
        fs::create_dir(&nested).expect("create nested directory");
        let nested_database = nested.join("nested.db");
        fs::write(&nested_database, b"nested").expect("seed nested database");
        assert!(matches!(
            root_lock.pin_existing_database(&nested_database),
            Err(DatabasePinError::OutsideRoot { .. })
        ));

        let reparse_database = root.path().join("reparse.db");
        create_junction(&reparse_database, outside.path());
        assert!(matches!(
            root_lock.pin_existing_database(&reparse_database),
            Err(DatabasePinError::ReparsePoint(_))
        ));
        fs::remove_dir(&reparse_database).expect("remove test reparse point");

        let sidecar_main = root.path().join("sidecar.db");
        fs::write(&sidecar_main, b"main").expect("seed sidecar main");
        let pin = root_lock
            .pin_existing_database(&sidecar_main)
            .expect("pin main without qualifying sidecars");
        let absent = root_lock
            .inspect_database_sidecars(&sidecar_main)
            .expect("absent sidecars");
        assert_eq!(absent.wal, DatabaseSidecarState::Absent);
        assert_eq!(absent.shm, DatabaseSidecarState::Absent);

        let residue_wal = root.path().join("sidecar.db-wal");
        let residue_shm = root.path().join("sidecar.db-shm");
        fs::write(&residue_wal, vec![0u8; 40]).expect("crash WAL residue");
        fs::write(&residue_shm, vec![0u8; 16]).expect("truncated SHM residue");
        let residue = root_lock
            .inspect_database_sidecars(&sidecar_main)
            .expect("preserve crash residue");
        assert!(matches!(
            residue.wal,
            DatabaseSidecarState::Present {
                truncated_header: false,
                size: 40,
                ..
            }
        ));
        assert!(matches!(
            residue.shm,
            DatabaseSidecarState::Present {
                truncated_header: true,
                size: 16,
                ..
            }
        ));
        assert_eq!(fs::read(&residue_wal).expect("WAL residue kept"), vec![0u8; 40]);
        assert_eq!(fs::read(&residue_shm).expect("SHM residue kept"), vec![0u8; 16]);
        assert!(pin.verify_path_identity().expect("main pin still holds"));

        let colliding_wal = root.path().join("sidecar.db-wal");
        fs::remove_file(&colliding_wal).expect("remove residue WAL before reparse");
        create_junction(&colliding_wal, outside.path());
        assert!(
            pin.verify_path_identity().expect("main pin ignores WAL reparse"),
            "main pin must not treat WAL reparse as its own identity"
        );
        assert!(matches!(
            root_lock.inspect_database_sidecars(&sidecar_main),
            Err(DatabasePinError::ReparsePoint(_))
        ));
        fs::remove_dir(&colliding_wal).expect("remove WAL reparse");
        fs::remove_file(&residue_shm).expect("remove SHM residue");
        drop(pin);

        let directory_database = root.path().join("directory.db");
        fs::create_dir(&directory_database).expect("create directory-shaped database");
        assert!(matches!(
            root_lock.pin_existing_database(&directory_database),
            Err(DatabasePinError::NotRegularFile(_))
        ));

        assert!(matches!(
            root_lock.create_and_pin_database(&root.path().join("NUL.db")),
            Err(DatabasePinError::UnsafeChildName(_))
        ));
        assert!(matches!(
            root_lock.pin_existing_database(Path::new(r"\\.\NUL")),
            Err(DatabasePinError::InvalidPath(_))
        ));
    }

    #[test]
    fn database_create_rejects_all_reserved_device_spellings_without_creating_files() {
        let root = TempRoot::new("database-reserved-names");
        let root_lock = RootLock::acquire(root.path()).expect("acquire root");
        let reserved_names = [
            "CON",
            "con.db",
            "COM1",
            "com9.sqlite",
            "LPT1",
            "LpT9.db",
            "COM¹",
            "com¹.db",
            "CoM².sqlite",
            "cOm³.any-extension",
            "LPT¹",
            "lpt¹.db",
            "LpT².sqlite",
            "lPt³.any-extension",
        ];

        for name in reserved_names {
            let path = root.path().join(name);
            assert!(matches!(
                root_lock.create_and_pin_database(&path),
                Err(DatabasePinError::UnsafeChildName(_))
            ));
            assert!(
                fs::symlink_metadata(&path).is_err(),
                "reserved name created a filesystem entry: {name}"
            );
        }
    }

    #[test]
    fn interior_nul_paths_never_address_or_create_their_truncated_targets() {
        let root = TempRoot::new("database-interior-nul");

        let mut poisoned_root = root.path().as_os_str().to_os_string();
        poisoned_root.push("\0-shadow-root");
        let before_poisoned_root = wide_call_count();
        assert!(matches!(
            RootLock::acquire(Path::new(&poisoned_root)),
            Err(RootLockError::UnsupportedRoot { .. })
        ));
        assert_eq!(wide_call_count(), before_poisoned_root);

        let root_lock = RootLock::acquire(root.path()).expect("acquire unpoisoned root");
        let existing = root.path().join("existing.db");
        let moved = root.path().join("existing.moved.db");
        fs::write(&existing, b"original").expect("seed truncation target");

        let nul_cases = [
            ("existing.db", "shadow"),
            ("fresh.db", "shadow"),
            ("NUL", ".db"),
            ("COM¹", ".sqlite"),
        ];
        let before_poisoned_children = wide_call_count();
        for (prefix, suffix) in nul_cases {
            let mut name = OsString::from(prefix);
            name.push("\0");
            name.push(suffix);
            let poisoned = root.path().join(name);
            let result = if prefix == "existing.db" {
                root_lock.pin_existing_database(&poisoned).map(|_| ())
            } else {
                root_lock.create_and_pin_database(&poisoned).map(|_| ())
            };
            assert!(matches!(
                result,
                Err(DatabasePinError::InvalidPath(
                    RootLockError::UnsupportedRoot { .. }
                ))
            ));
        }
        assert_eq!(wide_call_count(), before_poisoned_children);

        assert_eq!(
            fs::read(&existing).expect("existing target unchanged"),
            b"original"
        );
        fs::rename(&existing, &moved).expect("NUL request did not pin truncated target");
        let entries: Vec<OsString> = fs::read_dir(root.path())
            .expect("read isolated root")
            .map(|entry| entry.expect("read directory entry").file_name())
            .collect();
        assert_eq!(entries, [OsString::from("existing.moved.db")]);

        let normal = root.path().join("normal.db");
        fs::write(&normal, b"normal").expect("seed normal database");
        let pin = root_lock
            .pin_existing_database(&normal)
            .expect("pin normal database");
        assert_eq!(pin.path().file_name(), normal.file_name());
        assert!(pin.verify_path_identity().expect("normal identity matches"));
    }
}
