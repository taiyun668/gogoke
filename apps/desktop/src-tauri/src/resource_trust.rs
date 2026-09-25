//! The installed shell's resource boundary. The signature authenticates an
//! index; the index authenticates the exact bytes handed to the WebView and
//! to the managed service. A remembered "verified" flag is never authority.

use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use winreg::{enums::HKEY_CURRENT_USER, RegKey};
use zip::{CompressionMethod, ZipArchive};

const CANDIDATE_KEY: &str = include_str!("../gogoke-candidate-public-key.txt");
const OWNER_KEY: &str = include_str!("../gogoke-release-public-key.txt");
const CANDIDATE_PREFIX: &[u8] = b"GOGOKE-CI-CANDIDATE-RESOURCE-V1\0";
const CANDIDATE_MANIFEST: &str = "CANDIDATE-RESOURCES.windows";
const OWNER_MANIFEST: &str = "SHA256SUMS.windows";
const RESOURCE_PACK: &str = "gogoke-resources.windows.zip";
const RESOURCE_INDEX: &str = "resource-index.json";
const INSTALL_RECEIPT: &str = "gogoke-install-receipt.ini";
const ACTIVE_SET_POINTER: &str = "gogoke-current-resource-set";
const RESOURCE_SETS: &str = "gogoke-resource-sets";
const LIFECYCLE_LOCK: &str = "gogoke-install-lifecycle.lock";
const REPARSE_POINT: u32 = 0x400;
const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
const MAX_INDEX_BYTES: u64 = 4 << 20;
const MAX_MANIFEST_BYTES: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Domain {
    Candidate,
    Formal,
}

#[derive(Clone, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ByteRecord {
    length: u64,
    sha256: String,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PackRecord {
    file_name: String,
    length: u64,
    sha256: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexedFile {
    path: String,
    length: u64,
    sha256: String,
}

#[derive(Clone, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Executables {
    portable_shell: ByteRecord,
    installed_shell: ByteRecord,
    native_host: ByteRecord,
    node: ByteRecord,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResourceIndex {
    schema: String,
    source_commit: String,
    version: String,
    generation_id: String,
    pack: PackRecord,
    files: Vec<IndexedFile>,
    installed_files: Vec<IndexedFile>,
    executables: Executables,
}

struct ManifestBinding {
    version: String,
    source_commit: Option<String>,
    release_type: Option<String>,
    hashes: HashMap<String, String>,
}

struct SignedIndex {
    domain: Domain,
    index: ResourceIndex,
    binding: ManifestBinding,
    set_id: String,
}

#[derive(Clone)]
pub(crate) struct VerifiedResources {
    pub(crate) domain: Domain,
    pub(crate) version: String,
    pub(crate) source_commit: String,
    pub(crate) generation_id: String,
    pub(crate) set_id: String,
    pub(crate) generation_root: PathBuf,
    pub(crate) service_root: PathBuf,
    pub(crate) install_root: PathBuf,
    pub(crate) native_host_path: PathBuf,
    pub(crate) node_runtime_path: PathBuf,
    native_host: ByteRecord,
    node: ByteRecord,
    executables: Executables,
    files: Arc<HashMap<String, ByteRecord>>,
    installed_files: Arc<HashMap<String, ByteRecord>>,
}

#[derive(Clone)]
pub(crate) struct ResourceState(Arc<RwLock<VerifiedResources>>);

pub(crate) struct LifecycleLock {
    _file: fs::File,
}

pub(crate) fn acquire_lifecycle_lock(root: &Path) -> Result<LifecycleLock, String> {
    let parent = root.parent().ok_or("GOGOKE_INSTALL_ROOT_UNAVAILABLE")?;
    let metadata = fs::symlink_metadata(parent)
        .map_err(|_| "GOGOKE_INSTALL_PARENT_UNAVAILABLE".to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .share_mode(0)
        .open(parent.join(LIFECYCLE_LOCK))
        .map_err(|_| "GOGOKE_INSTALL_LIFECYCLE_BUSY".to_string())?;
    Ok(LifecycleLock { _file: file })
}

impl ResourceState {
    pub(crate) fn new(resources: VerifiedResources) -> Self {
        Self(Arc::new(RwLock::new(resources)))
    }

    pub(crate) fn current(&self) -> Result<VerifiedResources, String> {
        self.0
            .read()
            .map(|value| value.clone())
            .map_err(|_| "GOGOKE_RESOURCE_STATE_UNAVAILABLE".to_string())
    }

    pub(crate) fn replace(&self, resources: VerifiedResources) -> Result<(), String> {
        *self
            .0
            .write()
            .map_err(|_| "GOGOKE_RESOURCE_STATE_UNAVAILABLE".to_string())? = resources;
        Ok(())
    }
}

fn lowercase_sha(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "GOGOKE_RESOURCE_FILE_MISSING".to_string())?;
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || metadata.len() > limit
    {
        return Err("GOGOKE_RESOURCE_FILE_UNSAFE".to_string());
    }
    fs::read(path).map_err(|_| "GOGOKE_RESOURCE_FILE_UNREADABLE".to_string())
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_sha256(path: &Path, expected: &ByteRecord) -> Result<(), String> {
    let _ = read_verified_file(path, expected, false)?;
    Ok(())
}

fn read_verified_file(
    path: &Path,
    expected: &ByteRecord,
    retain_bytes: bool,
) -> Result<Vec<u8>, String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| "GOGOKE_RESOURCE_FILE_MISSING".to_string())?;
    if !metadata.is_file()
        || metadata.file_attributes() & REPARSE_POINT != 0
        || metadata.len() != expected.length
    {
        return Err("GOGOKE_RESOURCE_FILE_IDENTITY_MISMATCH".to_string());
    }
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| "GOGOKE_RESOURCE_FILE_UNREADABLE".to_string())?;
    if file
        .metadata()
        .map_err(|_| "GOGOKE_RESOURCE_FILE_UNREADABLE".to_string())?
        .file_attributes()
        & REPARSE_POINT
        != 0
    {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    let mut digest = Sha256::new();
    let mut count = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = if retain_bytes {
        Vec::with_capacity(expected.length.min(16 << 20) as usize)
    } else {
        Vec::new()
    };
    loop {
        let size = file
            .read(&mut buffer)
            .map_err(|_| "GOGOKE_RESOURCE_FILE_UNREADABLE".to_string())?;
        if size == 0 {
            break;
        }
        count = count
            .checked_add(size as u64)
            .ok_or("GOGOKE_RESOURCE_FILE_TOO_LARGE")?;
        if count > expected.length {
            return Err("GOGOKE_RESOURCE_FILE_IDENTITY_MISMATCH".to_string());
        }
        digest.update(&buffer[..size]);
        if retain_bytes {
            bytes.extend_from_slice(&buffer[..size]);
        }
    }
    if count != expected.length || format!("{:x}", digest.finalize()) != expected.sha256 {
        return Err("GOGOKE_RESOURCE_FILE_IDENTITY_MISMATCH".to_string());
    }
    Ok(bytes)
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if value.len() % 2 != 0 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("GOGOKE_RESOURCE_SIGNATURE_SHAPE".to_string());
    }
    (0..value.len())
        .step_by(2)
        .map(|position| {
            u8::from_str_radix(&value[position..position + 2], 16)
                .map_err(|_| "GOGOKE_RESOURCE_SIGNATURE_SHAPE".to_string())
        })
        .collect()
}

fn verify_signature(
    public_hex: &str,
    payload: &[u8],
    signature_hex: &[u8],
    prefix: &[u8],
) -> Result<(), String> {
    let public = decode_hex(public_hex.trim())?;
    if public.len() != 64 {
        return Err("GOGOKE_RESOURCE_PUBLIC_KEY_SHAPE".to_string());
    }
    let mut point = Vec::with_capacity(65);
    point.push(4);
    point.extend(public);
    let key = VerifyingKey::from_sec1_bytes(&point)
        .map_err(|_| "GOGOKE_RESOURCE_PUBLIC_KEY_SHAPE".to_string())?;
    let signature_text = std::str::from_utf8(signature_hex)
        .map_err(|_| "GOGOKE_RESOURCE_SIGNATURE_SHAPE".to_string())?;
    let signature = Signature::from_slice(&decode_hex(signature_text.trim())?)
        .map_err(|_| "GOGOKE_RESOURCE_SIGNATURE_SHAPE".to_string())?;
    let mut signed = Vec::with_capacity(prefix.len() + payload.len());
    signed.extend_from_slice(prefix);
    signed.extend_from_slice(payload);
    key.verify(&signed, &signature)
        .map_err(|_| "GOGOKE_RESOURCE_SIGNATURE_INVALID".to_string())
}

fn manifest_binding(manifest: &[u8], domain: Domain) -> Result<ManifestBinding, String> {
    if manifest.contains(&b'\r') || !manifest.ends_with(b"\n") {
        return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
    }
    let text =
        std::str::from_utf8(manifest).map_err(|_| "GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string())?;
    let mut headers = HashMap::new();
    let mut hashes = HashMap::new();
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("# gogoke-") {
            let (key, value) = header
                .split_once(": ")
                .ok_or("GOGOKE_RESOURCE_MANIFEST_FORMAT")?;
            if headers.insert(key, value).is_some() || value.is_empty() {
                return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
            }
        } else {
            let (hash, name) = line
                .split_once("  ")
                .ok_or("GOGOKE_RESOURCE_MANIFEST_FORMAT")?;
            if !lowercase_sha(hash, 64) || name.is_empty() || hashes.insert(name, hash).is_some() {
                return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
            }
        }
    }
    let expected_headers: &[&str] = match domain {
        Domain::Candidate => &[
            "Candidate-Purpose",
            "Test-Only",
            "Repository",
            "Source-Commit",
            "Run-Id",
            "Run-Attempt",
            "Artifact-Id",
            "Version",
        ],
        Domain::Formal => &["Version", "Release-Type"],
    };
    if headers.len() != expected_headers.len()
        || expected_headers
            .iter()
            .any(|name| !headers.contains_key(name))
    {
        return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
    }
    let version = headers["Version"].to_owned();
    semver::Version::parse(&version).map_err(|_| "GOGOKE_RESOURCE_VERSION_INVALID".to_string())?;
    let source = if domain == Domain::Candidate {
        if headers["Candidate-Purpose"] != "CI_CANDIDATE_RESOURCE"
            || headers["Test-Only"] != "true"
            || headers["Repository"] != "taiyun668/gogoke"
            || ["Run-Id", "Run-Attempt", "Artifact-Id"]
                .iter()
                .any(|name| !headers[name].bytes().all(|byte| byte.is_ascii_digit()))
            || !lowercase_sha(headers["Source-Commit"], 40)
        {
            return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
        }
        Some(headers["Source-Commit"].to_owned())
    } else {
        if headers["Release-Type"] != "full" && headers["Release-Type"] != "resources" {
            return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
        }
        None
    };
    let required_names: Vec<String> =
        if domain == Domain::Candidate || headers.get("Release-Type") == Some(&"resources") {
            vec![RESOURCE_INDEX.to_string(), RESOURCE_PACK.to_string()]
        } else {
            vec![
                RESOURCE_INDEX.to_string(),
                RESOURCE_PACK.to_string(),
                format!("gogoke-{version}-windows-x64-unsigned-setup.exe"),
                format!("gogoke-{version}-windows-x64-unsigned-portable.zip"),
            ]
        };
    if hashes.len() != required_names.len()
        || required_names
            .iter()
            .any(|name| !hashes.contains_key(name.as_str()))
    {
        return Err("GOGOKE_RESOURCE_MANIFEST_FORMAT".to_string());
    }
    Ok(ManifestBinding {
        version,
        source_commit: source,
        release_type: (domain == Domain::Formal).then(|| headers["Release-Type"].to_owned()),
        hashes: hashes
            .into_iter()
            .map(|(name, hash)| (name.to_owned(), hash.to_owned()))
            .collect(),
    })
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.bytes().all(|byte| (0x20..=0x7e).contains(&byte)
            && !matches!(byte, b'\\' | b':' | b'*' | b'?' | b'"' | b'<' | b'>' | b'|'))
        && path.split('/').all(|part| {
            if part.is_empty() || part == "." || part == ".." || part.ends_with('.') || part.ends_with(' ') {
                return false;
            }
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            !matches!(
                stem.as_str(),
                "CON"
                    | "PRN"
                    | "AUX"
                    | "NUL"
                    | "COM1"
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
        })
}

fn signed_index(root: &Path) -> Result<SignedIndex, String> {
    let candidate = root.join(CANDIDATE_MANIFEST);
    let formal = root.join(OWNER_MANIFEST);
    let domain = match (candidate.exists(), formal.exists()) {
        (true, false) => Domain::Candidate,
        (false, true) => Domain::Formal,
        _ => return Err("GOGOKE_RESOURCE_SIGNATURE_DOMAIN_AMBIGUOUS".to_string()),
    };
    let name = if domain == Domain::Candidate {
        CANDIDATE_MANIFEST
    } else {
        OWNER_MANIFEST
    };
    let manifest = read_bounded(&root.join(name), MAX_MANIFEST_BYTES)?;
    let signature = read_bounded(&root.join(format!("{name}.sig")), 256)?;
    let (public, prefix) = if domain == Domain::Candidate {
        (CANDIDATE_KEY, CANDIDATE_PREFIX)
    } else {
        (OWNER_KEY, &[][..])
    };
    verify_signature(public, &manifest, &signature, prefix)?;
    let binding = manifest_binding(&manifest, domain)?;
    let index_hash = &binding.hashes[RESOURCE_INDEX];
    let pack_hash = &binding.hashes[RESOURCE_PACK];
    let index_bytes = read_bounded(&root.join(RESOURCE_INDEX), MAX_INDEX_BYTES)?;
    if sha256(&index_bytes) != index_hash.as_str() {
        return Err("GOGOKE_RESOURCE_INDEX_HASH_MISMATCH".to_string());
    }
    let set_id = sha256(&index_bytes);
    let index: ResourceIndex = serde_json::from_slice(&index_bytes)
        .map_err(|_| "GOGOKE_RESOURCE_INDEX_FORMAT".to_string())?;
    if index.schema != "gogoke.resource-index.v1"
        || index.version != binding.version
        || !lowercase_sha(&index.source_commit, 40)
        || !lowercase_sha(&index.generation_id, 64)
        || binding
            .source_commit
            .as_deref()
            .is_some_and(|expected| expected != index.source_commit)
        || index.pack.file_name != RESOURCE_PACK
        || index.pack.sha256 != pack_hash.as_str()
        || index.generation_id != pack_hash.as_str()
    {
        return Err("GOGOKE_RESOURCE_INDEX_IDENTITY_MISMATCH".to_string());
    }
    let pack_record = ByteRecord {
        length: index.pack.length,
        sha256: pack_hash.to_owned(),
    };
    file_sha256(&root.join(RESOURCE_PACK), &pack_record)?;
    Ok(SignedIndex {
        domain,
        index,
        binding,
        set_id,
    })
}

fn active_set_dir(root: &Path) -> Result<PathBuf, String> {
    let pointer = root.join(ACTIVE_SET_POINTER);
    if !pointer.exists() {
        return Ok(root.to_path_buf());
    }
    let bytes = read_bounded(&pointer, 65)?;
    if bytes.len() != 65 || bytes[64] != b'\n' {
        return Err("GOGOKE_RESOURCE_ACTIVE_SET_INVALID".to_string());
    }
    let set_id =
        std::str::from_utf8(&bytes[..64]).map_err(|_| "GOGOKE_RESOURCE_ACTIVE_SET_INVALID")?;
    if !lowercase_sha(set_id, 64) {
        return Err("GOGOKE_RESOURCE_ACTIVE_SET_INVALID".to_string());
    }
    let sets = root.join(RESOURCE_SETS);
    let sets_metadata = fs::symlink_metadata(&sets)
        .map_err(|_| "GOGOKE_RESOURCE_ACTIVE_SET_MISSING".to_string())?;
    if !sets_metadata.is_dir() || sets_metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    let selected = sets.join(set_id);
    let metadata = fs::symlink_metadata(&selected)
        .map_err(|_| "GOGOKE_RESOURCE_ACTIVE_SET_MISSING".to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    Ok(selected)
}

fn stage_signed_set(source: &Path, root: &Path, signed: &SignedIndex) -> Result<PathBuf, String> {
    let sets = root.join(RESOURCE_SETS);
    physical_directory_or_create(&sets)?;
    let destination = sets.join(&signed.set_id);
    if destination.exists() {
        let metadata = fs::symlink_metadata(&destination)
            .map_err(|_| "GOGOKE_RESOURCE_SET_UNREADABLE".to_string())?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
        }
        let current = signed_index(&destination)?;
        if current.domain != signed.domain
            || current.binding.hashes != signed.binding.hashes
            || current.index.source_commit != signed.index.source_commit
        {
            return Err("GOGOKE_RESOURCE_SET_COLLISION".to_string());
        }
        return Ok(destination);
    }
    let temporary = sets.join(format!(".stage-{}", uuid::Uuid::new_v4()));
    physical_directory_or_create(&temporary)?;
    let manifest_name = if signed.domain == Domain::Candidate {
        CANDIDATE_MANIFEST
    } else {
        OWNER_MANIFEST
    };
    for name in [
        RESOURCE_INDEX.to_string(),
        RESOURCE_PACK.to_string(),
        manifest_name.to_string(),
        format!("{manifest_name}.sig"),
    ] {
        fs::copy(source.join(&name), temporary.join(&name))
            .map_err(|_| "GOGOKE_RESOURCE_SET_STAGE_FAILED".to_string())?;
    }
    let staged = signed_index(&temporary)?;
    if staged.domain != signed.domain
        || staged.binding.hashes != signed.binding.hashes
        || staged.index.source_commit != signed.index.source_commit
    {
        return Err("GOGOKE_RESOURCE_SET_STAGE_MISMATCH".to_string());
    }
    fs::rename(&temporary, &destination)
        .map_err(|_| "GOGOKE_RESOURCE_SET_PUBLISH_FAILED".to_string())?;
    Ok(destination)
}

pub(crate) fn activate_resource_set(root: &Path, set_id: &str) -> Result<(), String> {
    if !lowercase_sha(set_id, 64) {
        return Err("GOGOKE_RESOURCE_ACTIVE_SET_INVALID".to_string());
    }
    let selected = root.join(RESOURCE_SETS).join(set_id);
    let signed = signed_index(&selected)?;
    if signed.set_id != set_id {
        return Err("GOGOKE_RESOURCE_ACTIVE_SET_INVALID".to_string());
    }
    let pointer = root.join(ACTIVE_SET_POINTER);
    let partial = root.join(format!(
        ".{ACTIVE_SET_POINTER}.{}.part",
        uuid::Uuid::new_v4()
    ));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|_| "GOGOKE_RESOURCE_POINTER_WRITE_FAILED".to_string())?;
    output
        .write_all(format!("{set_id}\n").as_bytes())
        .and_then(|_| output.sync_all())
        .map_err(|_| "GOGOKE_RESOURCE_POINTER_WRITE_FAILED".to_string())?;
    drop(output);
    let wide = |path: &Path| {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let moved = unsafe {
        MoveFileExW(
            wide(&partial).as_ptr(),
            wide(&pointer).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let _ = fs::remove_file(&partial);
        return Err("GOGOKE_RESOURCE_POINTER_PUBLISH_FAILED".to_string());
    }
    Ok(())
}

fn validate_install_registration(root: &Path, domain: Domain) -> Result<String, String> {
    let key_name = if domain == Domain::Candidate {
        "gogoke-candidate"
    } else {
        "gogoke"
    };
    let registry = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(
            "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key_name}"
        ))
        .map_err(|_| "GOGOKE_INSTALL_REGISTRATION_MISSING".to_string())?;
    let location: String = registry
        .get_value("InstallLocation")
        .map_err(|_| "GOGOKE_INSTALL_REGISTRATION_INVALID".to_string())?;
    let instance: String = registry
        .get_value("InstallInstanceId")
        .map_err(|_| "GOGOKE_INSTALL_REGISTRATION_INVALID".to_string())?;
    let registered_domain: String = registry
        .get_value("InstallDomain")
        .map_err(|_| "GOGOKE_INSTALL_REGISTRATION_INVALID".to_string())?;
    let expected_domain = if domain == Domain::Candidate {
        "CI_CANDIDATE_RESOURCE"
    } else {
        "OWNER_RELEASE"
    };
    if instance.len() < 16
        || registered_domain != expected_domain
        || Path::new(&location)
            .canonicalize()
            .map_err(|_| "GOGOKE_INSTALL_REGISTRATION_INVALID".to_string())?
            != root
                .canonicalize()
                .map_err(|_| "GOGOKE_INSTALL_REGISTRATION_INVALID".to_string())?
    {
        return Err("GOGOKE_INSTALL_REGISTRATION_INVALID".to_string());
    }
    reject_opposite_registered_root(root, domain)?;
    Ok(instance)
}

fn reject_opposite_registered_root(root: &Path, domain: Domain) -> Result<(), String> {
    let opposite = if domain == Domain::Candidate { "gogoke" } else { "gogoke-candidate" };
    let other = RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{opposite}"
    ));
    let Ok(other) = other else { return Ok(()); };
    let location: String = other.get_value("InstallLocation")
        .map_err(|_| "GOGOKE_INSTALL_OPPOSITE_REGISTRATION_INVALID".to_string())?;
    let other_root = Path::new(&location);
    let same = if other_root.exists() {
        other_root.canonicalize().map_err(|_| "GOGOKE_INSTALL_OPPOSITE_REGISTRATION_INVALID")?
            == root.canonicalize().map_err(|_| "GOGOKE_INSTALL_ROOT_UNAVAILABLE")?
    } else {
        other_root.to_string_lossy().eq_ignore_ascii_case(&root.to_string_lossy())
    };
    if same { return Err("GOGOKE_INSTALL_DOMAIN_ROOT_COLLISION".to_string()); }
    Ok(())
}

fn collect_files(root: &Path, relative: &str, found: &mut HashSet<String>) -> Result<(), String> {
    let directory = root.join(relative);
    let root_metadata = fs::symlink_metadata(&directory)
        .map_err(|_| "GOGOKE_RESOURCE_GENERATION_MISSING".to_string())?;
    if !root_metadata.is_dir() || root_metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    for entry in
        fs::read_dir(&directory).map_err(|_| "GOGOKE_RESOURCE_GENERATION_MISSING".to_string())?
    {
        let entry = entry.map_err(|_| "GOGOKE_RESOURCE_GENERATION_UNREADABLE".to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "GOGOKE_RESOURCE_PATH_UNSAFE".to_string())?;
        let child = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        if !safe_relative_path(&child) {
            return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
        }
        let metadata = fs::symlink_metadata(entry.path())
            .map_err(|_| "GOGOKE_RESOURCE_GENERATION_UNREADABLE".to_string())?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
        }
        if metadata.is_dir() {
            collect_files(root, &child, found)?;
        } else if metadata.is_file() {
            if !found.insert(child) {
                return Err("GOGOKE_RESOURCE_PATH_COLLISION".to_string());
            }
        } else {
            return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
        }
    }
    Ok(())
}

fn indexed_files(index: &ResourceIndex) -> Result<HashMap<String, ByteRecord>, String> {
    let mut expected = HashMap::new();
    let mut folded = HashSet::new();
    for file in &index.files {
        if !safe_relative_path(&file.path)
            || !(file.path.starts_with("frontend/") || file.path.starts_with("dist/"))
            || matches!(
                file.path
                    .rsplit('.')
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
                "exe"
                    | "dll"
                    | "node"
                    | "sys"
                    | "msi"
                    | "msix"
                    | "com"
                    | "scr"
                    | "cpl"
                    | "ocx"
                    | "bat"
                    | "cmd"
                    | "ps1"
            )
            || !lowercase_sha(&file.sha256, 64)
            || !folded.insert(file.path.to_ascii_lowercase())
        {
            return Err("GOGOKE_RESOURCE_INDEX_FORMAT".to_string());
        }
        expected.insert(
            file.path.clone(),
            ByteRecord {
                length: file.length,
                sha256: file.sha256.clone(),
            },
        );
    }
    if !expected.contains_key("frontend/index.html") || !expected.contains_key("dist/bin.mjs") {
        return Err("GOGOKE_RESOURCE_ENTRY_MISSING".to_string());
    }
    Ok(expected)
}

fn reserved_installed_path(folded: &str) -> bool {
    const FIXED_FILES: &[&str] = &[
        "gogoke.exe",
        "gogoke-native-host.exe",
        "gogoke-service/runtime/node.exe",
        "resource-index.json",
        "gogoke-resources.windows.zip",
        "sha256sums.windows",
        "sha256sums.windows.sig",
        "candidate-resources.windows",
        "candidate-resources.windows.sig",
        "gogoke-current-resource-set",
        "gogoke-install-receipt.ini",
        "uninstall.exe",
    ];
    FIXED_FILES.iter().any(|fixed| {
        folded
            .strip_prefix(*fixed)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }) || matches!(folded, "gogoke-service" | "gogoke-service/runtime")
        || ["gogoke-resource-sets", "gogoke-service/generations"]
            .iter()
            .any(|directory| {
                folded
                    .strip_prefix(*directory)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
            })
        || folded.starts_with(".gogoke-current-resource-set.")
        || folded.starts_with(".gogoke-install-receipt.ini.")
}

fn installed_files(index: &ResourceIndex) -> Result<HashMap<String, ByteRecord>, String> {
    if index.installed_files.is_empty() {
        return Err("GOGOKE_INSTALL_FILE_LIST_EMPTY".to_string());
    }
    let mut expected = HashMap::new();
    let mut folded = HashSet::new();
    for file in &index.installed_files {
        let folded_path = file.path.to_ascii_lowercase();
        if !safe_relative_path(&file.path)
            || !lowercase_sha(&file.sha256, 64)
            || reserved_installed_path(&folded_path)
            || !folded.insert(folded_path)
        {
            return Err("GOGOKE_INSTALL_FILE_LIST_INVALID".to_string());
        }
        expected.insert(
            file.path.clone(),
            ByteRecord {
                length: file.length,
                sha256: file.sha256.clone(),
            },
        );
    }
    Ok(expected)
}

fn verify_installed_files(
    root: &Path,
    expected: &HashMap<String, ByteRecord>,
) -> Result<(), String> {
    for (path, record) in expected {
        file_sha256(&root.join(path), record)?;
    }
    Ok(())
}

fn verify_generation(
    generation_root: &Path,
    expected: &HashMap<String, ByteRecord>,
) -> Result<(), String> {
    for (path, record) in expected {
        file_sha256(&generation_root.join(path), record)?;
    }
    let mut actual = HashSet::new();
    collect_files(generation_root, "", &mut actual)?;
    if actual.len() != expected.len() || actual.iter().any(|path| !expected.contains_key(path)) {
        return Err("GOGOKE_RESOURCE_GENERATION_EXTRA_FILE".to_string());
    }
    Ok(())
}

pub(crate) fn verify_bootstrap() -> Result<VerifiedResources, String> {
    let executable =
        std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE".to_string())?;
    let root = executable.parent().ok_or("GOGOKE_EXE_PATH_UNAVAILABLE")?;
    let set_dir = active_set_dir(root)?;
    let SignedIndex {
        domain,
        index,
        set_id,
        ..
    } = signed_index(&set_dir)?;
    if set_dir != root
        && set_dir.file_name().and_then(|value| value.to_str()) != Some(set_id.as_str())
    {
        return Err("GOGOKE_RESOURCE_ACTIVE_SET_INVALID".to_string());
    }
    let shell = if domain == Domain::Candidate {
        &index.executables.installed_shell
    } else if file_sha256(&executable, &index.executables.installed_shell).is_ok() {
        &index.executables.installed_shell
    } else {
        &index.executables.portable_shell
    };
    file_sha256(&executable, shell)?;
    if shell.sha256 == index.executables.installed_shell.sha256 || domain == Domain::Candidate {
        let _ = validate_install_registration(root, domain)?;
    }
    let native_host_path = root.join("gogoke-native-host.exe");
    file_sha256(&native_host_path, &index.executables.native_host)?;
    let service_root = root.join("gogoke-service");
    let node_runtime_path = service_root.join("runtime/node.exe");
    file_sha256(&node_runtime_path, &index.executables.node)?;
    let generation_root = service_root.join("generations").join(&index.generation_id);
    let expected = indexed_files(&index)?;
    verify_generation(&generation_root, &expected)?;
    let static_files = installed_files(&index)?;
    verify_installed_files(root, &static_files)?;
    Ok(VerifiedResources {
        domain,
        version: index.version,
        source_commit: index.source_commit,
        generation_id: index.generation_id,
        set_id,
        generation_root,
        service_root,
        install_root: root.to_path_buf(),
        native_host_path,
        node_runtime_path,
        native_host: index.executables.native_host.clone(),
        node: index.executables.node.clone(),
        executables: index.executables,
        files: Arc::new(expected),
        installed_files: Arc::new(static_files),
    })
}

pub(crate) fn verify_install_set(path: &Path) -> Result<(), String> {
    let signed = signed_index(path)?;
    if signed.domain == Domain::Formal && signed.binding.release_type.as_deref() != Some("full") {
        return Err("GOGOKE_INSTALL_SET_NOT_FULL".to_string());
    }
    if signed.domain == Domain::Formal {
        for (name, hash) in &signed.binding.hashes {
            if name == RESOURCE_INDEX || name == RESOURCE_PACK {
                continue;
            }
            let file = path.join(name);
            let length = fs::symlink_metadata(&file)
                .map_err(|_| "GOGOKE_INSTALL_ASSET_MISSING".to_string())?
                .len();
            file_sha256(
                &file,
                &ByteRecord {
                    length,
                    sha256: hash.clone(),
                },
            )?;
        }
    }
    Ok(())
}

pub(crate) fn verify_install_target(source: &Path, target: &Path) -> Result<(), String> {
    verify_install_set(source)?;
    if !source.is_absolute() || !target.is_absolute()
        || target.components().any(|component| matches!(component, std::path::Component::ParentDir | std::path::Component::CurDir)) {
        return Err("GOGOKE_INSTALL_TARGET_INVALID".to_string());
    }
    let signed = signed_index(source)?;
    let verifier = std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE")?;
    file_sha256(&verifier, &signed.index.executables.installed_shell)?;
    let parent = target.parent().ok_or("GOGOKE_INSTALL_TARGET_INVALID")?;
    for path in parent.ancestors() {
        if !path.exists() { return Err("GOGOKE_INSTALL_PARENT_MISSING".to_string()); }
        let metadata = fs::symlink_metadata(path).map_err(|_| "GOGOKE_INSTALL_PARENT_UNAVAILABLE")?;
        if metadata.file_attributes() & REPARSE_POINT != 0 { return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string()); }
    }
    let normalized = if target.exists() {
        let metadata = fs::symlink_metadata(target).map_err(|_| "GOGOKE_INSTALL_TARGET_INVALID")?;
        if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
        }
        if fs::read_dir(target)
            .map_err(|_| "GOGOKE_INSTALL_TARGET_INVALID")?
            .next()
            .is_some()
        {
            return Err("GOGOKE_INSTALL_EXISTING_TARGET_REQUIRES_UPDATE".to_string());
        }
        target.canonicalize().map_err(|_| "GOGOKE_INSTALL_TARGET_INVALID")?
    } else {
        parent.canonicalize().map_err(|_| "GOGOKE_INSTALL_PARENT_UNAVAILABLE")?
            .join(target.file_name().ok_or("GOGOKE_INSTALL_TARGET_INVALID")?)
    };
    let registry_base = RegKey::predef(HKEY_CURRENT_USER);
    for (key_name, domain) in [("gogoke", Domain::Formal), ("gogoke-candidate", Domain::Candidate)] {
        let key = registry_base.open_subkey(format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{key_name}"));
        let Ok(key) = key else { continue; };
        let registered: String = key.get_value("InstallLocation").map_err(|_| "GOGOKE_INSTALL_REGISTRATION_INVALID")?;
        let registered_path = Path::new(&registered).canonicalize().unwrap_or_else(|_| PathBuf::from(&registered));
        if registered_path.to_string_lossy().eq_ignore_ascii_case(&normalized.to_string_lossy()) {
            if domain != signed.domain { return Err("GOGOKE_INSTALL_DOMAIN_ROOT_COLLISION".to_string()); }
        } else if domain == signed.domain {
            return Err("GOGOKE_INSTALL_DOMAIN_ALREADY_REGISTERED".to_string());
        }
    }
    Ok(())
}

fn physical_directory_or_create(path: &Path) -> Result<(), String> {
    if path.exists() {
        let meta = fs::symlink_metadata(path)
            .map_err(|_| "GOGOKE_RESOURCE_DIRECTORY_UNREADABLE".to_string())?;
        if !meta.is_dir() || meta.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
        }
        return Ok(());
    }
    fs::create_dir(path).map_err(|_| "GOGOKE_RESOURCE_DIRECTORY_CREATE_FAILED".to_string())?;
    let meta = fs::symlink_metadata(path)
        .map_err(|_| "GOGOKE_RESOURCE_DIRECTORY_UNREADABLE".to_string())?;
    if !meta.is_dir() || meta.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    Ok(())
}

fn extract_generation(
    pack_path: &Path,
    generation_root: &Path,
    expected: &HashMap<String, ByteRecord>,
) -> Result<(), String> {
    let file =
        fs::File::open(pack_path).map_err(|_| "GOGOKE_RESOURCE_PACK_UNREADABLE".to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|_| "GOGOKE_RESOURCE_PACK_INVALID".to_string())?;
    if zip.len() != expected.len() {
        return Err("GOGOKE_RESOURCE_PACK_ENTRY_COUNT".to_string());
    }
    let mut seen = HashSet::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|_| "GOGOKE_RESOURCE_PACK_INVALID".to_string())?;
        let relative = entry.name().to_owned();
        let record = expected
            .get(&relative)
            .ok_or("GOGOKE_RESOURCE_PACK_UNLISTED_ENTRY")?;
        if !safe_relative_path(&relative)
            || !entry.is_file()
            || entry.compression() != CompressionMethod::Stored
            || entry.size() != record.length
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 != 0o100000)
            || !seen.insert(relative.to_ascii_lowercase())
        {
            return Err("GOGOKE_RESOURCE_PACK_UNSAFE_ENTRY".to_string());
        }
        let target = generation_root.join(&relative);
        let mut parent = generation_root.to_path_buf();
        for part in relative.split('/').take(relative.matches('/').count()) {
            parent.push(part);
            physical_directory_or_create(&parent)?;
        }
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|_| "GOGOKE_RESOURCE_EXTRACT_CREATE_FAILED".to_string())?;
        let mut digest = Sha256::new();
        let mut length = 0u64;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let size = entry
                .read(&mut buffer)
                .map_err(|_| "GOGOKE_RESOURCE_PACK_INVALID".to_string())?;
            if size == 0 {
                break;
            }
            length = length
                .checked_add(size as u64)
                .ok_or("GOGOKE_RESOURCE_PACK_SIZE")?;
            if length > record.length {
                return Err("GOGOKE_RESOURCE_PACK_SIZE".to_string());
            }
            digest.update(&buffer[..size]);
            output
                .write_all(&buffer[..size])
                .map_err(|_| "GOGOKE_RESOURCE_EXTRACT_WRITE_FAILED".to_string())?;
        }
        output
            .sync_all()
            .map_err(|_| "GOGOKE_RESOURCE_EXTRACT_WRITE_FAILED".to_string())?;
        if length != record.length || format!("{:x}", digest.finalize()) != record.sha256 {
            return Err("GOGOKE_RESOURCE_PACK_ENTRY_HASH".to_string());
        }
    }
    Ok(())
}

pub(crate) fn install_resources(source: &Path) -> Result<(), String> {
    if !source.is_absolute() {
        return Err("GOGOKE_INSTALL_SOURCE_NOT_ABSOLUTE".to_string());
    }
    verify_install_set(source)?;
    let source_signed = signed_index(source)?;
    let executable =
        std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE".to_string())?;
    let root = executable.parent().ok_or("GOGOKE_EXE_PATH_UNAVAILABLE")?;
    reject_opposite_registered_root(root, source_signed.domain)?;
    let installed_signed = signed_index(root)?;
    if source_signed.domain != installed_signed.domain
        || source_signed.binding.hashes != installed_signed.binding.hashes
        || source_signed.index.generation_id != installed_signed.index.generation_id
    {
        return Err("GOGOKE_INSTALL_SET_IDENTITY_MISMATCH".to_string());
    }
    let manifest_name = if source_signed.domain == Domain::Candidate {
        CANDIDATE_MANIFEST
    } else {
        OWNER_MANIFEST
    };
    for name in [manifest_name.to_string(), format!("{manifest_name}.sig")] {
        if read_bounded(&source.join(&name), MAX_MANIFEST_BYTES)?
            != read_bounded(&root.join(&name), MAX_MANIFEST_BYTES)?
        {
            return Err("GOGOKE_INSTALL_SET_IDENTITY_MISMATCH".to_string());
        }
    }
    file_sha256(
        &executable,
        &installed_signed.index.executables.installed_shell,
    )?;
    file_sha256(
        &root.join("gogoke-native-host.exe"),
        &installed_signed.index.executables.native_host,
    )?;
    file_sha256(
        &root.join("gogoke-service/runtime/node.exe"),
        &installed_signed.index.executables.node,
    )?;
    verify_installed_files(root, &installed_files(&installed_signed.index)?)?;
    let expected = indexed_files(&installed_signed.index)?;
    let service_root = root.join("gogoke-service");
    physical_directory_or_create(&service_root)?;
    let generations = service_root.join("generations");
    physical_directory_or_create(&generations)?;
    let destination = generations.join(&installed_signed.index.generation_id);
    if !destination.exists() {
        let temporary = generations.join(format!(".stage-{}", uuid::Uuid::new_v4()));
        physical_directory_or_create(&temporary)?;
        extract_generation(&root.join(RESOURCE_PACK), &temporary, &expected)?;
        verify_generation(&temporary, &expected)?;
        fs::rename(&temporary, &destination)
            .map_err(|_| "GOGOKE_RESOURCE_GENERATION_PUBLISH_FAILED".to_string())?;
    }
    verify_generation(&destination, &expected)?;
    stage_signed_set(root, root, &installed_signed)?;
    activate_resource_set(root, &installed_signed.set_id)?;
    let receipt = root.join(INSTALL_RECEIPT);
    if receipt.exists() {
        return Err("GOGOKE_INSTALL_RECEIPT_ALREADY_EXISTS".to_string());
    }
    let domain = match installed_signed.domain {
        Domain::Candidate => "CI_CANDIDATE_RESOURCE",
        Domain::Formal => "OWNER_RELEASE",
    };
    let payload = format!(
        "[Gogoke]\nVersion={}\nDomain={domain}\n",
        installed_signed.index.version
    );
    let partial = root.join(format!(".{INSTALL_RECEIPT}.{}.part", uuid::Uuid::new_v4()));
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)
        .map_err(|_| "GOGOKE_INSTALL_RECEIPT_CREATE_FAILED".to_string())?;
    output
        .write_all(payload.as_bytes())
        .and_then(|_| output.sync_all())
        .map_err(|_| "GOGOKE_INSTALL_RECEIPT_WRITE_FAILED".to_string())?;
    drop(output);
    fs::rename(&partial, &receipt).map_err(|_| "GOGOKE_INSTALL_RECEIPT_PUBLISH_FAILED".to_string())
}

pub(crate) fn stage_resource_update(
    source: &Path,
    current: &VerifiedResources,
) -> Result<VerifiedResources, String> {
    if current.domain != Domain::Formal {
        return Err("GOGOKE_CANDIDATE_RELEASE_DISABLED".to_string());
    }
    let signed = signed_index(source)?;
    if signed.domain != Domain::Formal
        || signed.binding.release_type.as_deref() != Some("resources")
    {
        return Err("GOGOKE_RESOURCE_UPDATE_KIND_INVALID".to_string());
    }
    let index = &signed.index;
    let old_version =
        semver::Version::parse(&current.version).map_err(|_| "GOGOKE_RESOURCE_VERSION_INVALID")?;
    let new_version =
        semver::Version::parse(&index.version).map_err(|_| "GOGOKE_RESOURCE_VERSION_INVALID")?;
    if new_version <= old_version {
        return Err("GOGOKE_RESOURCE_UPDATE_VERSION_NOT_NEWER".to_string());
    }
    if index.executables != current.executables {
        return Err("GOGOKE_RESOURCE_UPDATE_EXECUTABLE_CHANGED".to_string());
    }
    let static_files = installed_files(index)?;
    if &static_files != current.installed_files.as_ref() {
        return Err("GOGOKE_RESOURCE_UPDATE_INSTALLED_FILES_CHANGED".to_string());
    }
    validate_install_registration(&current.install_root, Domain::Formal)?;
    let executable = std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE")?;
    file_sha256(&executable, &index.executables.installed_shell)?;
    current.verify_runtime_files()?;
    let expected = indexed_files(index)?;
    let set_dir = stage_signed_set(source, &current.install_root, &signed)?;
    let generations = current.service_root.join("generations");
    physical_directory_or_create(&generations)?;
    let generation_root = generations.join(&index.generation_id);
    if !generation_root.exists() {
        let temporary = generations.join(format!(".stage-{}", uuid::Uuid::new_v4()));
        physical_directory_or_create(&temporary)?;
        extract_generation(&set_dir.join(RESOURCE_PACK), &temporary, &expected)?;
        verify_generation(&temporary, &expected)?;
        fs::rename(&temporary, &generation_root)
            .map_err(|_| "GOGOKE_RESOURCE_GENERATION_PUBLISH_FAILED")?;
    }
    verify_generation(&generation_root, &expected)?;
    Ok(VerifiedResources {
        domain: Domain::Formal,
        version: index.version.clone(),
        source_commit: index.source_commit.clone(),
        generation_id: index.generation_id.clone(),
        set_id: signed.set_id.clone(),
        generation_root,
        service_root: current.service_root.clone(),
        install_root: current.install_root.clone(),
        native_host_path: current.native_host_path.clone(),
        node_runtime_path: current.node_runtime_path.clone(),
        native_host: index.executables.native_host.clone(),
        node: index.executables.node.clone(),
        executables: index.executables.clone(),
        files: Arc::new(expected),
        installed_files: Arc::new(static_files),
    })
}

impl VerifiedResources {
    pub(crate) fn registered_instance(&self) -> Result<String, String> {
        let exe = std::env::current_exe().map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE")?;
        let installed = self.install_root.join("gogoke.exe");
        if exe
            .canonicalize()
            .map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE")?
            != installed
                .canonicalize()
                .map_err(|_| "GOGOKE_EXE_PATH_UNAVAILABLE")?
        {
            return Err("GOGOKE_UNINSTALL_NOT_INSTALLED_SHELL".to_string());
        }
        file_sha256(&exe, &self.executables.installed_shell)?;
        validate_install_registration(&self.install_root, self.domain)
    }

    pub(crate) fn uninstall_registry_key(&self) -> &'static str {
        if self.domain == Domain::Candidate {
            "gogoke-candidate"
        } else {
            "gogoke"
        }
    }

    pub(crate) fn owned_files_for_uninstall(&self) -> Result<Vec<(PathBuf, String)>, String> {
        self.registered_instance()?;
        let root = &self.install_root;
        let mut files = std::collections::BTreeMap::<PathBuf, String>::new();
        let mut add = |path: PathBuf, hash: String| -> Result<(), String> {
            if let Some(old) = files.insert(path, hash.clone()) {
                if old != hash {
                    return Err("GOGOKE_UNINSTALL_OWNERSHIP_CONFLICT".to_string());
                }
            }
            Ok(())
        };
        add(
            root.join("gogoke.exe"),
            self.executables.installed_shell.sha256.clone(),
        )?;
        add(
            root.join("gogoke-native-host.exe"),
            self.executables.native_host.sha256.clone(),
        )?;
        add(
            root.join("gogoke-service/runtime/node.exe"),
            self.executables.node.sha256.clone(),
        )?;
        for (path, record) in self.installed_files.iter() {
            add(root.join(path), record.sha256.clone())?;
        }
        let mut set_directories = Vec::new();
        let sets = root.join(RESOURCE_SETS);
        let sets_metadata =
            fs::symlink_metadata(&sets).map_err(|_| "GOGOKE_RESOURCE_SET_MISSING".to_string())?;
        if !sets_metadata.is_dir() || sets_metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
        }
        for entry in
            fs::read_dir(&sets).map_err(|_| "GOGOKE_RESOURCE_SET_UNREADABLE".to_string())?
        {
            let entry = entry.map_err(|_| "GOGOKE_RESOURCE_SET_UNREADABLE".to_string())?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "GOGOKE_RESOURCE_SET_INVALID".to_string())?;
            if !lowercase_sha(&name, 64) {
                continue;
            }
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .map_err(|_| "GOGOKE_RESOURCE_SET_UNREADABLE".to_string())?;
            if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
                continue;
            }
            if signed_index(&path).is_ok() {
                set_directories.push(path);
            }
        }
        if set_directories.is_empty() {
            return Err("GOGOKE_RESOURCE_SET_MISSING".to_string());
        }
        for set in set_directories {
            let signed = signed_index(&set)?;
            if signed.domain != self.domain
                || signed.set_id
                    != set
                        .file_name()
                        .and_then(|value| value.to_str())
                        .unwrap_or_default()
                || signed.index.executables != self.executables
                || &installed_files(&signed.index)? != self.installed_files.as_ref()
            {
                return Err("GOGOKE_UNINSTALL_SET_IDENTITY_MISMATCH".to_string());
            }
            let generation = root
                .join("gogoke-service/generations")
                .join(&signed.index.generation_id);
            let expected = indexed_files(&signed.index)?;
            verify_generation(&generation, &expected)?;
            for (path, record) in expected {
                add(generation.join(path), record.sha256)?;
            }
            let manifest = if signed.domain == Domain::Candidate {
                CANDIDATE_MANIFEST
            } else {
                OWNER_MANIFEST
            };
            for name in [
                RESOURCE_INDEX.to_string(),
                RESOURCE_PACK.to_string(),
                manifest.to_string(),
                format!("{manifest}.sig"),
            ] {
                let path = set.join(name);
                let hash =
                    if path.file_name().and_then(|value| value.to_str()) == Some(RESOURCE_PACK) {
                        signed.index.pack.sha256.clone()
                    } else {
                        sha256(&read_bounded(
                            &path,
                            MAX_MANIFEST_BYTES.max(MAX_INDEX_BYTES),
                        )?)
                    };
                add(path, hash)?;
            }
        }
        let root_signed = signed_index(root)?;
        if root_signed.domain != self.domain || root_signed.index.executables != self.executables {
            return Err("GOGOKE_UNINSTALL_ROOT_SET_MISMATCH".to_string());
        }
        let manifest = if self.domain == Domain::Candidate {
            CANDIDATE_MANIFEST
        } else {
            OWNER_MANIFEST
        };
        for name in [
            RESOURCE_INDEX.to_string(),
            RESOURCE_PACK.to_string(),
            manifest.to_string(),
            format!("{manifest}.sig"),
        ] {
            let path = root.join(name);
            let hash = if path.file_name().and_then(|value| value.to_str()) == Some(RESOURCE_PACK) {
                root_signed.index.pack.sha256.clone()
            } else {
                sha256(&read_bounded(
                    &path,
                    MAX_MANIFEST_BYTES.max(MAX_INDEX_BYTES),
                )?)
            };
            add(path, hash)?;
        }
        let pointer = root.join(ACTIVE_SET_POINTER);
        add(pointer.clone(), sha256(&read_bounded(&pointer, 65)?))?;
        for (path, hash) in &files {
            let metadata = fs::symlink_metadata(path)
                .map_err(|_| "GOGOKE_UNINSTALL_OWNED_FILE_MISSING".to_string())?;
            file_sha256(
                path,
                &ByteRecord {
                    length: metadata.len(),
                    sha256: hash.clone(),
                },
            )?;
        }
        Ok(files.into_iter().collect())
    }

    pub(crate) fn verify_runtime_files(&self) -> Result<(), String> {
        file_sha256(&self.native_host_path, &self.native_host)?;
        file_sha256(&self.node_runtime_path, &self.node)?;
        let root = self
            .service_root
            .parent()
            .ok_or("GOGOKE_INSTALL_ROOT_UNAVAILABLE")?;
        verify_installed_files(root, &self.installed_files)?;
        if !self.files.contains_key("dist/bin.mjs") {
            return Err("GOGOKE_RESOURCE_ENTRY_MISSING".to_string());
        }
        for (path, record) in self.files.iter().filter(|(path, _)| path.starts_with("dist/")) {
            file_sha256(&self.generation_root.join(path), record)?;
        }
        Ok(())
    }

    pub(crate) fn read_frontend(&self, request_path: &str) -> Result<Vec<u8>, String> {
        let requested = request_path.trim_start_matches('/');
        if !safe_relative_path(requested) {
            return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
        }
        let relative = format!("frontend/{requested}");
        let record = self
            .files
            .get(&relative)
            .ok_or("GOGOKE_RESOURCE_PATH_UNLISTED")?;
        let path = self.generation_root.join(relative);
        read_verified_file(&path, record, true)
    }
}

pub(crate) fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn signed_manifest_shape_separates_full_resources_and_candidate() {
        let full = format!("# gogoke-Version: 1.2.3\n# gogoke-Release-Type: full\n{HASH}  resource-index.json\n{HASH}  gogoke-resources.windows.zip\n{HASH}  gogoke-1.2.3-windows-x64-unsigned-setup.exe\n{HASH}  gogoke-1.2.3-windows-x64-unsigned-portable.zip\n");
        let binding = manifest_binding(full.as_bytes(), Domain::Formal).expect("full shape");
        assert_eq!(binding.release_type.as_deref(), Some("full"));
        let with_duplicate = full.replace(
            "# gogoke-Version: 1.2.3\n",
            "# gogoke-Version: 1.2.3\n# gogoke-Version: 9.9.9\n",
        );
        assert!(manifest_binding(with_duplicate.as_bytes(), Domain::Formal).is_err());
        let resource_with_installer = full.replace("full", "resources");
        assert!(manifest_binding(resource_with_installer.as_bytes(), Domain::Formal).is_err());
        let wrong_domain = full.replace(
            "# gogoke-Release-Type: full",
            "# gogoke-Candidate-Purpose: CI_CANDIDATE_RESOURCE",
        );
        assert!(manifest_binding(wrong_domain.as_bytes(), Domain::Candidate).is_err());
    }

    #[test]
    fn resource_paths_reject_traversal_device_names_and_native_payloads() {
        for path in [
            "../frontend/index.html",
            "frontend/CON.txt",
            "frontend/asset.dll",
            "frontend/dir/../index.html",
        ] {
            let index = ResourceIndex {
                schema: "gogoke.resource-index.v1".to_string(),
                source_commit: "0".repeat(40),
                version: "1.2.3".to_string(),
                generation_id: "0".repeat(64),
                pack: PackRecord {
                    file_name: RESOURCE_PACK.to_string(),
                    length: 0,
                    sha256: "0".repeat(64),
                },
                files: ["frontend/index.html", "dist/bin.mjs", path]
                    .into_iter()
                    .map(|item| IndexedFile {
                        path: item.to_string(),
                        length: 0,
                        sha256: "0".repeat(64),
                    })
                    .collect(),
                installed_files: vec![],
                executables: Executables {
                    portable_shell: ByteRecord {
                        length: 0,
                        sha256: "0".repeat(64),
                    },
                    installed_shell: ByteRecord {
                        length: 0,
                        sha256: "0".repeat(64),
                    },
                    native_host: ByteRecord {
                        length: 0,
                        sha256: "0".repeat(64),
                    },
                    node: ByteRecord {
                        length: 0,
                        sha256: "0".repeat(64),
                    },
                },
            };
            assert!(
                indexed_files(&index).is_err(),
                "unsafe path was accepted: {path}"
            );
        }
    }

    #[test]
    fn installed_inventory_preserves_product_created_install_paths() {
        let bytes = || ByteRecord {
            length: 0,
            sha256: HASH.to_string(),
        };
        let mut index = ResourceIndex {
            schema: "gogoke.resource-index.v1".to_string(),
            source_commit: "0".repeat(40),
            version: "1.2.3".to_string(),
            generation_id: HASH.to_string(),
            pack: PackRecord {
                file_name: RESOURCE_PACK.to_string(),
                length: 0,
                sha256: HASH.to_string(),
            },
            files: vec![],
            installed_files: vec![IndexedFile {
                path: String::new(),
                length: 0,
                sha256: HASH.to_string(),
            }],
            executables: Executables {
                portable_shell: bytes(),
                installed_shell: bytes(),
                native_host: bytes(),
                node: bytes(),
            },
        };
        for path in [
            "gogoke.exe/child",
            "gogoke-install-receipt.ini",
            "gogoke-current-resource-set",
            "gogoke-resource-sets",
            "gogoke-resource-sets/set/file",
            "GOGOKE-service/Generations",
            "GOGOKE-service/Generations/child",
            ".gogoke-current-resource-set.fake.part",
            ".gogoke-install-receipt.ini.fake.part",
        ] {
            index.installed_files[0].path = path.to_string();
            assert!(installed_files(&index).is_err(), "reserved path accepted: {path}");
        }
        index.installed_files[0].path = "gogoke-service/node_modules/package.json".to_string();
        assert!(installed_files(&index).is_ok());
    }
}
