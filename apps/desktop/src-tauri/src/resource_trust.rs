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
use std::os::windows::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, MoveFileExW, SetFileInformationByHandle, DELETE, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_READ,
    FileRenameInfo, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, OPEN_EXISTING,
};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
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

// Temporary cloud-only installer diagnosis. Remove after the first failing
// installed invocation is identified; this is never an authority input.
pub(crate) fn write_ci_install_error_once(message: &str) {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return;
    }
    let (Ok(run), Ok(attempt)) = (
        std::env::var("GITHUB_RUN_ID"),
        std::env::var("GITHUB_RUN_ATTEMPT"),
    ) else {
        return;
    };
    if !run.bytes().all(|byte| byte.is_ascii_digit())
        || !attempt.bytes().all(|byte| byte.is_ascii_digit())
        || message.len() > 256
    {
        return;
    }
    let path = std::env::temp_dir().join(format!("gogoke-install-error-{run}-{attempt}.txt"));
    if let Ok(mut output) = fs::OpenOptions::new().write(true).create_new(true).open(path) {
        let _ = output.write_all(message.as_bytes());
    }
}

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
    executables: Executables,
    files: Arc<HashMap<String, ByteRecord>>,
    installed_files: Arc<HashMap<String, ByteRecord>>,
    runtime_lease: Arc<RuntimeLease>,
}

/// Physical path custody for bytes used by a running resource generation.
/// Directory handles prevent an ancestor being replaced with a junction after
/// verification; leaf handles deny write/delete sharing until the last lease
/// holder drops them. Paths remain useful to Node, which needs path arguments.
#[derive(Default)]
pub(crate) struct RuntimeLease {
    directories: HashMap<PathBuf, fs::File>,
    files: HashMap<PathBuf, (fs::File, ByteRecord)>,
}

impl RuntimeLease {
    pub(crate) fn module_file_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<_> = self.files.keys().cloned().collect();
        paths.sort();
        paths
    }

    pub(crate) fn pin_generated_file(&mut self, path: &Path, bytes: &[u8]) -> Result<(), String> {
        self.pin_file(path, &ByteRecord {
            length: bytes.len() as u64,
            sha256: sha256(bytes),
        }, false)?;
        Ok(())
    }

    fn pin_ancestors(&mut self, path: &Path) -> Result<(), String> {
        if !path.is_absolute() {
            return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
        }
        let parent = path.parent().ok_or("GOGOKE_RESOURCE_PATH_UNSAFE")?;
        for ancestor in parent.ancestors().collect::<Vec<_>>().into_iter().rev() {
            if self.directories.contains_key(ancestor) {
                continue;
            }
            let directory = fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(ancestor)
                .map_err(|error: std::io::Error| {
                    write_ci_install_error_once(&format!(
                        "pin-ancestor-open:{}:{}",
                        ancestor.file_name().and_then(|name| name.to_str()).unwrap_or("root"),
                        error.raw_os_error().unwrap_or(0)
                    ));
                    "GOGOKE_RESOURCE_DIRECTORY_UNREADABLE".to_string()
                })?;
            let metadata = directory
                .metadata()
                .map_err(|error: std::io::Error| {
                    write_ci_install_error_once(&format!(
                        "pin-ancestor-metadata:{}:{}",
                        ancestor.file_name().and_then(|name| name.to_str()).unwrap_or("root"),
                        error.raw_os_error().unwrap_or(0)
                    ));
                    "GOGOKE_RESOURCE_DIRECTORY_UNREADABLE".to_string()
                })?;
            if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
            }
            self.directories.insert(ancestor.to_path_buf(), directory);
        }
        Ok(())
    }

    fn pin_file(
        &mut self,
        path: &Path,
        expected: &ByteRecord,
        retain_bytes: bool,
    ) -> Result<Vec<u8>, String> {
        self.pin_ancestors(path)?;
        if let Some((file, prior)) = self.files.get(path) {
            if prior != expected {
                return Err("GOGOKE_RESOURCE_FILE_IDENTITY_MISMATCH".to_string());
            }
            return read_verified_open_file(file, expected, retain_bytes);
        }
        let file = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(OPEN_REPARSE_POINT)
            .open(path)
            .map_err(|_| "GOGOKE_RESOURCE_FILE_UNREADABLE".to_string())?;
        let metadata = file
            .metadata()
            .map_err(|_| "GOGOKE_RESOURCE_FILE_UNREADABLE".to_string())?;
        if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
            return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
        }
        if metadata.len() != expected.length {
            return Err("GOGOKE_RESOURCE_FILE_IDENTITY_MISMATCH".to_string());
        }
        let bytes = read_verified_open_file(&file, expected, retain_bytes)?;
        self.files
            .insert(path.to_path_buf(), (file, expected.clone()));
        Ok(bytes)
    }

    fn reverify(&self) -> Result<(), String> {
        for (file, expected) in self.files.values() {
            read_verified_open_file(file, expected, false)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct ResourceState(Arc<RwLock<ResourceStateInner>>);

struct ResourceStateInner {
    active: String,
    sets: HashMap<String, VerifiedResources>,
}

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
        let active = resources.set_id.clone();
        let sets = HashMap::from([(active.clone(), resources)]);
        Self(Arc::new(RwLock::new(ResourceStateInner { active, sets })))
    }

    pub(crate) fn current(&self) -> Result<VerifiedResources, String> {
        let state = self.0
            .read()
            .map_err(|_| "GOGOKE_RESOURCE_STATE_UNAVAILABLE".to_string())?;
        state
            .sets
            .get(&state.active)
            .cloned()
            .ok_or_else(|| "GOGOKE_RESOURCE_STATE_UNAVAILABLE".to_string())
    }

    pub(crate) fn replace(&self, resources: VerifiedResources) -> Result<(), String> {
        let mut state = self
            .0
            .write()
            .map_err(|_| "GOGOKE_RESOURCE_STATE_UNAVAILABLE".to_string())?;
        state.active = resources.set_id.clone();
        state.sets.insert(resources.set_id.clone(), resources);
        Ok(())
    }

    pub(crate) fn read_frontend_request(
        &self,
        request_path: &str,
    ) -> Result<(Vec<u8>, &'static str), String> {
        let (set_id, relative) = frontend_request_path(request_path)?;
        let resources = self
            .0
            .read()
            .map_err(|_| "GOGOKE_RESOURCE_STATE_UNAVAILABLE".to_string())?
            .sets
            .get(set_id)
            .cloned()
            .ok_or_else(|| "GOGOKE_RESOURCE_SET_UNAVAILABLE".to_string())?;
        let bytes = resources.read_frontend(&relative)?;
        Ok((bytes, content_type(&relative)))
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
    RuntimeLease::default().pin_file(path, expected, retain_bytes)
}

fn read_verified_open_file(
    file: &fs::File,
    expected: &ByteRecord,
    retain_bytes: bool,
) -> Result<Vec<u8>, String> {
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
            .seek_read(&mut buffer, count)
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

fn frontend_request_path(path: &str) -> Result<(&str, String), String> {
    let (set_id, encoded) = path
        .strip_prefix('/')
        .and_then(|value| value.split_once('/'))
        .ok_or_else(|| "GOGOKE_RESOURCE_PATH_UNSAFE".to_string())?;
    if !lowercase_sha(set_id, 64) {
        return Err("GOGOKE_RESOURCE_SET_ID_INVALID".to_string());
    }
    let source = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(source.len());
    let mut offset = 0;
    while offset < source.len() {
        if source[offset] == b'%' {
            if offset + 2 >= source.len() {
                return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
            }
            let digits = std::str::from_utf8(&source[offset + 1..offset + 3])
                .map_err(|_| "GOGOKE_RESOURCE_PATH_UNSAFE".to_string())?;
            let byte = u8::from_str_radix(digits, 16)
                .map_err(|_| "GOGOKE_RESOURCE_PATH_UNSAFE".to_string())?;
            if byte == b'/' || byte == b'\\' {
                return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
            }
            decoded.push(byte);
            offset += 3;
        } else {
            decoded.push(source[offset]);
            offset += 1;
        }
    }
    let relative = String::from_utf8(decoded)
        .map_err(|_| "GOGOKE_RESOURCE_PATH_UNSAFE".to_string())?;
    if !safe_relative_path(&relative) {
        return Err("GOGOKE_RESOURCE_PATH_UNSAFE".to_string());
    }
    Ok((set_id, relative))
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
    // Hold the physical parent before creating any new leaf. The stage name
    // itself is exclusive; a caller-supplied directory is never reused here.
    let mut parent_lease = RuntimeLease::default();
    parent_lease.pin_ancestors(&temporary)?;
    fs::create_dir(&temporary)
        .map_err(|_| "GOGOKE_RESOURCE_DIRECTORY_CREATE_FAILED".to_string())?;
    let stage_handle = open_owned_stage_directory(&temporary)?;
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
        copy_stage_leaf_no_replace(&source.join(&name), &temporary.join(&name))?;
    }
    let staged = signed_index(&temporary)?;
    if staged.domain != signed.domain
        || staged.binding.hashes != signed.binding.hashes
        || staged.index.source_commit != signed.index.source_commit
    {
        return Err("GOGOKE_RESOURCE_SET_STAGE_MISMATCH".to_string());
    }
    rename_owned_stage_no_replace(&stage_handle, &destination)?;
    Ok(destination)
}

fn open_owned_stage_directory(path: &Path) -> Result<fs::File, String> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(), DELETE | FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
            std::ptr::null(), OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err("GOGOKE_RESOURCE_SET_STAGE_UNOWNED".to_string());
    }
    let directory = unsafe { fs::File::from_raw_handle(raw as _) };
    let metadata = directory.metadata()
        .map_err(|_| "GOGOKE_RESOURCE_SET_STAGE_UNOWNED".to_string())?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    Ok(directory)
}

fn copy_stage_leaf_no_replace(source: &Path, target: &Path) -> Result<(), String> {
    let mut source_file = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
        .custom_flags(OPEN_REPARSE_POINT).open(source)
        .map_err(|_| "GOGOKE_RESOURCE_SET_STAGE_SOURCE_FAILED".to_string())?;
    let metadata = source_file.metadata()
        .map_err(|_| "GOGOKE_RESOURCE_SET_STAGE_SOURCE_FAILED".to_string())?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
    let mut target_file = fs::OpenOptions::new().write(true).create_new(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(OPEN_REPARSE_POINT).open(target)
        .map_err(|_| "GOGOKE_RESOURCE_SET_STAGE_COLLISION".to_string())?;
    std::io::copy(&mut source_file, &mut target_file)
        .and_then(|_| target_file.sync_all())
        .map_err(|_| "GOGOKE_RESOURCE_SET_STAGE_FAILED".to_string())?;
    Ok(())
}

fn rename_owned_stage_no_replace(stage: &fs::File, destination: &Path) -> Result<(), String> {
    let name: Vec<u16> = destination.as_os_str().encode_wide().collect();
    let byte_len = name.len().checked_mul(2)
        .ok_or("GOGOKE_RESOURCE_SET_PUBLISH_FAILED")?;
    let total = std::mem::size_of::<FILE_RENAME_INFO>().checked_add(byte_len)
        .ok_or("GOGOKE_RESOURCE_SET_PUBLISH_FAILED")?;
    let words = total.div_ceil(std::mem::size_of::<usize>());
    let mut storage = vec![0usize; words];
    let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    unsafe {
        (*info).Anonymous.ReplaceIfExists = false;
        (*info).RootDirectory = std::ptr::null_mut();
        (*info).FileNameLength = byte_len as u32;
        std::ptr::copy_nonoverlapping(name.as_ptr(), (*info).FileName.as_mut_ptr(), name.len());
    }
    let success = unsafe {
        SetFileInformationByHandle(
            stage.as_raw_handle() as _, FileRenameInfo,
            info.cast(), total as u32,
        )
    };
    if success == 0 {
        write_ci_install_error_once(&format!(
            "rename-owned-stage:{}",
            std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
        ));
        return Err("GOGOKE_RESOURCE_SET_PUBLISH_FAILED".to_string());
    }
    Ok(())
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
    verify_generation_in_lease(generation_root, expected, &mut RuntimeLease::default())
}

fn verify_generation_in_lease(
    generation_root: &Path,
    expected: &HashMap<String, ByteRecord>,
    lease: &mut RuntimeLease,
) -> Result<(), String> {
    for (path, record) in expected {
        lease.pin_file(&generation_root.join(path), record, false)?;
    }
    let mut actual = HashSet::new();
    collect_files(generation_root, "", &mut actual)?;
    if actual.len() != expected.len() || actual.iter().any(|path| !expected.contains_key(path)) {
        return Err("GOGOKE_RESOURCE_GENERATION_EXTRA_FILE".to_string());
    }
    Ok(())
}

fn build_runtime_lease(
    install_root: &Path,
    generation_root: &Path,
    native_host_path: &Path,
    node_runtime_path: &Path,
    native_host: &ByteRecord,
    node: &ByteRecord,
    installed_files: &HashMap<String, ByteRecord>,
    generation_files: &HashMap<String, ByteRecord>,
) -> Result<Arc<RuntimeLease>, String> {
    let mut lease = RuntimeLease::default();
    lease.pin_file(native_host_path, native_host, false)?;
    lease.pin_file(node_runtime_path, node, false)?;
    for (path, record) in installed_files {
        lease.pin_file(&install_root.join(path), record, false)?;
    }
    if !generation_files.contains_key("dist/bin.mjs") {
        return Err("GOGOKE_RESOURCE_ENTRY_MISSING".to_string());
    }
    for (path, record) in generation_files
        .iter()
        .filter(|(path, _)| path.starts_with("dist/"))
    {
        lease.pin_file(&generation_root.join(path), record, false)?;
    }
    // This check runs while the generation ancestor is held, so the directory
    // enumerated here is the one whose listed service bytes were just opened.
    let mut actual = HashSet::new();
    collect_files(generation_root, "", &mut actual)?;
    if actual.len() != generation_files.len()
        || actual
            .iter()
            .any(|path| !generation_files.contains_key(path))
    {
        return Err("GOGOKE_RESOURCE_GENERATION_EXTRA_FILE".to_string());
    }
    Ok(Arc::new(lease))
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
    let runtime_lease = build_runtime_lease(
        root,
        &generation_root,
        &native_host_path,
        &node_runtime_path,
        &index.executables.native_host,
        &index.executables.node,
        &static_files,
        &expected,
    )?;
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
        executables: index.executables,
        files: Arc::new(expected),
        installed_files: Arc::new(static_files),
        runtime_lease,
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
    directories: &mut RuntimeLease,
) -> Result<(), String> {
    let file = fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
        .custom_flags(OPEN_REPARSE_POINT).open(pack_path)
        .map_err(|_| "GOGOKE_RESOURCE_PACK_UNREADABLE".to_string())?;
    let pack_metadata = file.metadata()
        .map_err(|_| "GOGOKE_RESOURCE_PACK_UNREADABLE".to_string())?;
    if !pack_metadata.is_file() || pack_metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string());
    }
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
        directories.pin_ancestors(&target)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(OPEN_REPARSE_POINT)
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

fn publish_generation_from_pack(
    pack_path: &Path,
    generations: &Path,
    destination: &Path,
    expected: &HashMap<String, ByteRecord>,
) -> Result<(), String> {
    let temporary = generations.join(format!(".stage-{}", uuid::Uuid::new_v4()));
    let mut parent_lease = RuntimeLease::default();
    parent_lease.pin_ancestors(&temporary)?;
    fs::create_dir(&temporary)
        .map_err(|_| "GOGOKE_RESOURCE_DIRECTORY_CREATE_FAILED".to_string())?;
    let stage_handle = open_owned_stage_directory(&temporary)?;
    let mut directories = RuntimeLease::default();
    directories.directories.insert(temporary.clone(), stage_handle);
    extract_generation(pack_path, &temporary, expected, &mut directories)?;
    verify_generation_in_lease(&temporary, expected, &mut directories)?;
    let stage_handle = directories.directories.remove(&temporary)
        .ok_or("GOGOKE_RESOURCE_GENERATION_PUBLISH_FAILED")?;
    drop(directories); // Nested directory handles must close before parent rename.
    rename_owned_stage_no_replace(&stage_handle, destination)
        .map_err(|_| "GOGOKE_RESOURCE_GENERATION_PUBLISH_FAILED".to_string())?;
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
        publish_generation_from_pack(&root.join(RESOURCE_PACK), &generations, &destination, &expected)?;
    }
    verify_generation(&destination, &expected)?;
    stage_signed_set(root, root, &installed_signed)?;
    activate_resource_set(root, &installed_signed.set_id)?;
    let receipt = root.join(INSTALL_RECEIPT);
    if receipt.exists() {
        return Err("GOGOKE_INSTALL_RECEIPT_ALREADY_EXISTS".to_string());
    }
    let payload = install_receipt_payload(&installed_signed.index.version, installed_signed.domain);
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(OPEN_REPARSE_POINT)
        .open(&receipt)
        .map_err(|_| "GOGOKE_INSTALL_RECEIPT_CREATE_FAILED".to_string())?;
    output
        .write_all(payload.as_bytes())
        .and_then(|_| output.sync_all())
        .map_err(|_| "GOGOKE_INSTALL_RECEIPT_WRITE_FAILED".to_string())
}

fn install_receipt_payload(version: &str, domain: Domain) -> String {
    let domain = match domain {
        Domain::Candidate => "CI_CANDIDATE_RESOURCE",
        Domain::Formal => "OWNER_RELEASE",
    };
    format!("[Gogoke]\nVersion={version}\nDomain={domain}\n")
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
    let generations = current.service_root.join("generations");
    physical_directory_or_create(&generations)?;
    let generation_root = generations.join(&index.generation_id);
    if !generation_root.exists() {
        publish_generation_from_pack(&source.join(RESOURCE_PACK), &generations, &generation_root, &expected)?;
    }
    verify_generation(&generation_root, &expected)?;
    let runtime_lease = build_runtime_lease(
        &current.install_root,
        &generation_root,
        &current.native_host_path,
        &current.node_runtime_path,
        &index.executables.native_host,
        &index.executables.node,
        &static_files,
        &expected,
    )?;
    // A normally named signed set must never point at an absent generation.
    stage_signed_set(source, &current.install_root, &signed)?;
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
        executables: index.executables.clone(),
        files: Arc::new(expected),
        installed_files: Arc::new(static_files),
        runtime_lease,
    })
}

impl VerifiedResources {
    pub(crate) fn runtime_lease(&self) -> Arc<RuntimeLease> {
        Arc::clone(&self.runtime_lease)
    }

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
        // New installations retain this product-created receipt until the
        // verified uninstall removes it. Older installed candidates already
        // deleted the receipt in NSIS, so its absence remains compatible.
        let receipt = root.join(INSTALL_RECEIPT);
        match fs::symlink_metadata(&receipt) {
            Ok(_) => {
                let payload = install_receipt_payload(&self.version, self.domain);
                let record = ByteRecord {
                    length: payload.len() as u64,
                    sha256: sha256(payload.as_bytes()),
                };
                file_sha256(&receipt, &record)?;
                add(receipt, record.sha256)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("GOGOKE_INSTALL_RECEIPT_UNREADABLE".to_string()),
        }
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
        self.runtime_lease.reverify()
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
    use std::process::Command;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn held_stage_verifies_and_publishes_generation() {
        let root = std::env::temp_dir().join(format!(
            "gogoke-stage-verify-test-{}", uuid::Uuid::new_v4().simple()
        ));
        let stage = root.join("stage");
        let published = root.join("published");
        fs::create_dir_all(stage.join("dist")).expect("owned stage directory");
        fs::write(stage.join("dist/bin.mjs"), b"signed service").expect("owned stage leaf");
        let expected = HashMap::from([(
            "dist/bin.mjs".to_string(),
            ByteRecord {
                length: 14,
                sha256: sha256(b"signed service"),
            },
        )]);
        let mut lease = RuntimeLease::default();
        lease.directories.insert(
            stage.clone(),
            open_owned_stage_directory(&stage).expect("hold exact stage object"),
        );
        verify_generation_in_lease(&stage, &expected, &mut lease)
            .expect("verify through the retained stage handle");
        let stage_handle = lease.directories.remove(&stage).expect("retained stage handle");
        drop(lease);
        rename_owned_stage_no_replace(&stage_handle, &published)
            .expect("publish verified stage without replacement");
        assert_eq!(
            fs::read(published.join("dist/bin.mjs")).expect("read published bytes"),
            b"signed service"
        );
        drop(stage_handle);
        let resolved_root = root.canonicalize().expect("owned fixture root");
        let resolved_temp = std::env::temp_dir().canonicalize().expect("test temp root");
        assert!(
            resolved_root.starts_with(&resolved_temp)
                && root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("gogoke-stage-verify-test-")),
            "recursive cleanup must stay inside the owned test fixture"
        );
        fs::remove_dir_all(&root).expect("remove owned test fixture");
    }

    #[test]
    fn stage_leaf_and_set_publication_never_replace_existing_objects() {
        let root = std::env::temp_dir().join(format!(
            "gogoke-stage-no-replace-test-{}", uuid::Uuid::new_v4().simple()
        ));
        fs::create_dir(&root).expect("owned test root");
        let source = root.join("source.bin");
        let target = root.join("target.bin");
        fs::write(&source, b"signed bytes").expect("owned source");
        fs::write(&target, b"external sentinel").expect("external target sentinel");
        assert_eq!(
            copy_stage_leaf_no_replace(&source, &target),
            Err("GOGOKE_RESOURCE_SET_STAGE_COLLISION".to_string())
        );
        assert_eq!(fs::read(&target).expect("read sentinel"), b"external sentinel");

        let linked_target = root.join("linked-target.bin");
        fs::hard_link(&target, &linked_target).expect("insert hard-link leaf");
        assert_eq!(
            copy_stage_leaf_no_replace(&source, &linked_target),
            Err("GOGOKE_RESOURCE_SET_STAGE_COLLISION".to_string())
        );
        assert_eq!(fs::read(&target).expect("read linked sentinel"), b"external sentinel");

        let stage = root.join("stage");
        let published = root.join("published");
        fs::create_dir(&stage).expect("owned stage");
        fs::create_dir(&published).expect("external empty directory");
        fs::write(stage.join("owned.bin"), b"owned").expect("owned stage leaf");
        let handle = open_owned_stage_directory(&stage).expect("open exact stage object");
        assert_eq!(
            rename_owned_stage_no_replace(&handle, &published),
            Err("GOGOKE_RESOURCE_SET_PUBLISH_FAILED".to_string())
        );
        assert!(stage.join("owned.bin").is_file(), "owned source remains after refusal");
        assert_eq!(fs::read_dir(&published).expect("read external directory").count(), 0);
        drop(handle);
        fs::remove_dir_all(&root).expect("remove settled owned test fixture");
    }

    #[test]
    fn runtime_lease_rejects_junction_and_holds_leaf_and_ancestor_until_drop() {
        let root = std::env::temp_dir().join(format!(
            "gogoke-resource-lease-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let physical = root.join("physical");
        let outside = root.join("outside");
        fs::create_dir_all(&physical).expect("physical test directory");
        fs::create_dir_all(&outside).expect("outside test directory");
        let leaf = physical.join("bin.mjs");
        fs::write(&leaf, b"signed service").expect("test service bytes");
        fs::write(outside.join("bin.mjs"), b"other service").expect("outside bytes");
        let record = ByteRecord {
            length: 14,
            sha256: sha256(b"signed service"),
        };

        let junction = root.join("junction");
        let status = Command::new("cmd")
            .args(["/D", "/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&outside)
            .status()
            .expect("junction command");
        assert!(status.success(), "junction fixture must be real");
        let mut rejected = RuntimeLease::default();
        assert_eq!(
            rejected.pin_file(&junction.join("bin.mjs"), &record, false),
            Err("GOGOKE_RESOURCE_REPARSE_POINT".to_string())
        );
        drop(rejected);
        assert!(read_verified_file(&junction.join("bin.mjs"), &record, true).is_err());

        let mut lease = RuntimeLease::default();
        lease
            .pin_file(&leaf, &record, false)
            .expect("verified lease");
        let lease = Arc::new(lease);
        let retained = Arc::clone(&lease);
        assert!(fs::write(&leaf, b"mutated service").is_err());
        assert!(fs::rename(&physical, root.join("swapped")).is_err());
        lease.reverify().expect("open leaf remains verified");
        drop(lease);
        assert!(fs::write(&leaf, b"mutated service").is_err());
        drop(retained);
        fs::write(&leaf, b"mutated service").expect("drop releases leaf");
        fs::rename(&physical, root.join("swapped")).expect("drop releases ancestor");
        assert!(read_verified_file(&root.join("swapped/bin.mjs"), &record, false).is_err());
        fs::remove_dir(&junction).expect("remove owned junction");
        let resolved_root = root.canonicalize().expect("owned fixture root");
        let resolved_temp = std::env::temp_dir().canonicalize().expect("test temp root");
        assert!(
            resolved_root.starts_with(&resolved_temp)
                && root
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("gogoke-resource-lease-test-")),
            "recursive cleanup must stay inside the owned test fixture"
        );
        fs::remove_dir_all(&root).expect("remove owned fixture");
    }

    #[test]
    fn frontend_request_decodes_one_url_layer_and_rejects_ambiguous_paths() {
        let request = |path: &str| {
            frontend_request_path(&format!("/{HASH}/{path}")).map(|(_, relative)| relative)
        };
        assert_eq!(request("assets/icon%20one.svg").unwrap(), "assets/icon one.svg");
        assert_eq!(request("assets/icon%23one.svg").unwrap(), "assets/icon#one.svg");
        assert_eq!(request("assets/icon%25one.svg").unwrap(), "assets/icon%one.svg");
        assert_eq!(request("assets/icon%252e.svg").unwrap(), "assets/icon%2e.svg");
        for path in [
            "assets/icon%2Fone.svg",
            "assets/icon%5cone.svg",
            "assets/%2e%2e/secret.js",
            "assets/icon%00.svg",
            "assets/icon%.svg",
            "assets/icon%GG.svg",
        ] {
            assert!(request(path).is_err(), "ambiguous request accepted: {path}");
        }
        assert!(frontend_request_path("/assets/index.js").is_err());
        assert!(frontend_request_path("/not-a-set/index.html").is_err());
    }

    #[test]
    fn frontend_requests_remain_bound_to_their_verified_generation() {
        let root = std::env::temp_dir().join(format!(
            "gogoke-generation-route-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let old_id = HASH.to_string();
        let new_id = "a".repeat(64);
        let resources = |set_id: &str, payload: &[u8]| {
            let generation_root = root.join(set_id);
            fs::create_dir_all(generation_root.join("frontend")).expect("test generation");
            fs::write(generation_root.join("frontend/index.html"), payload)
                .expect("test frontend bytes");
            let record = ByteRecord {
                length: payload.len() as u64,
                sha256: sha256(payload),
            };
            let executable = ByteRecord {
                length: 0,
                sha256: HASH.to_string(),
            };
            VerifiedResources {
                domain: Domain::Formal,
                version: "1.2.3".to_string(),
                source_commit: "0".repeat(40),
                generation_id: set_id.to_string(),
                set_id: set_id.to_string(),
                generation_root,
                service_root: root.clone(),
                install_root: root.clone(),
                native_host_path: root.join("native-host.exe"),
                node_runtime_path: root.join("node.exe"),
                executables: Executables {
                    portable_shell: executable.clone(),
                    installed_shell: executable.clone(),
                    native_host: executable.clone(),
                    node: executable,
                },
                files: Arc::new(HashMap::from([("frontend/index.html".to_string(), record)])),
                installed_files: Arc::new(HashMap::new()),
                runtime_lease: Arc::new(RuntimeLease::default()),
            }
        };
        let old = resources(&old_id, b"old generation");
        let new = resources(&new_id, b"new generation");
        let state = ResourceState::new(old.clone());
        state.replace(new).expect("switch pending generation");
        assert_eq!(
            state
                .read_frontend_request(&format!("/{old_id}/index.html"))
                .unwrap()
                .0,
            b"old generation".to_vec()
        );
        assert_eq!(
            state
                .read_frontend_request(&format!("/{new_id}/index.html"))
                .unwrap()
                .0,
            b"new generation".to_vec()
        );
        assert!(state.read_frontend_request("/index.html").is_err());
        assert!(state
            .read_frontend_request(&format!("/{}/index.html", "b".repeat(64)))
            .is_err());
        state.replace(old).expect("roll back pending generation");
        assert_eq!(state.current().unwrap().set_id, old_id);
        assert_eq!(
            state
                .read_frontend_request(&format!("/{new_id}/index.html"))
                .unwrap()
                .0,
            b"new generation".to_vec()
        );
        fs::remove_dir_all(&root).expect("remove owned test generation");
    }

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
