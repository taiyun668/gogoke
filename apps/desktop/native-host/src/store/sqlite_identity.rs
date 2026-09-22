use std::ffi::{c_char, c_int, c_void, CStr};

pub const SQLITE_VERSION: &str = "3.53.2";
pub const SQLITE_SOURCE_ID: &str =
    "2026-06-03 19:12:13 d6e03d8c777cfa2d35e3b60d8ec3e0187f3e9f99d8e2ee9cac695fd6fcdf1a24";
pub const SQLITE3_C_SHA256: &str = env!("GOGOKE_SQLITE3_C_SHA256");
pub const SQLITE3_H_SHA256: &str = env!("GOGOKE_SQLITE3_H_SHA256");
pub const SQLITE3_PATCH_SHA256: &str = env!("GOGOKE_SQLITE3_PATCH_SHA256");
pub const SQLITE3_GENERATED_C_SHA256: &str = env!("GOGOKE_SQLITE3_GENERATED_C_SHA256");

const REQUIRED_OPTIONS: &[&[u8]] = &[
    b"THREADSAFE=1\0",
    b"DQS=0\0",
    b"DEFAULT_FOREIGN_KEYS\0",
    b"DEFAULT_WAL_SYNCHRONOUS=2\0",
    b"OMIT_LOAD_EXTENSION\0",
];

unsafe extern "C" {
    fn sqlite3_libversion() -> *const c_char;
    fn sqlite3_sourceid() -> *const c_char;
    fn sqlite3_threadsafe() -> c_int;
    fn sqlite3_compileoption_used(option: *const c_char) -> c_int;
    fn sqlite3_open(filename: *const c_char, database: *mut *mut c_void) -> c_int;
    fn sqlite3_close(database: *mut c_void) -> c_int;
    fn sqlite3_prepare_v2(
        database: *mut c_void,
        sql: *const c_char,
        bytes: c_int,
        statement: *mut *mut c_void,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_step(statement: *mut c_void) -> c_int;
    fn sqlite3_column_int(statement: *mut c_void, column: c_int) -> c_int;
    fn sqlite3_finalize(statement: *mut c_void) -> c_int;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SqliteBuildIdentity {
    pub version: String,
    pub source_id: String,
    pub threadsafe: bool,
    pub required_options: Vec<String>,
    pub sqlite3_c_sha256: &'static str,
    pub sqlite3_h_sha256: &'static str,
    pub sqlite3_patch_sha256: &'static str,
    pub sqlite3_generated_c_sha256: &'static str,
}

#[derive(Debug)]
pub enum SqliteIdentityError {
    NullIdentity(&'static str),
    InvalidUtf8(&'static str),
    VersionMismatch {
        expected: &'static str,
        actual: String,
    },
    SourceMismatch {
        expected: &'static str,
        actual: String,
    },
    ThreadsafeDisabled,
    MissingCompileOption(String),
    OpenFailed(c_int),
    PrepareFailed {
        pragma: &'static str,
        code: c_int,
    },
    StepFailed {
        pragma: &'static str,
        code: c_int,
    },
    DefaultMismatch {
        pragma: &'static str,
        expected: c_int,
        actual: c_int,
    },
}

impl std::fmt::Display for SqliteIdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for SqliteIdentityError {}

fn pragma_integer(
    database: *mut c_void,
    pragma: &'static str,
    sql: &'static [u8],
) -> Result<c_int, SqliteIdentityError> {
    let mut statement = std::ptr::null_mut();
    // SAFETY: database is open and sql is static NUL-terminated text.
    let prepare = unsafe {
        sqlite3_prepare_v2(
            database,
            sql.as_ptr().cast(),
            -1,
            &mut statement,
            std::ptr::null_mut(),
        )
    };
    if prepare != 0 {
        return Err(SqliteIdentityError::PrepareFailed {
            pragma,
            code: prepare,
        });
    }
    // SAFETY: statement was returned by SQLite.
    let step = unsafe { sqlite3_step(statement) };
    if step != 100 {
        // SAFETY: statement is owned by this function.
        unsafe { sqlite3_finalize(statement) };
        return Err(SqliteIdentityError::StepFailed { pragma, code: step });
    }
    // SAFETY: the PRAGMA row contains one integer column.
    let value = unsafe { sqlite3_column_int(statement, 0) };
    // SAFETY: statement is owned by this function.
    unsafe { sqlite3_finalize(statement) };
    Ok(value)
}

fn verify_connection_defaults() -> Result<(), SqliteIdentityError> {
    let mut database = std::ptr::null_mut();
    // SAFETY: filename is static NUL-terminated text and output is local storage.
    let open = unsafe { sqlite3_open(c":memory:".as_ptr(), &mut database) };
    if open != 0 {
        return Err(SqliteIdentityError::OpenFailed(open));
    }
    let result = (|| {
        for (pragma, sql, expected) in [
            ("foreign_keys", b"PRAGMA foreign_keys;\0".as_slice(), 1),
            ("trusted_schema", b"PRAGMA trusted_schema;\0".as_slice(), 0),
        ] {
            let actual = pragma_integer(database, pragma, sql)?;
            if actual != expected {
                return Err(SqliteIdentityError::DefaultMismatch {
                    pragma,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    })();
    // SAFETY: database was opened in this function.
    unsafe { sqlite3_close(database) };
    result
}

fn c_identity(pointer: *const c_char, label: &'static str) -> Result<String, SqliteIdentityError> {
    if pointer.is_null() {
        return Err(SqliteIdentityError::NullIdentity(label));
    }
    // SAFETY: SQLite identity functions return process-lifetime NUL-terminated strings.
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| SqliteIdentityError::InvalidUtf8(label))
}

pub fn verify_sqlite_build_identity() -> Result<SqliteBuildIdentity, SqliteIdentityError> {
    // SAFETY: pure SQLite identity/configuration inspection after static initialization.
    let version = c_identity(unsafe { sqlite3_libversion() }, "sqlite3_libversion")?;
    if version != SQLITE_VERSION {
        return Err(SqliteIdentityError::VersionMismatch {
            expected: SQLITE_VERSION,
            actual: version,
        });
    }
    // SAFETY: pure SQLite identity/configuration inspection after static initialization.
    let source_id = c_identity(unsafe { sqlite3_sourceid() }, "sqlite3_sourceid")?;
    if source_id != SQLITE_SOURCE_ID {
        return Err(SqliteIdentityError::SourceMismatch {
            expected: SQLITE_SOURCE_ID,
            actual: source_id,
        });
    }
    // SAFETY: sqlite3_threadsafe has no arguments and no side effects.
    if unsafe { sqlite3_threadsafe() } != 1 {
        return Err(SqliteIdentityError::ThreadsafeDisabled);
    }

    let mut observed = Vec::with_capacity(REQUIRED_OPTIONS.len());
    for option in REQUIRED_OPTIONS {
        // SAFETY: every entry is a static NUL-terminated ASCII option name.
        if unsafe { sqlite3_compileoption_used(option.as_ptr().cast()) } != 1 {
            let name = CStr::from_bytes_with_nul(option)
                .expect("static option is NUL terminated")
                .to_string_lossy()
                .into_owned();
            return Err(SqliteIdentityError::MissingCompileOption(name));
        }
        observed.push(
            CStr::from_bytes_with_nul(option)
                .expect("static option is NUL terminated")
                .to_string_lossy()
                .into_owned(),
        );
    }
    verify_connection_defaults()?;

    Ok(SqliteBuildIdentity {
        version,
        source_id,
        threadsafe: true,
        required_options: observed,
        sqlite3_c_sha256: SQLITE3_C_SHA256,
        sqlite3_h_sha256: SQLITE3_H_SHA256,
        sqlite3_patch_sha256: SQLITE3_PATCH_SHA256,
        sqlite3_generated_c_sha256: SQLITE3_GENERATED_C_SHA256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_sqlite_image_reports_the_admitted_identity_and_hardening() {
        let identity = verify_sqlite_build_identity().expect("fixed SQLite image");
        assert_eq!(identity.version, SQLITE_VERSION);
        assert_eq!(identity.source_id, SQLITE_SOURCE_ID);
        assert!(identity.threadsafe);
        assert_eq!(identity.required_options.len(), REQUIRED_OPTIONS.len());
        assert_eq!(identity.sqlite3_c_sha256, SQLITE3_C_SHA256);
        assert_eq!(identity.sqlite3_h_sha256, SQLITE3_H_SHA256);
        assert_eq!(identity.sqlite3_patch_sha256, SQLITE3_PATCH_SHA256);
        assert_eq!(
            identity.sqlite3_generated_c_sha256,
            SQLITE3_GENERATED_C_SHA256
        );
    }
}
