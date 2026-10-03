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
use std::sync::{Arc, Mutex, OnceLock, Weak};

const NT_ROOT: &str = "GOGOKE_LPAC_PATH_NT_ROOT";
const DOS_ROOT: &str = "GOGOKE_LPAC_PATH_DOS_ROOT";
const ROOT_COUNT: &str = "GOGOKE_LPAC_PATH_ROOT_COUNT";
const NT_ROOT_1: &str = "GOGOKE_LPAC_PATH_NT_ROOT_1";
const DOS_ROOT_1: &str = "GOGOKE_LPAC_PATH_DOS_ROOT_1";
const NT_ROOT_2: &str = "GOGOKE_LPAC_PATH_NT_ROOT_2";
const DOS_ROOT_2: &str = "GOGOKE_LPAC_PATH_DOS_ROOT_2";
const MAX_ROOTS: usize = 3;
const REPARSE: u32 = 0x400;
const DIRECTORY: u32 = 0x10;
const OPEN_PHYSICAL: u32 = 0x0220_0000;
const READ_ATTRIBUTES: u32 = 0x80;
const DELETE_ACCESS: u32 = 0x0001_0000;
const SHARE_READ_WRITE: u32 = 3;
const SHARE_READ_WRITE_DELETE: u32 = 7;

struct CachedDirectory {
    path: PathBuf,
    identity: RootIdentity,
    file: Weak<File>,
}

static DIRECTORY_CUSTODY: OnceLock<Mutex<Vec<CachedDirectory>>> = OnceLock::new();

fn directory_custody() -> &'static Mutex<Vec<CachedDirectory>> {
    DIRECTORY_CUSTODY.get_or_init(|| Mutex::new(Vec::new()))
}

#[repr(C)]
struct FileId {
    volume: u64,
    id: [u8; 16],
}
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
    fn GetFileInformationByHandleEx(
        handle: *mut c_void,
        class: i32,
        output: *mut c_void,
        length: u32,
    ) -> i32;
    fn GetFileInformationByHandle(handle: *mut c_void, output: *mut FileInfo) -> i32;
    fn GetFinalPathNameByHandleW(
        handle: *mut c_void,
        output: *mut u16,
        length: u32,
        flags: u32,
    ) -> u32;
    fn CompareStringOrdinal(
        first: *const u16,
        first_len: i32,
        second: *const u16,
        second_len: i32,
        ignore_case: i32,
    ) -> i32;
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn identity(file: &File) -> io::Result<RootIdentity> {
    let mut found = FileId {
        volume: 0,
        id: [0; 16],
    };
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            18,
            (&mut found as *mut FileId).cast(),
            std::mem::size_of::<FileId>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(RootIdentity {
        volume_serial: found.volume,
        file_id: found.id,
    })
}

fn physical_directory(path: &Path) -> io::Result<Arc<File>> {
    // Serialize check/open so two native H launches share one DELETE-owning
    // handle for the same exact path and file ID. A plain attributes handle
    // must share DELETE so it can inspect an already pinned directory.
    let mut cache = directory_custody()
        .lock()
        .map_err(|_| invalid("compatibility directory custody poisoned"))?;
    let observed = OpenOptions::new()
        .access_mode(READ_ATTRIBUTES)
        .share_mode(SHARE_READ_WRITE_DELETE)
        .custom_flags(OPEN_PHYSICAL)
        .open(path)?;
    if observed.metadata()?.file_attributes() & (REPARSE | DIRECTORY) != DIRECTORY {
        return Err(invalid("compatibility directory is not physical"));
    }
    let observed_id = identity(&observed)?;
    let observed_dos = final_path(&observed, 0)?;
    let observed_nt = final_path(&observed, 2)?;
    cache.retain(|entry| entry.file.strong_count() != 0);
    if cache
        .iter()
        .any(|entry| entry.path == path && entry.identity != observed_id)
    {
        return Err(invalid(
            "compatibility path resolves to conflicting physical identity",
        ));
    }
    if let Some(entry) = cache
        .iter()
        .find(|entry| entry.path == path && entry.identity == observed_id)
    {
        let held = entry
            .file
            .upgrade()
            .ok_or_else(|| invalid("compatibility shared custody expired"))?;
        if identity(&held)? != observed_id
            || final_path(&held, 0)? != observed_dos
            || final_path(&held, 2)? != observed_nt
        {
            return Err(invalid("compatibility shared directory identity changed"));
        }
        return Ok(held);
    }
    let file = OpenOptions::new()
        .access_mode(READ_ATTRIBUTES | DELETE_ACCESS)
        .share_mode(SHARE_READ_WRITE)
        .custom_flags(OPEN_PHYSICAL)
        .open(path)?;
    if file.metadata()?.file_attributes() & (REPARSE | DIRECTORY) != DIRECTORY
        || identity(&file)? != observed_id
        || final_path(&file, 0)? != observed_dos
        || final_path(&file, 2)? != observed_nt
    {
        return Err(invalid("compatibility directory changed during custody"));
    }
    let file = Arc::new(file);
    cache.push(CachedDirectory {
        path: path.to_path_buf(),
        identity: observed_id,
        file: Arc::downgrade(&file),
    });
    Ok(file)
}

fn final_path(file: &File, flags: u32) -> io::Result<String> {
    let mut value = vec![0u16; 32_768];
    let count = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle(),
            value.as_mut_ptr(),
            value.len() as u32,
            flags,
        )
    };
    if count == 0 {
        return Err(io::Error::last_os_error());
    }
    if count as usize >= value.len() {
        return Err(invalid("home final path exceeds Win32 limit"));
    }
    value.truncate(count as usize);
    let value = OsString::from_wide(&value)
        .into_string()
        .map_err(|_| invalid("home final path is not well-formed Unicode"))?;
    if value.ends_with('\\') || value.contains('\0') {
        return Err(invalid("unsupported home final path"));
    }
    Ok(value)
}

fn within_root(path: &str, root: &str) -> bool {
    let path: Vec<u16> = path.encode_utf16().collect();
    let root: Vec<u16> = root.encode_utf16().collect();
    if path.len() < root.len() {
        return false;
    }
    let comparison = unsafe {
        CompareStringOrdinal(
            path.as_ptr(),
            root.len() as i32,
            root.as_ptr(),
            root.len() as i32,
            1,
        )
    };
    (comparison == 2 || comparison == 0)
        && (path.len() == root.len() || path[root.len()] == b'\\' as u16)
}

#[derive(Debug)]
struct HeldRoot {
    _directory: Arc<File>,
    identity: RootIdentity,
    nt: String,
    dos: String,
}

/// Native directory custody independent of the Codex-only import shim.
/// Reuses the same shared physical handles for homes, worktree and ancestors.
#[derive(Debug)]
pub(crate) struct DirectoryRoots {
    _directories: Vec<Arc<File>>,
    homes: Vec<HeldRoot>,
}

impl DirectoryRoots {
    pub(crate) fn prepare(root: &RootLock, roots: &[(PathBuf, RootIdentity)]) -> io::Result<Self> {
        if roots.is_empty() || roots.len() > MAX_ROOTS {
            return Err(invalid("compatibility root count"));
        }
        let canonical = root.canonical_root();
        // RootLock already owns DELETE/no-share-delete on this exact root.
        // Reopening it with DELETE would self-conflict; its held identity is
        // the native root namespace pin for every descendant checked below.
        let base = canonical.canonical_path.join("v37-native-components");
        let version = base.join(gogoke_lpac_path_compat::MODULE_SHA256);
        let mut directories = Vec::new();
        let mut homes = Vec::with_capacity(roots.len());
        for (path, expected) in roots {
            if !path.is_absolute() || path.starts_with(&base) || version.starts_with(path) {
                return Err(invalid("compatibility mapping overlaps native module"));
            }
            let relative = path
                .strip_prefix(&canonical.canonical_path)
                .map_err(|_| invalid("compatibility mapping lies outside locked root"))?;
            if relative.as_os_str().is_empty() {
                return Err(invalid("compatibility mapping cannot be shared root"));
            }
            let components: Vec<_> = relative.components().collect();
            if matches!(components.first(),Some(std::path::Component::Normal(name))
                if name.to_string_lossy().eq_ignore_ascii_case("v37-native-components"))
            {
                return Err(invalid("compatibility mapping overlaps native module"));
            }
            let mut cursor = canonical.canonical_path.clone();
            for (index, component) in components.iter().enumerate() {
                let std::path::Component::Normal(name) = component else {
                    return Err(invalid("compatibility mapping path component"));
                };
                cursor.push(name);
                let handle = physical_directory(&cursor)?;
                if index + 1 == components.len() {
                    if &identity(&handle)? != expected {
                        return Err(invalid("compatibility mapping physical identity changed"));
                    }
                    let dos = final_path(&handle, 0)?;
                    let nt = final_path(&handle, 2)?;
                    let dos_bytes = dos.as_bytes();
                    if !nt.starts_with("\\Device\\")
                        || !dos.starts_with("\\\\?\\")
                        || dos_bytes.len() < 7
                        || !dos_bytes[4].is_ascii_alphabetic()
                        || dos_bytes[5] != b':'
                        || dos_bytes[6] != b'\\'
                    {
                        return Err(invalid("compatibility requires native local drive roots"));
                    }
                    homes.push(HeldRoot {
                        _directory: handle,
                        identity: expected.clone(),
                        nt,
                        dos,
                    });
                } else {
                    directories.push(handle);
                }
            }
        }
        for i in 0..homes.len() {
            for j in i + 1..homes.len() {
                let a = &homes[i];
                let b = &homes[j];
                if a.identity == b.identity
                    || within_root(&a.nt, &b.nt)
                    || within_root(&b.nt, &a.nt)
                    || within_root(&a.dos, &b.dos)
                    || within_root(&b.dos, &a.dos)
                {
                    return Err(invalid("compatibility roots duplicate or overlap"));
                }
            }
        }
        Ok(Self { _directories: directories, homes })
    }

    pub(crate) fn verify(&self) -> io::Result<()> {
        verify_held_roots(&self.homes)
    }
}

fn verify_held_roots(homes: &[HeldRoot]) -> io::Result<()> {
    for home in homes {
        if identity(&home._directory)? != home.identity
            || final_path(&home._directory, 0)? != home.dos
            || final_path(&home._directory, 2)? != home.nt
        {
            return Err(invalid("compatibility held mapping identity changed"));
        }
    }
    Ok(())
}

/// Sealed native value. Held file and ancestor handles block module replacement
/// and path renaming. Kept by prepared/active/unknown process custody, not IPC.
#[derive(Debug)]
pub(crate) struct CompatModule {
    path: PathBuf,
    ansi: CString,
    file: File,
    file_identity: RootIdentity,
    _directories: Vec<Arc<File>>,
    homes: Vec<HeldRoot>,
    profile_name: String,
}

impl CompatModule {
    pub(crate) fn prepare(
        root: &RootLock,
        home: &Path,
        expected: &RootIdentity,
        profile: &AppContainerProfile,
        profile_name: &str,
    ) -> io::Result<Arc<Self>> {
        Self::prepare_with_roots(
            root,
            &[(home.to_path_buf(), expected.clone())],
            profile,
            profile_name,
        )
    }

    /// Native-only finite map. Every directory and ancestor stays held without
    /// FILE_SHARE_DELETE through the child lifetime. It changes path spelling,
    /// never ACL rights or H's actual worktree admission decision.
    pub(crate) fn prepare_with_roots(
        root: &RootLock,
        roots: &[(PathBuf, RootIdentity)],
        profile: &AppContainerProfile,
        profile_name: &str,
    ) -> io::Result<Arc<Self>> {
        let DirectoryRoots { _directories: mut directories, homes } = DirectoryRoots::prepare(root, roots)?;
        let base = root.canonical_root().canonical_path.join("v37-native-components");
        let version = base.join(gogoke_lpac_path_compat::MODULE_SHA256);
        for directory in [&base, &version] {
            match fs::create_dir(directory) {
                Ok(()) => (),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
                Err(error) => return Err(error),
            }
            directories.push(physical_directory(directory)?);
        }
        let path = version.join("gogoke_lpac_path_compat.dll");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(&path)
        {
            Ok(mut output) => {
                output.write_all(gogoke_lpac_path_compat::MODULE_BYTES)?;
                output.sync_all()?;
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error),
        }
        let file = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .custom_flags(0x0020_0000)
            .open(&path)?;
        let metadata = file.metadata()?;
        let mut info: FileInfo = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if metadata.file_attributes() & (REPARSE | DIRECTORY) != 0 || info.links != 1 {
            return Err(invalid("compatibility DLL is not a single physical file"));
        }
        let file_identity = identity(&file)?;
        let ansi = gogoke_lpac_path_compat::module_path_ansi(&path)?;
        let module = Self {
            path,
            ansi,
            file,
            file_identity,
            _directories: directories,
            homes,
            profile_name: profile_name.to_owned(),
        };
        module.verify()?;
        profile
            .grant_bound_program(&module.path, &module.file_identity)
            .map_err(|error| invalid(&format!("compatibility DLL RX grant: {error}")))?;
        profile
            .verify_bound_program_grant(&module.path, &module.file_identity)
            .map_err(|error| invalid(&format!("compatibility DLL RX witness: {error}")))?;
        module.verify()?;
        Ok(Arc::new(module))
    }

    fn verify(&self) -> io::Result<()> {
        verify_held_roots(&self.homes)?;
        if identity(&self.file)? != self.file_identity
            || AppContainerProfile::capture_program_identity(&self.path)
                .map_err(|error| invalid(&format!("compatibility DLL identity: {error}")))?
                != self.file_identity
        {
            return Err(invalid("compatibility DLL identity changed"));
        }
        let mut file = self.file.try_clone()?;
        file.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.take(gogoke_lpac_path_compat::MODULE_BYTES.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes != gogoke_lpac_path_compat::MODULE_BYTES
            || sha256_hex(&bytes) != gogoke_lpac_path_compat::MODULE_SHA256
        {
            return Err(invalid("compatibility DLL embedded-byte hash mismatch"));
        }
        Ok(())
    }

    pub(crate) fn extend_environment(&self, environment: &mut Vec<(String, String)>) {
        environment.push((NT_ROOT.to_owned(), self.homes[0].nt.clone()));
        environment.push((DOS_ROOT.to_owned(), self.homes[0].dos.clone()));
        if self.homes.len() > 1 {
            environment.push((ROOT_COUNT.to_owned(), self.homes.len().to_string()));
            environment.push((NT_ROOT_1.to_owned(), self.homes[1].nt.clone()));
            environment.push((DOS_ROOT_1.to_owned(), self.homes[1].dos.clone()));
        }
        if self.homes.len() == 3 {
            environment.push((NT_ROOT_2.to_owned(), self.homes[2].nt.clone()));
            environment.push((DOS_ROOT_2.to_owned(), self.homes[2].dos.clone()));
        }
    }

    pub(crate) fn validate_launch(
        &self,
        profile: Option<&str>,
        environment: Option<&[(String, String)]>,
    ) -> io::Result<()> {
        if profile != Some(self.profile_name.as_str()) {
            return Err(invalid("compatibility LPAC profile mismatch"));
        }
        let environment = environment.ok_or_else(|| invalid("compatibility environment absent"))?;
        let mut expected = vec![
            (NT_ROOT, self.homes[0].nt.as_str()),
            (DOS_ROOT, self.homes[0].dos.as_str()),
        ];
        if self.homes.len() > 1 {
            expected.extend([
                (ROOT_COUNT, if self.homes.len() == 2 { "2" } else { "3" }),
                (NT_ROOT_1, self.homes[1].nt.as_str()),
                (DOS_ROOT_1, self.homes[1].dos.as_str()),
            ]);
        }
        if self.homes.len() == 3 {
            expected.extend([
                (NT_ROOT_2, self.homes[2].nt.as_str()),
                (DOS_ROOT_2, self.homes[2].dos.as_str()),
            ]);
        }
        for &(name, value) in &expected {
            if environment
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                .count()
                != 1
                || !environment
                    .iter()
                    .any(|(key, observed)| key == name && observed == value)
            {
                return Err(invalid("compatibility native mapping mismatch"));
            }
        }
        if environment.iter().any(|(key, _)| {
            key.to_ascii_uppercase().starts_with("GOGOKE_LPAC_PATH_")
                && !expected.iter().any(|(name, _)| key == name)
        }) {
            return Err(invalid("compatibility unexpected mapping variable"));
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
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("gogoke-compat-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let home = path.join("home");
        fs::create_dir(&home).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        // Native callers derive admitted paths from this exact RootLock.
        // Keep the fixture on that canonical spelling as well.
        let home = root.canonical_root().canonical_path.join("home");
        let home_identity = crate::root::inspect_root(&home).unwrap().identity;
        let name = format!("Gogoke37.CompatTest.{stamp}");
        let profile = AppContainerProfile::ensure(&name, false).unwrap();
        let module = CompatModule::prepare(&root, &home, &home_identity, &profile, &name).unwrap();
        let mut environment = Vec::new();
        module.extend_environment(&mut environment);
        module
            .validate_launch(Some(&name), Some(&environment))
            .unwrap();
        assert!(module
            .validate_launch(Some("Gogoke37.Other"), Some(&environment))
            .is_err());
        environment[0].1.push_str("-other");
        assert!(module
            .validate_launch(Some(&name), Some(&environment))
            .is_err());
        assert!(OpenOptions::new().write(true).open(&module.path).is_err());
        assert!(fs::remove_file(&module.path).is_err());
        let module_path = module.path.clone();
        drop(module);
        // A pre-existing unexpected byte sequence is refused, never repaired
        // in place or loaded under the fixed module's expected digest.
        fs::write(&module_path, b"not the embedded module").unwrap();
        assert!(CompatModule::prepare(&root, &home, &home_identity, &profile, &name).is_err());
        let wrong_identity = RootIdentity {
            volume_serial: home_identity.volume_serial,
            file_id: [0; 16],
        };
        assert!(CompatModule::prepare(&root, &home, &wrong_identity, &profile, &name).is_err());
        drop(root);
        fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn sealed_three_roots_are_distinct_and_reject_overlap_or_mapping_tamper() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-compat-three-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let home = path.join("v37-instances").join("instanceA");
        let session = path
            .join("v37-temporary-homes")
            .join("instanceA")
            .join("sessionA");
        let worktree = path.join("v37-worktrees").join("single").join("opaqueA");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&session).unwrap();
        fs::create_dir_all(&worktree).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let path = root.canonical_root().canonical_path.clone();
        let home = path.join("v37-instances").join("instanceA");
        let session = path
            .join("v37-temporary-homes")
            .join("instanceA")
            .join("sessionA");
        let worktree = path.join("v37-worktrees").join("single").join("opaqueA");
        let home_id = crate::root::inspect_root(&home).unwrap().identity;
        let session_id = crate::root::inspect_root(&session).unwrap().identity;
        let worktree_id = crate::root::inspect_root(&worktree).unwrap().identity;
        let name = format!("Gogoke37.CompatDual.{stamp}");
        let profile = AppContainerProfile::ensure(&name, false).unwrap();
        let roots = [
            (home.clone(), home_id.clone()),
            (session.clone(), session_id.clone()),
            (worktree.clone(), worktree_id.clone()),
        ];
        assert!(CompatModule::prepare_with_roots(&root, &[], &profile, &name).is_err());
        assert!(CompatModule::prepare_with_roots(
            &root,
            &[
                roots[0].clone(),
                roots[1].clone(),
                roots[2].clone(),
                roots[0].clone()
            ],
            &profile,
            &name
        )
        .is_err());
        let module = CompatModule::prepare_with_roots(&root, &roots, &profile, &name).unwrap();
        let mut environment = Vec::new();
        module.extend_environment(&mut environment);
        assert_eq!(environment.len(), 7);
        module
            .validate_launch(Some(&name), Some(&environment))
            .unwrap();
        assert_eq!(module.homes.len(), 3);
        assert_ne!(module.homes[0].identity, module.homes[1].identity);
        assert_ne!(module.homes[1].identity, module.homes[2].identity);
        assert!(fs::rename(&worktree, worktree.with_extension("moved")).is_err());
        let mut changed = environment.clone();
        changed
            .iter_mut()
            .find(|(key, _)| key == NT_ROOT_2)
            .unwrap()
            .1
            .push_str("-other");
        assert!(module.validate_launch(Some(&name), Some(&changed)).is_err());
        changed = environment.clone();
        changed.push((DOS_ROOT_2.to_owned(), "unexpected".to_owned()));
        assert!(module.validate_launch(Some(&name), Some(&changed)).is_err());
        changed = environment.clone();
        changed
            .iter_mut()
            .find(|(key, _)| key == ROOT_COUNT)
            .unwrap()
            .1 = "2".to_owned();
        assert!(module.validate_launch(Some(&name), Some(&changed)).is_err());
        assert!(CompatModule::prepare_with_roots(
            &root,
            &[
                (home.clone(), home_id.clone()),
                (home.clone(), home_id.clone())
            ],
            &profile,
            &name
        )
        .is_err());
        let nested = home.join("nested");
        fs::create_dir(&nested).unwrap();
        let nested_id = crate::root::inspect_root(&nested).unwrap().identity;
        assert!(CompatModule::prepare_with_roots(
            &root,
            &[(home.clone(), home_id.clone()), (nested, nested_id)],
            &profile,
            &name
        )
        .is_err());
        let wrong = RootIdentity {
            volume_serial: worktree_id.volume_serial,
            file_id: [0; 16],
        };
        assert!(CompatModule::prepare_with_roots(
            &root,
            &[(home, home_id), (worktree, wrong)],
            &profile,
            &name
        )
        .is_err());
        let module_base = path.join("v37-native-components");
        let module_base_id = crate::root::inspect_root(&module_base).unwrap().identity;
        assert!(CompatModule::prepare_with_roots(
            &root,
            &[(path.join("V37-NATIVE-COMPONENTS"), module_base_id)],
            &profile,
            &name
        )
        .is_err());
        let redirected = path.join("redirected");
        std::os::windows::fs::symlink_dir(&roots[2].0, &redirected)
            .expect("cloud reparse-control fixture must execute, never silently skip");
        {
            assert!(CompatModule::prepare_with_roots(
                &root,
                &[
                    (roots[0].0.clone(), roots[0].1.clone()),
                    (roots[1].0.clone(), roots[1].1.clone()),
                    (redirected.clone(), roots[2].1.clone())
                ],
                &profile,
                &name
            )
            .is_err());
            fs::remove_dir(&redirected).unwrap();
        }
        drop(module);
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn shared_directory_pin_lasts_until_final_module_and_keeps_child_operations() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-compat-shared-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let home = root.canonical_root().canonical_path.join("v37-instances").join("instanceA");
        fs::create_dir_all(&home).unwrap();
        let expected = crate::root::inspect_root(&home).unwrap().identity;
        let name = format!("Gogoke37.CompatShared.{stamp}");
        let profile = AppContainerProfile::ensure(&name, false).unwrap();
        let first = CompatModule::prepare(&root, &home, &expected, &profile, &name).unwrap();
        let second = CompatModule::prepare(&root, &home, &expected, &profile, &name).unwrap();
        assert!(Arc::ptr_eq(
            &first.homes[0]._directory,
            &second.homes[0]._directory
        ));
        let moved = home.with_extension("moved");
        let ancestor = home.parent().unwrap().to_path_buf();
        let ancestor_moved = ancestor.with_extension("moved");
        assert!(fs::rename(&home, &moved).is_err());
        assert!(fs::rename(&ancestor, &ancestor_moved).is_err());
        let child = home.join("child-a.txt");
        let child_moved = home.join("child-b.txt");
        fs::write(&child, b"owned synthetic child").unwrap();
        fs::rename(&child, &child_moved).unwrap();
        fs::remove_file(&child_moved).unwrap();
        drop(first);
        assert!(fs::rename(&home, &moved).is_err());
        assert!(fs::rename(&ancestor, &ancestor_moved).is_err());
        drop(second);
        fs::rename(&home, &moved).unwrap();
        fs::rename(&moved, &home).unwrap();
        fs::rename(&ancestor, &ancestor_moved).unwrap();
        fs::rename(&ancestor_moved, &ancestor).unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}
