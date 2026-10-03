//! Fixed native observations for the four additional Design 37 CLI drivers.
//! The caller supplies only a driver ID. This module does not inspect PATH,
//! run a CLI, or accept a path/version from the wire.

use super::registry::{ProgramObservation, RegistryError};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser};
use std::ffi::{c_void, OsString};
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;

const REPARSE_POINT: u32 = 0x400;
const MAX_PACKAGE_BYTES: u64 = 64 * 1024;

#[repr(C)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

// FOLDERID_RoamingAppData, FOLDERID_LocalAppData and FOLDERID_Profile.
const ROAMING: Guid = Guid { data1: 0x3eb685db, data2: 0x65f9, data3: 0x4cf6,
    data4: [0xa0, 0x3a, 0xe3, 0xef, 0x65, 0x72, 0x9f, 0x3d] };
const LOCAL: Guid = Guid { data1: 0xf1b32785, data2: 0x6fba, data3: 0x4fcf,
    data4: [0x9d, 0x55, 0x7b, 0x8e, 0x7f, 0x15, 0x70, 0x91] };
const PROFILE: Guid = Guid { data1: 0x5e6c858f, data2: 0x0e22, data3: 0x4760,
    data4: [0x9a, 0xfe, 0xea, 0x33, 0x17, 0xb6, 0x71, 0x73] };

#[link(name = "shell32")]
extern "system" {
    fn SHGetKnownFolderPath(folder: *const Guid, flags: u32, token: *mut c_void,
        path: *mut *mut u16) -> i32;
}
#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemFree(pointer: *mut c_void);
}

#[derive(Debug)]
pub(crate) enum ProviderCatalogError {
    UnknownDriver,
    NotInstalled(io::Error),
    UnsupportedObservation(&'static str),
    UnsupportedVersion,
    PackageIdentity,
    PackageFormat,
    IdentityChanged,
    KnownFolder(i32),
    Io(io::Error),
    Json(AtomicError),
    Utf8(std::str::Utf8Error),
    Program(RegistryError),
}

impl From<io::Error> for ProviderCatalogError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<AtomicError> for ProviderCatalogError {
    fn from(error: AtomicError) -> Self { Self::Json(error) }
}
impl From<RegistryError> for ProviderCatalogError {
    fn from(error: RegistryError) -> Self { Self::Program(error) }
}

fn known_folder(folder: &Guid) -> Result<PathBuf, ProviderCatalogError> {
    let mut raw = ptr::null_mut();
    // SAFETY: The GUID uses the Windows ABI; Windows allocates the returned
    // NUL-terminated path with CoTaskMem, released below.
    let hr = unsafe { SHGetKnownFolderPath(folder, 0, ptr::null_mut(), &mut raw) };
    if hr < 0 { return Err(ProviderCatalogError::KnownFolder(hr)); }
    if raw.is_null() { return Err(ProviderCatalogError::IdentityChanged); }
    let result = (|| {
        let mut length = 0usize;
        while length < 32_767 && unsafe { *raw.add(length) } != 0 { length += 1; }
        if length == 0 || length == 32_767 {
            return Err(ProviderCatalogError::IdentityChanged);
        }
        // SAFETY: The scan above found a terminator in the live allocation.
        let units = unsafe { std::slice::from_raw_parts(raw, length) };
        let path = PathBuf::from(OsString::from_wide(units));
        if !path.is_absolute() { return Err(ProviderCatalogError::IdentityChanged); }
        Ok(path)
    })();
    // SAFETY: Required deallocator for SHGetKnownFolderPath's allocation.
    unsafe { CoTaskMemFree(raw.cast()); }
    result
}

fn metadata(path: &Path) -> Result<fs::Metadata, ProviderCatalogError> {
    fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ProviderCatalogError::NotInstalled(error)
        } else { ProviderCatalogError::Io(error) }
    })
}

fn plain_dir(path: &Path) -> Result<(), ProviderCatalogError> {
    let item = metadata(path)?;
    if !item.is_dir() || item.file_attributes() & REPARSE_POINT != 0 {
        return Err(ProviderCatalogError::IdentityChanged);
    }
    Ok(())
}

fn plain_file(path: &Path) -> Result<(), ProviderCatalogError> {
    let item = metadata(path)?;
    if !item.is_file() || item.file_attributes() & REPARSE_POINT != 0 {
        return Err(ProviderCatalogError::IdentityChanged);
    }
    Ok(())
}

fn enter(path: &mut PathBuf, components: &[&str]) -> Result<(), ProviderCatalogError> {
    for component in components {
        path.push(component);
        plain_dir(path)?;
    }
    Ok(())
}

fn package_fact(path: &Path, expected_name: &str,
    expected_version: &str) -> Result<(), ProviderCatalogError> {
    let path_before = metadata(path)?;
    if !path_before.is_file() || path_before.file_attributes() & REPARSE_POINT != 0
        || path_before.len() > MAX_PACKAGE_BYTES {
        return Err(ProviderCatalogError::IdentityChanged);
    }
    let mut file = File::open(path)?;
    let before = file.metadata()?;
    if !before.is_file() || before.file_attributes() & REPARSE_POINT != 0
        || before.len() > MAX_PACKAGE_BYTES {
        return Err(ProviderCatalogError::IdentityChanged);
    }
    let mut bytes = Vec::new();
    file.by_ref().take(MAX_PACKAGE_BYTES + 1).read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    let path_after = metadata(path)?;
    if bytes.len() as u64 > MAX_PACKAGE_BYTES || before.len() != bytes.len() as u64
        || before.len() != after.len() || before.modified()? != after.modified()?
        || path_before.len() != path_after.len()
        || path_before.modified()? != path_after.modified()?
        || path_after.file_attributes() & REPARSE_POINT != 0 {
        return Err(ProviderCatalogError::IdentityChanged);
    }
    let source = std::str::from_utf8(&bytes).map_err(ProviderCatalogError::Utf8)?;
    let Json::Object(mut fields) = Parser::parse(source)? else {
        return Err(ProviderCatalogError::PackageFormat);
    };
    let Some(Json::String(name)) = fields.remove(&JsonString::from_str("name")) else {
        return Err(ProviderCatalogError::PackageFormat);
    };
    let Some(Json::String(version)) = fields.remove(&JsonString::from_str("version")) else {
        return Err(ProviderCatalogError::PackageFormat);
    };
    if name.to_well_formed_string().as_deref() != Some(expected_name) {
        return Err(ProviderCatalogError::PackageIdentity);
    }
    if version.to_well_formed_string().as_deref() != Some(expected_version) {
        return Err(ProviderCatalogError::UnsupportedVersion);
    }
    Ok(())
}

fn npm_binary(root: &mut PathBuf, package_parts: &[&str],
    root_name: &str, platform_name: &str, version: &str,
    executable: &str) -> Result<(ProgramObservation, PathBuf), ProviderCatalogError> {
    enter(root, &["npm", "node_modules"])?;
    enter(root, package_parts)?;
    let root_manifest = root.join("package.json");
    package_fact(&root_manifest, root_name, version)?;
    let mut platform = root.clone();
    enter(&mut platform, &["node_modules"])?;
    let platform_parts: Vec<&str> = platform_name.split('/').collect();
    enter(&mut platform, &platform_parts)?;
    let platform_manifest = platform.join("package.json");
    package_fact(&platform_manifest, platform_name, version)?;
    enter(root, &["bin"])?;
    let application = root.join(executable);
    plain_file(&application)?;
    let observation = ProgramObservation::observe(&application, version)?;
    // Detect a package swap while measuring executable bytes.
    package_fact(&root_manifest, root_name, version)?;
    package_fact(&platform_manifest, platform_name, version)?;
    Ok((observation, application))
}

/// Native F.1 observation and exact H application path for one fixed driver.
/// F stores the observation; H calls this again and compares the same pin.
/// Grok's fixed bytes are tied to the same ordinary-view binary's original
/// `--version` output, not to its unrelated old npm package or absent PE version.
/// Unknown replacement bytes are refused. Agy discovery remains unqualified.
pub(crate) fn discover_provider_program(driver_id: &str)
    -> Result<(ProgramObservation, PathBuf), ProviderCatalogError> {
    match driver_id {
        "claude" => npm_binary(&mut known_folder(&ROAMING)?,
            &["@anthropic-ai", "claude-code"], "@anthropic-ai/claude-code",
            "@anthropic-ai/claude-code-win32-x64", "2.1.196", "claude.exe"),
        "opencode" => npm_binary(&mut known_folder(&ROAMING)?,
            &["opencode-ai"], "opencode-ai", "opencode-windows-x64",
            "1.18.32", "opencode.exe"),
        "grok" => {
            let mut path = known_folder(&PROFILE)?;
            plain_dir(&path)?;
            enter(&mut path, &[".grok", "bin"])?;
            let application = path.join("grok.exe");
            plain_file(&application)?;
            let observation = ProgramObservation::observe(&application, "1.0.41")?;
            // Same-file hashes before/after the exact CLI's version command
            // matched; stdout was `grok 1.0.41 (4220f3b224a6) [stable]`.
            if !observation.matches_pin(
                "sha256:ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
                "1.0.41") {
                return Err(ProviderCatalogError::IdentityChanged);
            }
            Ok((observation, application))
        }
        "antigravity" => {
            let mut path = known_folder(&LOCAL)?;
            plain_dir(&path)?;
            enter(&mut path, &["agy", "bin"])?;
            plain_file(&path.join("agy.exe"))?;
            Err(ProviderCatalogError::UnsupportedObservation(
                "Antigravity 1.2.11 has no verified static version metadata for this binary"))
        }
        _ => Err(ProviderCatalogError::UnknownDriver),
    }
}
