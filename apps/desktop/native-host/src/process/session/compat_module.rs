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
const OBSERVATION_MODE: &str = "GOGOKE_LPAC_COMPAT_MODE";
const CLAUDE_PIPE_MODE: &str = "CLAUDE_PIPE_V1";
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
    mode: CompatMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompatMode { CodexPath, ClaudePipe }

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
        let DirectoryRoots { _directories: directories, homes } = DirectoryRoots::prepare(root, roots)?;
        Self::prepare_module(root, directories, homes, profile, profile_name, CompatMode::CodexPath)
    }

    /// Observation of the fixed Claude image only. The caller retains its
    /// ordinary directory custody separately; this mode never maps a path.
    pub(crate) fn prepare_claude_observation(
        root: &RootLock,
        profile: &AppContainerProfile,
        profile_name: &str,
    ) -> io::Result<Arc<Self>> {
        Self::prepare_module(root, Vec::new(), Vec::new(), profile, profile_name, CompatMode::ClaudePipe)
    }

    fn prepare_module(
        root: &RootLock,
        mut directories: Vec<Arc<File>>,
        homes: Vec<HeldRoot>,
        profile: &AppContainerProfile,
        profile_name: &str,
        mode: CompatMode,
    ) -> io::Result<Arc<Self>> {
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
            mode,
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
        if self.mode == CompatMode::ClaudePipe {
            environment.push((OBSERVATION_MODE.to_owned(), CLAUDE_PIPE_MODE.to_owned()));
            return;
        }
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
        if self.mode == CompatMode::ClaudePipe {
            if environment.iter().filter(|(key, _)| key.eq_ignore_ascii_case(OBSERVATION_MODE)).count() != 1
                || !environment.iter().any(|(key, value)| key == OBSERVATION_MODE && value == CLAUDE_PIPE_MODE)
                || environment.iter().any(|(key, _)| key.to_ascii_uppercase().starts_with("GOGOKE_LPAC_PATH_")) {
                return Err(invalid("Claude observation mode environment mismatch"));
            }
            return self.verify();
        }
        if environment.iter().any(|(key, _)| key.eq_ignore_ascii_case(OBSERVATION_MODE)) {
            return Err(invalid("Codex compatibility mode environment mismatch"));
        }
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

    pub(crate) fn expected_cli_sha256(&self) -> &'static str {
        match self.mode {
            CompatMode::CodexPath => gogoke_lpac_path_compat::OBSERVED_CLI_SHA256,
            CompatMode::ClaudePipe => gogoke_lpac_path_compat::OBSERVED_CLAUDE_SHA256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn real_claude_initialize_with_fixed_lpac_compat_persistent_stdio() {
        use crate::process::{NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch, StopBudgets};
        use crate::store::session_transport::provider_evidence::{commands, stream_json};
        use std::time::{Duration, Instant};

        const DIGEST: &str = "sha256:180d7b279455e8b89d4353a5146447be2f80b80fb0db14bdc6dd9cb98c0aef09";
        let program = crate::store::instance::locate_pinned_program("claude", DIGEST, "2.1.196")
            .expect("required real cloud catalog CLI and exact fixed pin");
        let original = program.parent().unwrap().parent().unwrap()
            .join("node_modules/@anthropic-ai/claude-code-win32-x64/claude.exe");
        let program_identity = AppContainerProfile::capture_catalog_program_identity(&program).unwrap();
        assert_eq!(AppContainerProfile::capture_catalog_program_identity(&original).unwrap(), program_identity);

        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-claude-direct-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let base = &root.canonical_root().canonical_path;
        let instance = base.join("instance-home");
        let session = base.join("session-home");
        let worktree = base.join("worktree");
        let roaming = instance.join("AppData/Roaming");
        let local = instance.join("AppData/Local");
        for directory in [&roaming, &local, &session, &worktree] {
            fs::create_dir_all(directory).unwrap();
        }
        let name = format!("Gogoke37.ClaudeDirect.{}.{stamp}", std::process::id());
        let profile = AppContainerProfile::ensure_for_cli(&name, true).unwrap();
        let instance_id = crate::root::inspect_root(&instance).unwrap().identity;
        let session_id = crate::root::inspect_root(&session).unwrap().identity;
        let worktree_id = crate::root::inspect_root(&worktree).unwrap().identity;
        profile.grant_bound_tree(&instance, &instance_id, true).unwrap();
        profile.grant_bound_tree(&session, &session_id, true).unwrap();
        profile.grant_bound_tree(&worktree, &worktree_id, true).unwrap();
        profile.grant_bound_catalog_program(&program, &program_identity).unwrap();
        profile.verify_bound_catalog_program_grant(&original, &program_identity).unwrap();
        let roots = Arc::new(DirectoryRoots::prepare(&root, &[
            (instance.clone(), instance_id), (session.clone(), session_id),
            (worktree.clone(), worktree_id),
        ]).unwrap());
        let module = CompatModule::prepare_claude_observation(&root, &profile, &name).unwrap();

        let system_root = std::env::var("SystemRoot").unwrap();
        let mut environment = vec![
            ("SystemRoot".into(), system_root.clone()), ("WINDIR".into(), system_root),
            ("HOME".into(), instance.to_string_lossy().into_owned()),
            ("USERPROFILE".into(), instance.to_string_lossy().into_owned()),
            ("APPDATA".into(), roaming.to_string_lossy().into_owned()),
            ("LOCALAPPDATA".into(), local.to_string_lossy().into_owned()),
            ("TEMP".into(), session.to_string_lossy().into_owned()),
            ("TMP".into(), session.to_string_lossy().into_owned()),
            ("CLAUDE_CONFIG_DIR".into(), instance.to_string_lossy().into_owned()),
            ("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into()),
        ];
        module.extend_environment(&mut environment);
        module.validate_launch(Some(&name), Some(&environment)).unwrap();
        let mut launch = ProcessLaunch::new(&program);
        launch.arguments = commands::claude_launch_args("claude-sonnet-4-6", "high", None).unwrap();
        launch.arguments.extend([
            "--permission-prompt-tool".into(), "stdio".into(),
            "--permission-mode".into(), "acceptEdits".into(),
            "--debug-file".into(), session.join("claude-startup-debug.log").to_string_lossy().into_owned(),
        ]);
        launch.current_directory = Some(worktree);
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.environment = Some(environment);
        launch.app_container_profile = Some(name.clone());
        launch.app_container_internet_client = true;
        launch.app_container_cli_identity_services = true;
        launch.path_compat = Some(module);
        launch.directory_roots = Some(roots);
        let request = PrepareRequest { launch, binding: NativeBinding {
            binary_digest_sha256: DIGEST.into(), profile_id: name,
            domain_id: "claude-direct-component".into(), generation: "1".into(),
        }};
        let mut custodian = ProcessCustodian::new().unwrap();
        let prepared = custodian.prepare(&request)
            .expect("fixed CLI suspended prepare, actual image SHA, LPAC token, Job and compat import");
        assert!(prepared.identity.pid > 0 && prepared.identity.creation_time_100ns > 0);
        assert!(custodian.active(&prepared.ticket).is_none());
        custodian.activate(&prepared).expect("activate exact prepared Claude child");
        let active = custodian.active(&prepared.ticket).unwrap();
        assert!(active.handles_are_non_inheritable().unwrap());
        let request_id = "claude-direct-initialize-original";
        let frame = commands::encode_claude(commands::ClaudeCommand::Initialize { request_id }).unwrap();
        let started = Instant::now();
        let mut raw_frames = Vec::new();
        let result = (|| -> Result<(), String> {
            active.write_persistent_frame(&frame)
                .map_err(|error| format!("original initialize write: {error}"))?;
            loop {
                let remaining = Duration::from_secs(30).saturating_sub(started.elapsed());
                if remaining.is_zero() { return Err("original initialize 30s total deadline".into()); }
                let raw = active.read_persistent_frame(remaining)
                    .map_err(|error| format!("original initialize read: {error}"))?;
                let decoded = stream_json::decode_claude_line(&raw);
                raw_frames.push(String::from_utf8_lossy(&raw).into_owned());
                match decoded {
                    Ok(stream_json::ClaudeData::ControlResponse { request_id: observed, success: true })
                        if observed == request_id => return Ok(()),
                    Ok(stream_json::ClaudeData::ControlResponse { request_id: observed, success: false })
                        if observed == request_id => return Err("original initialize was rejected".into()),
                    Err(error) => return Err(format!("original initialize stdout decode: {error:?}")),
                    _ => {},
                }
            }
        })();
        let fragment = active.persistent_stdout_fragment();
        let stderr = active.stderr_tail();
        let stderr_live = active.stderr_live_bytes();
        let exit_code = active.exit_code();
        let job_count = active.active_job_processes();
        let close = custodian.close_child_input(&prepared.ticket)
            .map_err(|error| format!("native Claude direct stdin close: {error}"));
        let close_evidence = close.clone();
        let proof = custodian.stop(&prepared.ticket, StopBudgets::production(), move || close);
        eprintln!("CLAUDE_DIRECT_INITIALIZE original_request_id={request_id} original_result={result:?} raw_stdout_frames={raw_frames:?} stdout_fragment={fragment:?} stderr_tail={stderr:?} stderr_live={stderr_live:?} exit_code={exit_code:?} active_job_processes={job_count:?} prepared_identity={:?} stdin_close={close_evidence:?} stop_proof={proof:?}", prepared.identity);
        let stop_ok = close_evidence.is_ok() && proof.as_ref().is_ok_and(|proof|
            proof.parent_exited && proof.writer_fence_verified && proof.active_job_processes == Some(0)
                && proof.errors.is_empty() && !proof.deadline_exceeded);
        drop(custodian);
        drop(request);
        drop(profile);
        drop(root);
        let cleanup = fs::remove_dir_all(path);
        assert!(result.is_ok(), "real fixed Claude initialize did not ACK its original request; see original component evidence above");
        assert!(stop_ok, "production stop after original initialize: {proof:?}");
        cleanup.expect("remove empty-auth Claude component fixture after stopped Job");
    }

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
        assert!(module.validate_launch(Some(&name), Some(&[
            (OBSERVATION_MODE.to_owned(), CLAUDE_PIPE_MODE.to_owned())
        ])).is_err());
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
    fn fixed_claude_observation_mode_rejects_mapping_and_mode_tamper() {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-compat-claude-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let name = format!("Gogoke37.CompatClaude.{stamp}");
        let profile = AppContainerProfile::ensure(&name, false).unwrap();
        let module = CompatModule::prepare_claude_observation(&root, &profile, &name).unwrap();
        let mut environment = Vec::new();
        module.extend_environment(&mut environment);
        assert_eq!(environment, vec![(OBSERVATION_MODE.to_owned(), CLAUDE_PIPE_MODE.to_owned())]);
        assert_eq!(module.expected_cli_sha256(), gogoke_lpac_path_compat::OBSERVED_CLAUDE_SHA256);
        module.validate_launch(Some(&name), Some(&environment)).unwrap();
        assert!(module.validate_launch(Some("Gogoke37.Other"), Some(&environment)).is_err());
        assert!(module.validate_launch(Some(&name), Some(&[])).is_err());
        let mut changed = environment.clone();
        changed[0].1 = "CODEX_PATH".into();
        assert!(module.validate_launch(Some(&name), Some(&changed)).is_err());
        changed = environment.clone();
        changed.push((OBSERVATION_MODE.to_ascii_lowercase(), CLAUDE_PIPE_MODE.into()));
        assert!(module.validate_launch(Some(&name), Some(&changed)).is_err());
        changed = environment.clone();
        changed.push((NT_ROOT.into(), "untrusted".into()));
        assert!(module.validate_launch(Some(&name), Some(&changed)).is_err());
        drop(module);
        drop(root);
        fs::remove_dir_all(path).unwrap();
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
