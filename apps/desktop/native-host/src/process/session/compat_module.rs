//! Host-only custody of the fixed path compatibility DLL and F-home mapping.
//! No protocol or configuration can select a module, path, bytes or mapping.

use super::AppContainerProfile;
use crate::root::{RootIdentity, RootLock};
use crate::store::digest::sha256_hex;
use std::ffi::{c_void, CString, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::ffi::OsStringExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const NT_ROOT: &str = "GOGOKE_LPAC_PATH_NT_ROOT";
const DOS_ROOT: &str = "GOGOKE_LPAC_PATH_DOS_ROOT";
const REPARSE: u32 = 0x400;
const DIRECTORY: u32 = 0x10;
const OPEN_PHYSICAL: u32 = 0x0220_0000;

#[repr(C)]
struct FileId { volume: u64, id: [u8; 16] }
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
    fn GetFinalPathNameByHandleW(handle: *mut c_void, output: *mut u16,
        length: u32, flags: u32) -> u32;
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn identity(file: &File) -> io::Result<RootIdentity> {
    let mut found = FileId { volume: 0, id: [0; 16] };
    if unsafe { GetFileInformationByHandleEx(file.as_raw_handle(), 18,
        (&mut found as *mut FileId).cast(), std::mem::size_of::<FileId>() as u32) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(RootIdentity { volume_serial: found.volume, file_id: found.id })
}

fn physical_directory(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().access_mode(0x80).share_mode(3)
        .custom_flags(OPEN_PHYSICAL).open(path)?;
    let attributes = file.metadata()?.file_attributes();
    if attributes & (REPARSE | DIRECTORY) != DIRECTORY {
        return Err(invalid("compatibility directory is not physical"));
    }
    Ok(file)
}

fn final_path(file: &File, flags: u32) -> io::Result<String> {
    let mut value = vec![0u16; 32_768];
    let count = unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(),
        value.as_mut_ptr(), value.len() as u32, flags) };
    if count == 0 { return Err(io::Error::last_os_error()); }
    if count as usize >= value.len() { return Err(invalid("home final path exceeds Win32 limit")); }
    value.truncate(count as usize);
    let value = OsString::from_wide(&value).into_string()
        .map_err(|_| invalid("home final path is not well-formed Unicode"))?;
    if value.ends_with('\\') || value.contains('\0') {
        return Err(invalid("unsupported home final path"));
    }
    Ok(value)
}

/// Sealed native value. Held file and ancestor handles block module replacement
/// and path renaming. Kept by prepared/active/unknown process custody, not IPC.
#[derive(Debug)]
pub(crate) struct CompatModule {
    path: PathBuf,
    ansi: CString,
    file: File,
    file_identity: RootIdentity,
    _directories: Vec<File>,
    _home: File,
    profile_name: String,
    nt_root: String,
    dos_root: String,
}

impl CompatModule {
    pub(crate) fn prepare(root: &RootLock, home: &Path, expected: &RootIdentity,
        profile: &AppContainerProfile, profile_name: &str) -> io::Result<Arc<Self>> {
        let canonical = root.canonical_root();
        let root_handle = physical_directory(&canonical.canonical_path)?;
        if identity(&root_handle)? != canonical.identity {
            return Err(invalid("compatibility root identity changed"));
        }
        let home_handle = physical_directory(home)?;
        if &identity(&home_handle)? != expected {
            return Err(invalid("compatibility home identity changed"));
        }
        let dos_root = final_path(&home_handle, 0)?;
        let nt_root = final_path(&home_handle, 2)?;
        if !nt_root.starts_with("\\Device\\") || !dos_root.starts_with("\\\\?\\")
            || dos_root.as_bytes().get(5) != Some(&b':') {
            return Err(invalid("compatibility requires a native local drive home"));
        }
        let base = canonical.canonical_path.join("v37-native-components");
        let version = base.join(gogoke_lpac_path_compat::MODULE_SHA256);
        if version.starts_with(home) || home.starts_with(&base) {
            return Err(invalid("compatibility module overlaps writable instance home"));
        }
        let mut directories = vec![root_handle];
        for directory in [&base, &version] {
            match fs::create_dir(directory) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error),
            }
            directories.push(physical_directory(directory)?);
        }
        let path = version.join("gogoke_lpac_path_compat.dll");
        match OpenOptions::new().write(true).create_new(true).share_mode(0).open(&path) {
            Ok(mut output) => {
                output.write_all(gogoke_lpac_path_compat::MODULE_BYTES)?;
                output.sync_all()?;
            },
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        let file = OpenOptions::new().read(true).share_mode(1)
            .custom_flags(0x0020_0000).open(&path)?;
        let metadata = file.metadata()?;
        let mut info: FileInfo = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if metadata.file_attributes() & (REPARSE | DIRECTORY) != 0
            || info.links != 1 {
            return Err(invalid("compatibility DLL is not a single physical file"));
        }
        let file_identity = identity(&file)?;
        let ansi = gogoke_lpac_path_compat::module_path_ansi(&path)?;
        let module = Self { path, ansi, file, file_identity,
            _directories: directories, _home: home_handle,
            profile_name: profile_name.to_owned(), nt_root, dos_root };
        module.verify()?;
        profile.grant_bound_program(&module.path, &module.file_identity)
            .map_err(|error| invalid(&format!("compatibility DLL RX grant: {error}")))?;
        profile.verify_bound_program_grant(&module.path, &module.file_identity)
            .map_err(|error| invalid(&format!("compatibility DLL RX witness: {error}")))?;
        module.verify()?;
        Ok(Arc::new(module))
    }

    fn verify(&self) -> io::Result<()> {
        if identity(&self.file)? != self.file_identity
            || AppContainerProfile::capture_program_identity(&self.path)
                .map_err(|error| invalid(&format!("compatibility DLL identity: {error}")))?
                != self.file_identity {
            return Err(invalid("compatibility DLL identity changed"));
        }
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(gogoke_lpac_path_compat::MODULE_BYTES.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes != gogoke_lpac_path_compat::MODULE_BYTES
            || sha256_hex(&bytes) != gogoke_lpac_path_compat::MODULE_SHA256 {
            return Err(invalid("compatibility DLL embedded-byte hash mismatch"));
        }
        Ok(())
    }

    pub(crate) fn extend_environment(&self, environment: &mut Vec<(String, String)>) {
        environment.push((NT_ROOT.to_owned(), self.nt_root.clone()));
        environment.push((DOS_ROOT.to_owned(), self.dos_root.clone()));
    }

    pub(crate) fn validate_launch(&self, profile: Option<&str>,
        environment: Option<&[(String, String)]>) -> io::Result<()> {
        if profile != Some(self.profile_name.as_str()) { return Err(invalid("compatibility LPAC profile mismatch")); }
        let environment = environment.ok_or_else(|| invalid("compatibility environment absent"))?;
        for (name, expected) in [(NT_ROOT, &self.nt_root), (DOS_ROOT, &self.dos_root)] {
            if environment.iter().filter(|(key, _)| key.eq_ignore_ascii_case(name)).count() != 1
                || !environment.iter().any(|(key, value)| key == name && value == expected) {
                return Err(invalid("compatibility native mapping mismatch"));
            }
        }
        self.verify()
    }

    pub(crate) unsafe fn update_suspended(&self, process: *mut c_void) -> io::Result<()> {
        self.verify()?;
        gogoke_lpac_path_compat::update_suspended(process, &self.ansi)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fixed_module_locks_bytes_identity_and_native_mapping() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-compat-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let home = path.join("home");
        fs::create_dir(&home).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let home_identity = crate::root::inspect_root(&home).unwrap().identity;
        let name = format!("Gogoke37.CompatTest.{stamp}");
        let profile = AppContainerProfile::ensure(&name, false).unwrap();
        let module = CompatModule::prepare(&root, &home, &home_identity, &profile, &name).unwrap();
        let mut environment = Vec::new();
        module.extend_environment(&mut environment);
        module.validate_launch(Some(&name), Some(&environment)).unwrap();
        assert!(module.validate_launch(Some("Gogoke37.Other"), Some(&environment)).is_err());
        environment[0].1.push_str("-other");
        assert!(module.validate_launch(Some(&name), Some(&environment)).is_err());
        assert!(OpenOptions::new().write(true).open(&module.path).is_err());
        assert!(fs::remove_file(&module.path).is_err());
        let module_path = module.path.clone();
        drop(module);
        // A pre-existing unexpected byte sequence is refused, never repaired
        // in place or loaded under the fixed module's expected digest.
        fs::write(&module_path, b"not the embedded module").unwrap();
        assert!(CompatModule::prepare(&root, &home, &home_identity, &profile, &name).is_err());
        let wrong_identity = RootIdentity { volume_serial: home_identity.volume_serial,
            file_id: [0; 16] };
        assert!(CompatModule::prepare(&root, &home, &wrong_identity, &profile, &name).is_err());
        drop(root);
        fs::remove_dir_all(&path).unwrap();
    }
}
