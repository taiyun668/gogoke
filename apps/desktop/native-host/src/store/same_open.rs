//! Route-B B1: one-shot Rust custodian to SQLite Windows VFS handoff.
//!
//! This is deliberately a narrow native boundary.  It does not expose a SQL
//! transport or accept a database filename from an unbound caller.  The root
//! module first validates the direct-child path and owns the DELETE-capable
//! handle; the patched SQLite VFS then consumes that exact handle for the
//! main database open.

use crate::root::{RootIdentity, RootLock, SqliteMainHandle};
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};
use std::path::Path;

const SQLITE_OK: c_int = 0;
const SQLITE_OPEN_READONLY: c_int = 0x0000_0001;
const SQLITE_OPEN_READWRITE: c_int = 0x0000_0002;

unsafe extern "C" {
    fn sqlite3_gogoke_require_main_handle() -> c_int;
    fn sqlite3_gogoke_owner_generation() -> c_ulong;
    fn sqlite3_gogoke_bind_main_handle(
        handle: *mut c_void,
        path: *const c_char,
        created_new: c_int,
        read_write: c_int,
        root_volume: u64,
        root_file_id: *const u8,
        root_file_id_length: c_int,
    ) -> c_int;
    fn sqlite3_gogoke_cancel_main_handle(expected_generation: c_ulong) -> *mut c_void;
    fn sqlite3_gogoke_end_main_open(expected_generation: c_ulong) -> c_int;
    fn sqlite3_gogoke_bound_generation() -> c_ulong;
    #[cfg(test)]
    fn sqlite3_gogoke_bound_vfs_name() -> *const c_char;
    fn sqlite3_gogoke_current_adopted_generation() -> c_ulong;
    fn sqlite3_gogoke_get_close_ledger(
        generation: c_ulong,
        calls: *mut u64,
        result: *mut c_int,
        error: *mut c_ulong,
    ) -> c_int;
    #[cfg(test)]
    fn sqlite3_gogoke_close_ledger_overflow() -> c_ulong;
    #[cfg(test)]
    fn sqlite3_gogoke_auto_extension_count() -> c_ulong;
    #[cfg(test)]
    fn sqlite3_gogoke_test_force_next_close_failure() -> c_int;
    #[cfg(test)]
    fn sqlite3_gogoke_test_enable_full_pathname_reentry() -> c_int;
    #[cfg(test)]
    fn sqlite3_gogoke_test_last_reentry_result() -> c_int;
    fn sqlite3_gogoke_get_open_ledger(
        adopted: *mut u64,
        stock: *mut u64,
        rejected: *mut u64,
        close_success: *mut u64,
        close_unknown: *mut u64,
        last_closed_generation: *mut u64,
    ) -> c_int;
    fn sqlite3_open_v2(
        filename: *const c_char,
        database: *mut *mut c_void,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(database: *mut c_void) -> c_int;
    #[cfg(test)]
    fn sqlite3_auto_extension(entry_point: Option<unsafe extern "C" fn()>) -> c_int;
    fn sqlite3_exec(
        database: *mut c_void,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        callback_arg: *mut c_void,
        error_message: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(pointer: *mut c_void);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenLedger {
    pub adopted: u64,
    pub stock_main_open: u64,
    pub rejected: u64,
    pub close_success: u64,
    pub close_unknown: u64,
    pub last_closed_generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloseLedger {
    pub generation: u64,
    pub calls: u64,
    pub result: bool,
    pub win32_error: u32,
}

#[derive(Debug)]
pub enum SameOpenError {
    UnsupportedPath,
    RequireFailed(c_int),
    BindFailed(c_int),
    OpenFailed(c_int),
    EndFailed(c_int),
    SqliteExec {
        code: c_int,
        message: String,
    },
    CloseFailed {
        sqlite_code: c_int,
        generation: u64,
        calls: u64,
        win32_error: u32,
    },
    CloseUnknown {
        generation: u64,
        calls: u64,
        win32_error: u32,
    },
    CloseLedgerMissing(u64),
    GenerationMissing,
    LedgerFailed(c_int),
}

impl std::fmt::Display for SameOpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SameOpenError {}

fn ledger() -> Result<OpenLedger, SameOpenError> {
    let mut adopted = 0;
    let mut stock = 0;
    let mut rejected = 0;
    let mut close_success = 0;
    let mut close_unknown = 0;
    let mut last_closed_generation = 0;
    // SAFETY: pointers refer to local output slots and the fixed native image
    // owns no Rust memory through this call.
    let rc = unsafe {
        sqlite3_gogoke_get_open_ledger(
            &mut adopted,
            &mut stock,
            &mut rejected,
            &mut close_success,
            &mut close_unknown,
            &mut last_closed_generation,
        )
    };
    if rc != SQLITE_OK {
        return Err(SameOpenError::LedgerFailed(rc));
    }
    Ok(OpenLedger {
        adopted,
        stock_main_open: stock,
        rejected,
        close_success,
        close_unknown,
        last_closed_generation,
    })
}

fn close_ledger(generation: u64) -> Result<CloseLedger, SameOpenError> {
    let mut calls = 0;
    let mut result = 0;
    let mut win32_error: c_ulong = 0;
    // Diagnostic ring, newest-first. Missing generation is fail-closed
    // (CloseLedgerMissing → poison), not a successful close. Overflow is
    // counted in sqlite3_gogoke_close_ledger_overflow; the ring is not the
    // sole durable close receipt.
    let rc = unsafe {
        sqlite3_gogoke_get_close_ledger(
            generation as c_ulong,
            &mut calls,
            &mut result,
            &mut win32_error,
        )
    };
    if rc != SQLITE_OK {
        return Err(SameOpenError::CloseLedgerMissing(generation));
    }
    Ok(CloseLedger {
        generation,
        calls,
        result: result != 0,
        win32_error: win32_error as u32,
    })
}

fn canonical_path(path: &Path) -> Result<CString, SameOpenError> {
    CString::new(
        path.to_str()
            .ok_or(SameOpenError::UnsupportedPath)?
            .as_bytes(),
    )
    .map_err(|_| SameOpenError::UnsupportedPath)
}

/// A database connection whose main file was opened by the patched Windows
/// VFS from the exact handle validated by `RootLock`.
pub struct VerifiedDatabaseConnection<'root> {
    database: *mut c_void,
    custody: SqliteMainHandle<'root>,
    generation: u64,
}

impl VerifiedDatabaseConnection<'_> {
    pub fn path(&self) -> &Path {
        self.custody.path()
    }

    pub fn created_new(&self) -> bool {
        self.custody.created_new()
    }

    pub fn identity(&self) -> &RootIdentity {
        self.custody.identity()
    }

    /// Physical root directory identity retained by the original custody pin.
    /// This is distinct from identity(), which identifies the database file.
    pub(crate) fn root_identity(&self) -> &RootIdentity {
        self.custody.root_identity()
    }

    pub(crate) fn as_ptr(&self) -> *mut c_void {
        self.database
    }

    pub fn execute(&mut self, sql: &str) -> Result<(), SameOpenError> {
        let sql = CString::new(sql).map_err(|_| SameOpenError::UnsupportedPath)?;
        let mut error_message = std::ptr::null_mut();
        // SAFETY: database is live, SQL is NUL-terminated, and no callback is
        // admitted at this boundary.
        let rc = unsafe {
            sqlite3_exec(
                self.database,
                sql.as_ptr(),
                None,
                std::ptr::null_mut(),
                &mut error_message,
            )
        };
        if rc == SQLITE_OK {
            return Ok(());
        }
        let message = if error_message.is_null() {
            String::new()
        } else {
            // SAFETY: SQLite returns a NUL-terminated diagnostic owned by the
            // caller after sqlite3_free.
            let value = unsafe { CStr::from_ptr(error_message) }
                .to_string_lossy()
                .into_owned();
            unsafe { sqlite3_free(error_message.cast()) };
            value
        };
        Err(SameOpenError::SqliteExec { code: rc, message })
    }

    pub fn close_checked(mut self) -> Result<OpenLedger, SameOpenError> {
        let identity = self.custody.root_identity().clone();
        // SAFETY: database is exclusively owned by this wrapper until close.
        let rc = unsafe { sqlite3_close(self.database) };
        // Prevent Drop from retrying sqlite3_close. In particular, a native
        // UNKNOWN close must never cause a second CloseHandle attempt.
        self.database = std::ptr::null_mut();
        let record = match close_ledger(self.generation) {
            Ok(record) => record,
            Err(error) => {
                RootLock::poison_identity(&identity);
                return Err(error);
            }
        };
        if record.calls != 1 {
            RootLock::poison_identity(&identity);
            return Err(SameOpenError::CloseFailed {
                sqlite_code: rc,
                generation: record.generation,
                calls: record.calls,
                win32_error: record.win32_error,
            });
        }
        if !record.result {
            RootLock::poison_identity(&identity);
            return Err(SameOpenError::CloseUnknown {
                generation: record.generation,
                calls: record.calls,
                win32_error: record.win32_error,
            });
        }
        if rc != SQLITE_OK {
            RootLock::poison_identity(&identity);
            return Err(SameOpenError::CloseFailed {
                sqlite_code: rc,
                generation: record.generation,
                calls: record.calls,
                win32_error: record.win32_error,
            });
        }
        ledger()
    }
}

impl Drop for VerifiedDatabaseConnection<'_> {
    fn drop(&mut self) {
        if !self.database.is_null() {
            // Drop cannot report COMMIT/close state.  The native generation
            // ledger records whether the underlying CloseHandle succeeded or
            // remained UNKNOWN independently of this return value.
            unsafe {
                let _ = sqlite3_close(self.database);
            }
            self.database = std::ptr::null_mut();
        }
    }
}

fn open_with_pin(
    pin: SqliteMainHandle<'_>,
) -> Result<VerifiedDatabaseConnection<'_>, SameOpenError> {
    let path = canonical_path(pin.path())?;
    let handle = pin.raw();
    let identity = pin.identity().clone();
    let created_new = pin.created_new();
    // SAFETY: the identity bytes are copied synchronously by the C handoff;
    // the OS handle is still owned by `pin` until adoption is observed.
    let require = unsafe { sqlite3_gogoke_require_main_handle() };
    if require != SQLITE_OK {
        return Err(SameOpenError::RequireFailed(require));
    }
    let bind = unsafe {
        sqlite3_gogoke_bind_main_handle(
            handle,
            path.as_ptr(),
            i32::from(created_new),
            1,
            identity.volume_serial,
            identity.file_id.as_ptr(),
            identity.file_id.len() as c_int,
        )
    };
    if bind != SQLITE_OK {
        unsafe {
            let _ = sqlite3_gogoke_cancel_main_handle(0);
            let _ = sqlite3_gogoke_end_main_open(0);
        }
        return Err(SameOpenError::BindFailed(bind));
    }
    let expected_generation = unsafe { sqlite3_gogoke_bound_generation() } as u64;
    if expected_generation == 0 {
        unsafe {
            let _ = sqlite3_gogoke_cancel_main_handle(0);
            let _ = sqlite3_gogoke_end_main_open(0);
        }
        return Err(SameOpenError::GenerationMissing);
    }

    let mut database = std::ptr::null_mut();
    // SAFETY: the exact same canonical path is bound above; the VFS is the
    // fixed default image and receives no URI or caller-selected VFS.
    let open = unsafe {
        sqlite3_open_v2(
            path.as_ptr(),
            &mut database,
            SQLITE_OPEN_READWRITE,
            std::ptr::null(),
        )
    };
    let adopted_generation = unsafe { sqlite3_gogoke_current_adopted_generation() } as u64;
    let consumed = adopted_generation == expected_generation;
    let end = unsafe { sqlite3_gogoke_end_main_open(expected_generation as c_ulong) };
    if consumed {
        // SQLite now owns the OS handle through winFile; disarm Rust's
        // DatabaseFilePin close path while retaining the root lifetime marker.
        // `pin` is mutable only after the open result is known.
        let mut pin = pin;
        pin.mark_transferred();
        if open != SQLITE_OK {
            if !database.is_null() {
                unsafe {
                    let _ = sqlite3_close(database);
                }
            }
            return Err(SameOpenError::OpenFailed(open));
        }
        if end != SQLITE_OK {
            unsafe {
                let _ = sqlite3_close(database);
            }
            return Err(SameOpenError::EndFailed(end));
        }
        return Ok(VerifiedDatabaseConnection {
            database,
            custody: pin,
            generation: expected_generation,
        });
    }

    // A path/URI/no-context rejection leaves the one-shot handle pending so
    // the custodian can reclaim it.  `pin` remains the owner and will close it.
    unsafe {
        let _ = sqlite3_gogoke_cancel_main_handle(expected_generation as c_ulong);
    }
    if end != SQLITE_OK {
        return Err(SameOpenError::EndFailed(end));
    }
    if !database.is_null() {
        unsafe {
            let _ = sqlite3_close(database);
        }
    }
    Err(SameOpenError::OpenFailed(open))
}

pub fn create_new<'root>(
    root: &'root RootLock,
    path: &Path,
) -> Result<VerifiedDatabaseConnection<'root>, SameOpenError> {
    let pin = root
        .create_and_pin_database(path)
        .map_err(|_| SameOpenError::UnsupportedPath)?
        .into_sqlite_main_handle();
    open_with_pin(pin)
}

pub fn open_existing<'root>(
    root: &'root RootLock,
    path: &Path,
) -> Result<VerifiedDatabaseConnection<'root>, SameOpenError> {
    let pin = root
        .pin_existing_database(path)
        .map_err(|_| SameOpenError::UnsupportedPath)?
        .into_sqlite_main_handle();
    open_with_pin(pin)
}

pub fn open_ledger() -> Result<OpenLedger, SameOpenError> {
    ledger()
}

#[cfg(test)]
pub(crate) fn route_b_test_guard() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLockError;
    use std::sync::MutexGuard;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_guard() -> MutexGuard<'static, ()> {
        route_b_test_guard()
    }

    fn scratch_root(label: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("gogoke-route-b1-{label}-{nonce}"));
        std::fs::create_dir(&root).expect("scratch root");
        root
    }

    #[test]
    fn route_b_api_is_linked_and_reports_ledger() {
        let _guard = test_guard();
        let snapshot = open_ledger().expect("Route-B ledger API");
        assert!(snapshot.stock_main_open <= snapshot.adopted + 1);
    }

    #[test]
    fn strict_main_open_without_custodian_context_is_rejected() {
        let _guard = test_guard();
        let path = CString::new(
            std::env::temp_dir()
                .join("gogoke-route-b-no-context.db")
                .to_string_lossy()
                .as_bytes(),
        )
        .expect("path");
        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        let mut database = std::ptr::null_mut();
        let open = unsafe {
            sqlite3_open_v2(
                path.as_ptr(),
                &mut database,
                SQLITE_OPEN_READWRITE,
                std::ptr::null(),
            )
        };
        assert_ne!(open, SQLITE_OK);
        if !database.is_null() {
            unsafe {
                let _ = sqlite3_close(database);
            }
        }
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(0) }, SQLITE_OK);
    }

    #[test]
    fn rejected_readonly_binding_does_not_poison_the_one_shot_slot() {
        let _guard = test_guard();
        let root_path = scratch_root("readonly-bind");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let pin = root
            .create_and_pin_database(&database_path)
            .expect("database pin")
            .into_sqlite_main_handle();
        let path = canonical_path(pin.path()).expect("canonical path");
        let identity = pin.identity().clone();
        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        let bind = unsafe {
            sqlite3_gogoke_bind_main_handle(
                pin.raw(),
                path.as_ptr(),
                1,
                0,
                identity.volume_serial,
                identity.file_id.as_ptr(),
                16,
            )
        };
        assert_ne!(bind, SQLITE_OK);
        assert!(unsafe { sqlite3_gogoke_cancel_main_handle(0) }.is_null());
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(0) }, SQLITE_OK);
        drop(pin);
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn uri_and_path_mismatch_are_rejected_without_stock_fallback() {
        let _guard = test_guard();
        let root_path = scratch_root("path-rejection");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let pin = root
            .create_and_pin_database(&database_path)
            .expect("database pin")
            .into_sqlite_main_handle();
        let path = canonical_path(pin.path()).expect("canonical path");
        let identity = pin.identity().clone();

        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        let uri = CString::new(format!("{}?mode=rw", path.to_string_lossy())).expect("URI path");
        let uri_bind = unsafe {
            sqlite3_gogoke_bind_main_handle(
                pin.raw(),
                uri.as_ptr(),
                1,
                1,
                identity.volume_serial,
                identity.file_id.as_ptr(),
                16,
            )
        };
        assert_ne!(uri_bind, SQLITE_OK);
        assert!(unsafe { sqlite3_gogoke_cancel_main_handle(0) }.is_null());
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(0) }, SQLITE_OK);

        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        let wrong_path = root_path.join("other.db");
        let wrong = canonical_path(&wrong_path).expect("wrong path");
        assert_eq!(
            unsafe {
                sqlite3_gogoke_bind_main_handle(
                    pin.raw(),
                    wrong.as_ptr(),
                    1,
                    1,
                    identity.volume_serial,
                    identity.file_id.as_ptr(),
                    16,
                )
            },
            SQLITE_OK
        );
        let mut database = std::ptr::null_mut();
        let open = unsafe {
            sqlite3_open_v2(
                path.as_ptr(),
                &mut database,
                SQLITE_OPEN_READWRITE,
                std::ptr::null(),
            )
        };
        assert_ne!(open, SQLITE_OK);
        if !database.is_null() {
            unsafe {
                let _ = sqlite3_close(database);
            }
        }
        let bound = unsafe { sqlite3_gogoke_bound_generation() };
        let reclaimed = unsafe { sqlite3_gogoke_cancel_main_handle(bound) };
        assert_eq!(reclaimed, pin.raw());
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(bound) }, SQLITE_OK);
        drop(pin);
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn disk_main_without_handoff_is_rejected_even_without_strict_session() {
        let _guard = test_guard();
        let root_path = scratch_root("no-context-disk");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let path = canonical_path(&database_path).expect("canonical path");
        let before = open_ledger().expect("ledger before");
        let mut database = std::ptr::null_mut();
        let open = unsafe {
            sqlite3_open_v2(
                path.as_ptr(),
                &mut database,
                SQLITE_OPEN_READWRITE,
                std::ptr::null(),
            )
        };
        assert_ne!(open, SQLITE_OK, "bare disk main open must fail closed");
        if !database.is_null() {
            unsafe {
                let _ = sqlite3_close(database);
            }
        }
        let after = open_ledger().expect("ledger after");
        assert_eq!(after.stock_main_open, before.stock_main_open);
        assert!(after.rejected > before.rejected);
        drop(root);
        assert!(
            !database_path.exists(),
            "rejected open must not create a file"
        );
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn dedicated_disk_main_rejects_second_open_and_attach() {
        let _guard = test_guard();
        let root_path = scratch_root("dedicated-main");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("same-open main");

        let path = canonical_path(&database_path).expect("canonical path");
        let mut second = std::ptr::null_mut();
        let second_open = unsafe {
            sqlite3_open_v2(
                path.as_ptr(),
                &mut second,
                SQLITE_OPEN_READWRITE,
                std::ptr::null(),
            )
        };
        assert_ne!(second_open, SQLITE_OK, "bare second disk open must fail");
        if !second.is_null() {
            unsafe {
                let _ = sqlite3_close(second);
            }
        }

        let attached = root_path.join("attached.db");
        let sql = format!("ATTACH DATABASE '{}' AS attached", attached.display());
        assert!(
            connection.execute(&sql).is_err(),
            "ATTACH must not bypass custody"
        );
        connection.close_checked().expect("checked close");
        drop(root);
        std::fs::remove_file(database_path).expect("main cleanup");
        std::fs::remove_file(attached).ok();
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn live_same_open_handle_denies_rename_delete_and_replace() {
        let _guard = test_guard();
        let root_path = scratch_root("live-pin");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let replacement = root_path.join("replacement.db");
        let renamed = root_path.join("renamed.db");
        let mut connection = create_new(&root, &database_path).expect("same-open main");
        connection
            .execute(
                "PRAGMA foreign_keys=ON;\
                 PRAGMA synchronous=FULL;\
                 PRAGMA journal_mode=WAL;\
                 CREATE TABLE probe(value INTEGER NOT NULL);\
                 BEGIN IMMEDIATE;\
                 INSERT INTO probe(value) VALUES (42);\
                 COMMIT;\
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .expect("WAL write and checkpoint");
        let sidecars = root
            .inspect_database_sidecars(&database_path)
            .expect("sidecar report while live");
        assert_eq!(sidecars.main_identity, *connection.identity());
        std::fs::write(&replacement, b"replacement").expect("scratch replacement");
        assert!(
            std::fs::rename(&database_path, &renamed).is_err(),
            "live SQLite handle must deny rename"
        );
        assert!(
            std::fs::remove_file(&database_path).is_err(),
            "live SQLite handle must deny delete"
        );
        assert!(
            std::fs::rename(&replacement, &database_path).is_err(),
            "live SQLite handle must deny replacement"
        );
        connection
            .execute("SELECT value FROM probe;")
            .expect("identity-pinned handle still queries");
        let ledger = open_ledger().expect("ledger");
        assert_eq!(ledger.stock_main_open, 0);
        connection.close_checked().expect("checked close");
        drop(root);
        std::fs::rename(&database_path, &renamed).expect("rename after close");
        std::fs::rename(&replacement, &database_path).expect("replace after close");
        std::fs::remove_file(&database_path).ok();
        std::fs::remove_file(&renamed).ok();
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn readonly_and_reopen_disk_main_have_no_stock_fallback() {
        let _guard = test_guard();
        let root_path = scratch_root("readonly-reopen");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("same-open main");
        connection
            .execute("CREATE TABLE probe(value INTEGER NOT NULL); INSERT INTO probe(value) VALUES (1);")
            .expect("seed");
        connection.close_checked().expect("checked close");

        let path = canonical_path(&database_path).expect("canonical path");
        let before = open_ledger().expect("ledger before");
        let mut readonly = std::ptr::null_mut();
        let readonly_open = unsafe {
            sqlite3_open_v2(
                path.as_ptr(),
                &mut readonly,
                SQLITE_OPEN_READONLY,
                std::ptr::null(),
            )
        };
        assert_ne!(readonly_open, SQLITE_OK, "readonly disk main must fail closed");
        if !readonly.is_null() {
            unsafe {
                let _ = sqlite3_close(readonly);
            }
        }
        let mut reopen = std::ptr::null_mut();
        let reopen_open = unsafe {
            sqlite3_open_v2(
                path.as_ptr(),
                &mut reopen,
                SQLITE_OPEN_READWRITE,
                std::ptr::null(),
            )
        };
        assert_ne!(reopen_open, SQLITE_OK, "unbound reopen must fail closed");
        if !reopen.is_null() {
            unsafe {
                let _ = sqlite3_close(reopen);
            }
        }
        let after = open_ledger().expect("ledger after");
        assert_eq!(after.stock_main_open, before.stock_main_open);
        assert!(after.rejected >= before.rejected + 2);

        let mut bound = open_existing(&root, &database_path).expect("custodian reopen");
        bound
            .execute("SELECT value FROM probe;")
            .expect("custodian reopen still reads");
        bound.close_checked().expect("checked close");
        drop(root);
        std::fs::remove_file(database_path).ok();
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn foreign_thread_cannot_consume_or_release_bound_handoff() {
        let _guard = test_guard();
        let root_path = scratch_root("thread-affinity");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let pin = root
            .create_and_pin_database(&database_path)
            .expect("database pin")
            .into_sqlite_main_handle();
        let path = canonical_path(pin.path()).expect("canonical path");
        let identity = pin.identity().clone();
        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        assert_eq!(
            unsafe {
                sqlite3_gogoke_bind_main_handle(
                    pin.raw(),
                    path.as_ptr(),
                    1,
                    1,
                    identity.volume_serial,
                    identity.file_id.as_ptr(),
                    16,
                )
            },
            SQLITE_OK
        );
        let expected = unsafe { sqlite3_gogoke_bound_generation() } as u64;
        assert_ne!(expected, 0);
        let raw = pin.raw() as usize;
        let path_bytes = path.as_bytes().to_vec();
        let foreign = thread::spawn(move || {
            let path = CString::new(path_bytes).expect("path");
            let mut database = std::ptr::null_mut();
            let open = unsafe {
                sqlite3_open_v2(
                    path.as_ptr(),
                    &mut database,
                    SQLITE_OPEN_READWRITE,
                    std::ptr::null(),
                )
            };
            if !database.is_null() {
                unsafe {
                    let _ = sqlite3_close(database);
                }
            }
            let cancel = unsafe { sqlite3_gogoke_cancel_main_handle(0) };
            let end = unsafe { sqlite3_gogoke_end_main_open(0) };
            let adopted = unsafe { sqlite3_gogoke_current_adopted_generation() } as u64;
            (open, cancel as usize, end, adopted, raw)
        })
        .join()
        .expect("foreign thread completed");
        assert_ne!(foreign.0, SQLITE_OK);
        assert_eq!(foreign.1, 0, "foreign thread must not reclaim handle");
        assert_ne!(foreign.2, SQLITE_OK, "foreign thread must not end session");
        assert_eq!(foreign.3, 0, "foreign thread must not adopt generation");
        let reclaimed = unsafe { sqlite3_gogoke_cancel_main_handle(expected as c_ulong) };
        assert_eq!(reclaimed as usize, raw);
        assert_eq!(
            unsafe { sqlite3_gogoke_end_main_open(expected as c_ulong) },
            SQLITE_OK
        );
        drop(pin);
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn checked_close_reports_unknown_after_one_native_attempt() {
        let _guard = test_guard();
        let root_path = scratch_root("close-fault");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let connection = create_new(&root, &database_path).expect("same-open main");
        let generation = connection.generation;
        assert_eq!(
            unsafe { sqlite3_gogoke_test_force_next_close_failure() },
            SQLITE_OK
        );
        let result = connection.close_checked();
        assert!(
            matches!(result, Err(SameOpenError::CloseUnknown { generation: observed, calls: 1, .. }) if observed == generation)
        );
        let record = close_ledger(generation).expect("close ledger");
        assert_eq!(record.calls, 1, "fault path must not retry CloseHandle");
        assert!(!record.result);
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn unknown_close_poisons_the_physical_root_against_later_acquire() {
        let _guard = test_guard();
        let root_path = scratch_root("close-poison");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let connection = create_new(&root, &database_path).expect("same-open main");
        let generation = connection.generation;
        let overflow_before = unsafe { sqlite3_gogoke_close_ledger_overflow() };
        assert_eq!(
            unsafe { sqlite3_gogoke_test_force_next_close_failure() },
            SQLITE_OK
        );
        assert!(matches!(
            connection.close_checked(),
            Err(SameOpenError::CloseUnknown { generation: observed, calls: 1, .. }) if observed == generation
        ));
        let record = close_ledger(generation).expect("current generation close ledger");
        assert_eq!(record.calls, 1, "poisoning must not retry the native close");
        assert!(!record.result, "unknown close must not become successful evidence");
        // Overflow is process-cumulative; other tests may already have filled the
        // diagnostic ring. One close can append at most one replacement record.
        let overflow_after = unsafe { sqlite3_gogoke_close_ledger_overflow() };
        assert!(
            overflow_after == overflow_before || overflow_after == overflow_before.wrapping_add(1),
            "one native close must not add multiple overflow records"
        );
        drop(root);
        let reused = RootLock::acquire(&root_path);
        assert!(
            matches!(reused, Err(RootLockError::Poisoned { .. })),
            "UNKNOWN close must reject later operations on the same physical root"
        );
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn test_image_compiles_route_b_testing_hooks() {
        assert_eq!(env!("GOGOKE_ROUTE_B_TESTING_COMPILED"), "1");
    }

    #[test]
    fn stale_owner_end_does_not_clear_a_newer_owner() {
        let _guard = test_guard();
        let root_path = scratch_root("stale-end");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let pin = root
            .create_and_pin_database(&database_path)
            .expect("database pin")
            .into_sqlite_main_handle();
        let path = canonical_path(pin.path()).expect("canonical path");
        let identity = pin.identity().clone();
        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        assert_eq!(
            unsafe {
                sqlite3_gogoke_bind_main_handle(
                    pin.raw(),
                    path.as_ptr(),
                    1,
                    1,
                    identity.volume_serial,
                    identity.file_id.as_ptr(),
                    16,
                )
            },
            SQLITE_OK
        );
        let first = unsafe { sqlite3_gogoke_bound_generation() };
        assert_ne!(first, 0);
        assert_eq!(
            unsafe { sqlite3_gogoke_cancel_main_handle(first) },
            pin.raw()
        );
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(first) }, SQLITE_OK);

        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        assert_eq!(
            unsafe {
                sqlite3_gogoke_bind_main_handle(
                    pin.raw(),
                    path.as_ptr(),
                    1,
                    1,
                    identity.volume_serial,
                    identity.file_id.as_ptr(),
                    16,
                )
            },
            SQLITE_OK
        );
        let second = unsafe { sqlite3_gogoke_owner_generation() };
        assert_ne!(second, 0);
        assert_ne!(second, first);
        assert_ne!(
            unsafe { sqlite3_gogoke_end_main_open(first) },
            SQLITE_OK,
            "stale end must not release the newer owner"
        );
        assert_eq!(unsafe { sqlite3_gogoke_owner_generation() }, second);
        let reclaimed = unsafe { sqlite3_gogoke_cancel_main_handle(second) };
        assert_eq!(reclaimed, pin.raw());
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(second) }, SQLITE_OK);
        drop(pin);
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn same_thread_fullpathname_reentry_cannot_steal_pending_handoff() {
        let _guard = test_guard();
        let root_path = scratch_root("reentry");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let pin = root
            .create_and_pin_database(&database_path)
            .expect("database pin")
            .into_sqlite_main_handle();
        assert_eq!(
            unsafe { sqlite3_gogoke_test_enable_full_pathname_reentry() },
            SQLITE_OK
        );
        let connection = open_with_pin(pin).expect("adopted open after rejected reentry");
        assert_ne!(
            unsafe { sqlite3_gogoke_test_last_reentry_result() },
            SQLITE_OK,
            "reentrant same-thread open must fail closed"
        );
        connection.close_checked().expect("checked close");
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    unsafe extern "C" fn test_auto_extension() {}

    #[test]
    fn strict_session_rejects_registration_and_pre_registered_callback() {
        let _guard = test_guard();
        assert_eq!(unsafe { sqlite3_gogoke_require_main_handle() }, SQLITE_OK);
        assert_ne!(
            unsafe { sqlite3_auto_extension(Some(test_auto_extension)) },
            SQLITE_OK
        );
        assert_eq!(unsafe { sqlite3_gogoke_end_main_open(0) }, SQLITE_OK);

        assert_ne!(
            unsafe { sqlite3_auto_extension(Some(test_auto_extension)) },
            SQLITE_OK
        );
        assert_eq!(unsafe { sqlite3_gogoke_auto_extension_count() }, 0);
        let root_path = scratch_root("auto-extension");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let pin = root
            .create_and_pin_database(&database_path)
            .expect("database pin")
            .into_sqlite_main_handle();
        let connection = open_with_pin(pin).expect("strict open after denied extension");
        assert_eq!(unsafe { sqlite3_gogoke_auto_extension_count() }, 0);
        connection.close_checked().expect("checked close");
        assert_eq!(unsafe { sqlite3_gogoke_auto_extension_count() }, 0);
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }

    #[test]
    fn load_extension_sql_is_unavailable_on_the_adopted_connection() {
        let _guard = test_guard();
        let root_path = scratch_root("load-extension");
        let root = RootLock::acquire(&root_path).expect("root lock");
        let database_path = root_path.join("main.db");
        let mut connection = create_new(&root, &database_path).expect("same-open main");
        let error = connection
            .execute("SELECT load_extension('x');")
            .expect_err("load_extension SQL must be omitted");
        match error {
            SameOpenError::SqliteExec { code, message } => {
                assert_ne!(code, SQLITE_OK);
                assert!(
                    message.contains("no such function") || message.contains("load_extension"),
                    "unexpected load_extension diagnostic: {message}"
                );
            }
            other => panic!("expected SQLite exec failure, got {other:?}"),
        }
        connection.close_checked().expect("checked close");
        drop(root);
        std::fs::remove_file(database_path).expect("scratch cleanup");
        std::fs::remove_dir(root_path).expect("scratch root cleanup");
    }
}
