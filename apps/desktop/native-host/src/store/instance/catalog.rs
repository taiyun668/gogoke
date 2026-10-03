//! Native-controlled provider catalog for F.1 registration. Wire callers
//! select a driver ID only; no path, version or digest crosses this boundary.

use super::registry::{ProgramObservation, RegistryError};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser};
use std::ffi::{c_void, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;

const CODEX_DRIVER: &str = "codex";
const CODEX_VERSION: &str = "0.160.0";
const ROOT_PACKAGE: &str = "@openai/codex";
const PLATFORM_VERSION: &str = "0.160.0-win32-x64";
const MAX_PACKAGE_BYTES: u64 = 64 * 1024;
const REPARSE_POINT: u32 = 0x400;

// Microsoft FOLDERID_RoamingAppData (knownfolderid.h). Avoid APPDATA, which
// is an inherited environment value rather than the current user's folder.
#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}
const ROAMING_APP_DATA: Guid = Guid {
    data1: 0x3eb685db,
    data2: 0x65f9,
    data3: 0x4cf6,
    data4: [0xa0, 0x3a, 0xe3, 0xef, 0x65, 0x72, 0x9f, 0x3d],
};
#[link(name = "shell32")]
extern "system" {
    fn SHGetKnownFolderPath(
        folder: *const Guid,
        flags: u32,
        token: *mut c_void,
        path: *mut *mut u16,
    ) -> i32;
}
#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemFree(pointer: *mut c_void);
}

#[derive(Debug)]
pub(crate) enum CatalogError {
    UnknownDriver,
    UnsupportedVersion,
    PackageIdentity,
    PackageFormat,
    IdentityChanged,
    KnownFolder(i32),
    Io(io::Error),
    Json(AtomicError),
    Program(RegistryError),
}
impl From<io::Error> for CatalogError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}
impl From<AtomicError> for CatalogError {
    fn from(error: AtomicError) -> Self {
        Self::Json(error)
    }
}
impl From<RegistryError> for CatalogError {
    fn from(error: RegistryError) -> Self {
        Self::Program(error)
    }
}

fn roaming_app_data() -> Result<PathBuf, CatalogError> {
    let mut raw = ptr::null_mut();
    // SAFETY: The GUID has the Windows ABI layout. A successful call returns
    // a NUL-terminated CoTaskMem allocation, freed before this function exits.
    let hr = unsafe { SHGetKnownFolderPath(&ROAMING_APP_DATA, 0, ptr::null_mut(), &mut raw) };
    if hr < 0 {
        return Err(CatalogError::KnownFolder(hr));
    }
    if raw.is_null() {
        return Err(CatalogError::IdentityChanged);
    }
    let result = (|| {
        let mut length = 0usize;
        // Windows paths are bounded by the UTF-16 extended-path limit.
        while length < 32_767 && unsafe { *raw.add(length) } != 0 {
            length += 1;
        }
        if length == 0 || length == 32_767 {
            return Err(CatalogError::IdentityChanged);
        }
        // SAFETY: `raw` is the live allocation returned by Windows; the scan
        // above found a terminator within the maximum Windows path length.
        let units = unsafe { std::slice::from_raw_parts(raw, length) };
        Ok(PathBuf::from(OsString::from_wide(units)))
    })();
    // SAFETY: SHGetKnownFolderPath specifies CoTaskMemFree for this allocation.
    unsafe {
        CoTaskMemFree(raw.cast());
    }
    result
}

fn plain_dir(path: &Path) -> Result<(), CatalogError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(CatalogError::IdentityChanged);
    }
    Ok(())
}

fn package_fact(path: &Path, expected_version: &str) -> Result<(), CatalogError> {
    let path_before = fs::symlink_metadata(path)?;
    if !path_before.is_file()
        || path_before.file_attributes() & REPARSE_POINT != 0
        || path_before.len() > MAX_PACKAGE_BYTES
    {
        return Err(CatalogError::IdentityChanged);
    }
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file()
        || before.file_attributes() & REPARSE_POINT != 0
        || before.len() > MAX_PACKAGE_BYTES
    {
        return Err(CatalogError::IdentityChanged);
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_PACKAGE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    let path_after = fs::symlink_metadata(path)?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES
        || before.len() != bytes.len() as u64
        || before.len() != after.len()
        || before.modified()? != after.modified()?
        || path_before.len() != path_after.len()
        || path_before.modified()? != path_after.modified()?
        || path_after.file_attributes() & REPARSE_POINT != 0
    {
        return Err(CatalogError::IdentityChanged);
    }
    let source = std::str::from_utf8(&bytes).map_err(|_| CatalogError::PackageFormat)?;
    let Json::Object(mut fields) = Parser::parse(source)? else {
        return Err(CatalogError::PackageFormat);
    };
    let name = fields.remove(&JsonString::from_str("name"));
    let version = fields.remove(&JsonString::from_str("version"));
    let Some(Json::String(name)) = name else {
        return Err(CatalogError::PackageFormat);
    };
    let Some(Json::String(version)) = version else {
        return Err(CatalogError::PackageFormat);
    };
    if name.to_well_formed_string().as_deref() != Some(ROOT_PACKAGE) {
        return Err(CatalogError::PackageIdentity);
    }
    if version.to_well_formed_string().as_deref() != Some(expected_version) {
        return Err(CatalogError::UnsupportedVersion);
    }
    Ok(())
}

struct LocatedProgram {
    application: PathBuf,
    observation: ProgramObservation,
}

fn discover_at(roaming: &Path, driver_id: &str) -> Result<LocatedProgram, CatalogError> {
    if driver_id != CODEX_DRIVER {
        return Err(CatalogError::UnknownDriver);
    }
    if !roaming.is_absolute() {
        return Err(CatalogError::IdentityChanged);
    }
    plain_dir(roaming)?;
    let mut path = roaming.to_path_buf();
    for part in ["npm", "node_modules", "@openai", "codex"] {
        path.push(part);
        plain_dir(&path)?;
    }
    let root_package = path.join("package.json");
    package_fact(&root_package, CODEX_VERSION)?;
    for part in ["node_modules", "@openai", "codex-win32-x64"] {
        path.push(part);
        plain_dir(&path)?;
    }
    let platform_package = path.join("package.json");
    package_fact(&platform_package, PLATFORM_VERSION)?;
    for part in ["vendor", "x86_64-pc-windows-msvc", "bin"] {
        path.push(part);
        plain_dir(&path)?;
    }
    let binary = path.join("codex.exe");
    let observed = ProgramObservation::observe(&binary, CODEX_VERSION)?;
    // A package replacement during binary measurement cannot establish a pin.
    package_fact(&root_package, CODEX_VERSION)?;
    package_fact(&platform_package, PLATFORM_VERSION)?;
    Ok(LocatedProgram { application: binary, observation: observed })
}

/// Native F.1 program observation for `K-INSTANCE/register`. Only `driverId`
/// comes from the wire. H must re-observe and compare these bytes at launch.
pub(crate) fn discover_program(driver_id: &str) -> Result<ProgramObservation, CatalogError> {
    if driver_id != CODEX_DRIVER {
        return Err(CatalogError::UnknownDriver);
    }
    Ok(discover_at(&roaming_app_data()?, driver_id)?.observation)
}

/// H resolves its launch path from the same native catalog and compares the
/// currently observed bytes to the durable instance pin. The path never
/// crosses the User/service wire; process custody must recheck the executable
/// at suspended creation and verify the launched image object again.
fn locate_pinned_at(roaming: &Path, driver_id: &str, digest: &str,
    version: &str) -> Result<PathBuf, CatalogError> {
    let located = discover_at(roaming, driver_id)?;
    if !located.observation.matches_pin(digest, version) {
        return Err(CatalogError::IdentityChanged);
    }
    Ok(located.application)
}

pub(crate) fn locate_pinned_program(driver_id: &str, digest: &str,
    version: &str) -> Result<PathBuf, CatalogError> {
    locate_pinned_at(&roaming_app_data()?, driver_id, digest, version)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&Path, &Path, &Path, &Path)) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "gogoke-codex-catalog-{}-{nonce}",
            std::process::id()
        ));
        let npm = root.join("npm/node_modules/@openai/codex");
        let platform = npm.join("node_modules/@openai/codex-win32-x64");
        let binary = platform.join("vendor/x86_64-pc-windows-msvc/bin/codex.exe");
        fs::create_dir_all(binary.parent().unwrap()).unwrap();
        let root_package = npm.join("package.json");
        let platform_package = platform.join("package.json");
        fs::write(
            &root_package,
            br#"{"name":"@openai/codex","version":"0.160.0"}"#,
        )
        .unwrap();
        fs::write(
            &platform_package,
            br#"{"name":"@openai/codex","version":"0.160.0-win32-x64"}"#,
        )
        .unwrap();
        fs::write(&binary, b"controlled fixture executable bytes").unwrap();
        run(&root, &root_package, &platform_package, &binary);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn controlled_layout_observes_only_pinned_codex() {
        fixture(|root, _, _, _| {
            assert!(discover_at(root, "codex").is_ok());
            assert!(matches!(
                discover_at(root, "claude"),
                Err(CatalogError::UnknownDriver)
            ));
        });
    }

    #[test]
    fn launch_location_requires_the_same_native_catalog_pin() {
        fixture(|root, _, _, binary| {
            let digest = crate::store::digest::content_hash(b"controlled fixture executable bytes");
            assert_eq!(locate_pinned_at(root, "codex", &digest, CODEX_VERSION).unwrap(), binary);
            assert!(matches!(locate_pinned_at(root, "codex", "sha256:changed", CODEX_VERSION),
                Err(CatalogError::IdentityChanged)));
            fs::write(binary, b"changed fixture executable bytes").unwrap();
            assert!(matches!(locate_pinned_at(root, "codex", &digest, CODEX_VERSION),
                Err(CatalogError::IdentityChanged)));
        });
    }

    #[test]
    fn rejects_root_or_platform_version_mismatch_and_missing_program() {
        fixture(|root, root_package, platform_package, binary| {
            fs::write(
                root_package,
                br#"{"name":"@openai/codex","version":"0.150.0"}"#,
            )
            .unwrap();
            assert!(matches!(
                discover_at(root, "codex"),
                Err(CatalogError::UnsupportedVersion)
            ));
            fs::write(
                root_package,
                br#"{"name":"@openai/codex","version":"0.160.0"}"#,
            )
            .unwrap();
            fs::write(
                platform_package,
                br#"{"name":"@openai/codex","version":"0.150.0-win32-x64"}"#,
            )
            .unwrap();
            assert!(matches!(
                discover_at(root, "codex"),
                Err(CatalogError::UnsupportedVersion)
            ));
            fs::write(
                platform_package,
                br#"{"name":"@openai/codex","version":"0.160.0-win32-x64"}"#,
            )
            .unwrap();
            fs::remove_file(binary).unwrap();
            assert!(matches!(
                discover_at(root, "codex"),
                Err(CatalogError::Program(RegistryError::Io(_)))
            ));
        });
    }

    #[test]
    fn rejects_claimed_package_name_and_duplicate_json_version() {
        fixture(|root, root_package, _, _| {
            fs::write(root_package, br#"{"name":"not-codex","version":"0.160.0"}"#).unwrap();
            assert!(matches!(
                discover_at(root, "codex"),
                Err(CatalogError::PackageIdentity)
            ));
            fs::write(
                root_package,
                br#"{"name":"@openai/codex","version":"0.160.0","version":"0.160.0"}"#,
            )
            .unwrap();
            assert!(matches!(
                discover_at(root, "codex"),
                Err(CatalogError::Json(_))
            ));
        });
    }
}
