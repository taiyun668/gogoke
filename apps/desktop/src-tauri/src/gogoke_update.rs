use futures_util::StreamExt;
use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
use reqwest::{Client, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager, WebviewWindow};
use uuid::Uuid;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

const FEED_URL: &str = "https://api.github.com/repos/taiyun668/gogoke/releases/latest";
const ALLOWED_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
const CHECKSUMS_NAME: &str = "SHA256SUMS.windows";
const SIGNATURE_NAME: &str = "SHA256SUMS.windows.sig";
const MAX_FEED_BYTES: usize = 1 << 20;
const MAX_INSTALLER_BYTES: usize = 250 << 20;
const MAX_RESOURCE_BYTES: usize = 500 << 20;
const RELEASE_PUBLIC_KEY: &str = include_str!("../gogoke-release-public-key.txt");
const UPDATE_COORDINATOR: &str = include_str!("../update/gogoke-update-coordinator.ps1");
const UPDATE_READY_PREFIX: &str = "--gogoke-update-ready=";
const SHELL_USER_AGENT: &str = "gogoke-shell/1";

static UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(target_os = "windows")]
static RESOURCE_READY: tokio::sync::Mutex<Option<ResourceReadyWaiter>> =
    tokio::sync::Mutex::const_new(None);

#[cfg(target_os = "windows")]
struct ResourceReadyWaiter {
    set_id: String,
    version: String,
    generation_id: String,
    sender: tokio::sync::oneshot::Sender<()>,
}

const RESOURCE_SET_QUERY: &str = "gogoke-resource-set";

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    published_at: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GogokeUpdateOffer {
    version: String,
    release_type: String,
    asset: String,
    sha256: String,
    published_at: String,
    notes_url: String,
    notes: String,
}

#[derive(Clone, Debug)]
struct ReleaseIdentity {
    offer: GogokeUpdateOffer,
    assets: Vec<VerifiedAsset>,
    manifest: Vec<u8>,
    signature: Vec<u8>,
}

#[derive(Clone, Debug)]
struct VerifiedAsset {
    name: String,
    url: String,
    sha256: String,
}

struct FormalManifest {
    version: String,
    release_type: String,
    hashes: HashMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PreparedUpdateState {
    schema: u8,
    status: String,
    offer: GogokeUpdateOffer,
    prepared_at: String,
    last_error: Option<String>,
}

fn installer_name(version: &str) -> String {
    format!("gogoke-{version}-windows-x64-unsigned-setup.exe")
}

fn portable_name(version: &str) -> String {
    format!("gogoke-{version}-windows-x64-unsigned-portable.zip")
}

fn parse_formal_manifest(bytes: &[u8]) -> Result<FormalManifest, String> {
    if bytes.contains(&b'\r') || !bytes.ends_with(b"\n") {
        return Err("release manifest must be canonical LF text".to_string());
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| "release manifest is not UTF-8".to_string())?;
    let mut headers = HashMap::new();
    let mut hashes = HashMap::new();
    for line in text.lines() {
        if let Some(header) = line.strip_prefix("# gogoke-") {
            let (name, value) = header
                .split_once(": ")
                .ok_or("release manifest header is malformed")?;
            if value.is_empty() || headers.insert(name, value).is_some() {
                return Err("release manifest has duplicate or empty header".to_string());
            }
        } else {
            let (hash, name) = line
                .split_once("  ")
                .ok_or("release manifest checksum line is malformed")?;
            if hash.len() != 64
                || !hash
                    .bytes()
                    .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
                || name.is_empty()
                || name.contains(' ')
                || hashes.insert(name.to_string(), hash.to_string()).is_some()
            {
                return Err("release manifest checksum is invalid or duplicate".to_string());
            }
        }
    }
    if headers.len() != 2
        || !headers.contains_key("Version")
        || !headers.contains_key("Release-Type")
    {
        return Err("release manifest headers are not exact".to_string());
    }
    let version = headers["Version"].to_string();
    Version::parse(&version).map_err(|_| "release manifest version is not SemVer".to_string())?;
    let release_type = headers["Release-Type"].to_string();
    let mut required = vec![
        "resource-index.json".to_string(),
        "gogoke-resources.windows.zip".to_string(),
    ];
    match release_type.as_str() {
        "full" => {
            required.push(installer_name(&version));
            required.push(portable_name(&version));
        }
        "resources" => {}
        _ => return Err("release manifest type is invalid".to_string()),
    }
    if hashes.len() != required.len() || required.iter().any(|name| !hashes.contains_key(name)) {
        return Err("release manifest asset set is not exact".to_string());
    }
    Ok(FormalManifest {
        version,
        release_type,
        hashes,
    })
}

fn validate_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|error| format!("invalid release URL: {error}"))?;
    if url.scheme() != "https" || !ALLOWED_HOSTS.contains(&url.host_str().unwrap_or_default()) {
        return Err("release URL is outside the gogoke publication host set".to_string());
    }
    Ok(url)
}

async fn get_bytes(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let url = validate_url(url)?;
    let response = client
        .get(url)
        .header("User-Agent", SHELL_USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("release request failed: {error}"))?;
    validate_url(response.url().as_str())?;
    if !response.status().is_success() {
        return Err(format!(
            "release request returned HTTP {}",
            response.status()
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("release response is larger than expected".to_string());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("could not read release response: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("release response is larger than expected".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn get_release_feed(client: &Client) -> Result<Option<Vec<u8>>, String> {
    let url = validate_url(FEED_URL)?;
    let response = client
        .get(url)
        .header("User-Agent", SHELL_USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|error| format!("release request failed: {error}"))?;
    validate_url(response.url().as_str())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(format!(
            "release request returned HTTP {}",
            response.status()
        ));
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| format!("could not read release response: {error}"))?;
        if bytes.len().saturating_add(chunk.len()) > MAX_FEED_BYTES {
            return Err("release response is larger than expected".to_string());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(bytes))
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim();
    if value.len() % 2 != 0 {
        return Err("release signature has invalid hex length".to_string());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16)
                .map_err(|_| "release signature is not hexadecimal".to_string())
        })
        .collect()
}

fn verify_manifest(manifest: &[u8], signature_hex: &[u8]) -> Result<(), String> {
    let public = decode_hex(RELEASE_PUBLIC_KEY)?;
    if public.len() != 64 {
        return Err("gogoke release public key is invalid".to_string());
    }
    let mut sec1 = Vec::with_capacity(65);
    sec1.push(4);
    sec1.extend_from_slice(&public);
    let key = VerifyingKey::from_sec1_bytes(&sec1)
        .map_err(|_| "gogoke release public key is invalid".to_string())?;
    let signature_text = std::str::from_utf8(signature_hex)
        .map_err(|_| "release signature is not ASCII".to_string())?;
    let signature = Signature::from_slice(&decode_hex(signature_text)?)
        .map_err(|_| "release signature has invalid shape".to_string())?;
    key.verify(manifest, &signature)
        .map_err(|_| "release manifest signature is invalid".to_string())
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("could not read staged update: {error}"))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn update_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("could not resolve gogoke update cache: {error}"))?
        .join("update-cache");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create gogoke update cache: {error}"))?;
    Ok(directory)
}

fn staged_release_directory(
    app: &AppHandle,
    identity: &ReleaseIdentity,
) -> Result<PathBuf, String> {
    let digest = format!("{:x}", Sha256::digest(&identity.manifest));
    let directory = update_directory(app)?.join("release-set").join(format!(
        "{}-{}",
        identity.offer.version,
        &digest[..16]
    ));
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not stage gogoke release set: {error}"))?;
    Ok(directory)
}

fn staged_assets_match(directory: &Path, identity: &ReleaseIdentity) -> Result<bool, String> {
    let mut expected: HashSet<&str> = identity
        .assets
        .iter()
        .map(|asset| asset.name.as_str())
        .collect();
    expected.insert(CHECKSUMS_NAME);
    expected.insert(SIGNATURE_NAME);
    let actual: HashSet<String> = std::fs::read_dir(directory)
        .map_err(|error| format!("could not inspect staged release set: {error}"))?
        .map(|entry| {
            let entry =
                entry.map_err(|error| format!("could not inspect staged release set: {error}"))?;
            if !entry
                .file_type()
                .map_err(|error| format!("could not inspect staged release set: {error}"))?
                .is_file()
            {
                return Err("staged release set contains a non-file entry".to_string());
            }
            entry
                .file_name()
                .into_string()
                .map_err(|_| "staged release set contains a non-UTF-8 name".to_string())
        })
        .collect::<Result<_, _>>()?;
    if actual.len() != expected.len() || actual.iter().any(|name| !expected.contains(name.as_str()))
    {
        return Ok(false);
    }
    for asset in &identity.assets {
        let path = directory.join(&asset.name);
        if !path.is_file() || sha256(&path)? != asset.sha256 {
            return Ok(false);
        }
    }
    Ok(std::fs::read(directory.join(CHECKSUMS_NAME))
        .ok()
        .as_deref()
        == Some(identity.manifest.as_slice())
        && std::fs::read(directory.join(SIGNATURE_NAME))
            .ok()
            .as_deref()
            == Some(identity.signature.as_slice()))
}

fn update_state_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(update_directory(app)?.join("update-state.json"))
}

fn consume_update_failure(app: &AppHandle) -> Result<Option<String>, String> {
    let directory = update_directory(app)?;
    let failure = directory.join("update-failure.log");
    if !failure.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(&failure)
        .map_err(|error| format!("could not read the previous update failure: {error}"))?;
    let bounded = &bytes[..bytes.len().min(8 * 1024)];
    let message = String::from_utf8_lossy(bounded).trim().to_string();
    let reported = directory.join("update-failure.reported.log");
    if reported.exists() {
        std::fs::remove_file(&reported)
            .map_err(|error| format!("could not rotate the update failure log: {error}"))?;
    }
    std::fs::rename(failure, reported)
        .map_err(|error| format!("could not preserve the update failure log: {error}"))?;
    let state_path = update_state_path(app)?;
    if let Ok(payload) = std::fs::read(&state_path) {
        if let Ok(mut state) = serde_json::from_slice::<PreparedUpdateState>(&payload) {
            state.status = "failed".to_string();
            state.last_error = Some(message.clone());
            let _ = write_update_state(app, &state);
        }
    }
    Ok(Some(message))
}

fn write_update_state(app: &AppHandle, state: &PreparedUpdateState) -> Result<(), String> {
    let path = update_state_path(app)?;
    let partial = path.with_extension("part");
    let payload = serde_json::to_vec_pretty(state)
        .map_err(|error| format!("could not serialize gogoke update state: {error}"))?;
    std::fs::write(&partial, payload)
        .map_err(|error| format!("could not write gogoke update state: {error}"))?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("could not replace gogoke update state: {error}"))?;
    }
    std::fs::rename(partial, path)
        .map_err(|error| format!("could not publish gogoke update state: {error}"))
}

fn read_update_state(app: &AppHandle) -> Result<PreparedUpdateState, String> {
    let path = update_state_path(app)?;
    let payload = std::fs::read(&path)
        .map_err(|error| format!("could not read prepared gogoke update state: {error}"))?;
    let state: PreparedUpdateState = serde_json::from_slice(&payload)
        .map_err(|error| format!("prepared gogoke update state is invalid: {error}"))?;
    if state.schema != 1 || state.status != "prepared" {
        return Err("gogoke update is not in the prepared state".to_string());
    }
    Ok(state)
}

fn installed_target(app: &AppHandle, current_exe: &Path) -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        use winreg::{enums::HKEY_CURRENT_USER, RegKey};
        let _ = app;
        let current_dir = current_exe
            .parent()
            .ok_or_else(|| "current gogoke executable has no parent directory".to_string())?;
        let registry = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\gogoke")
            .map_err(|_| "GOGOKE_UPDATE_INSTALL_REGISTRATION_MISSING".to_string())?;
        let registered: String = registry
            .get_value("InstallLocation")
            .map_err(|_| "GOGOKE_UPDATE_INSTALL_REGISTRATION_INVALID".to_string())?;
        let domain: String = registry
            .get_value("InstallDomain")
            .map_err(|_| "GOGOKE_UPDATE_INSTALL_REGISTRATION_INVALID".to_string())?;
        let instance: String = registry
            .get_value("InstallInstanceId")
            .map_err(|_| "GOGOKE_UPDATE_INSTALL_REGISTRATION_INVALID".to_string())?;
        let registered_path = Path::new(&registered)
            .canonicalize()
            .map_err(|_| "GOGOKE_UPDATE_INSTALL_REGISTRATION_INVALID".to_string())?;
        let current_path = current_dir
            .canonicalize()
            .map_err(|_| "GOGOKE_UPDATE_INSTALL_REGISTRATION_INVALID".to_string())?;
        if domain != "OWNER_RELEASE" || instance.len() < 16 || registered_path != current_path {
            return Err("GOGOKE_UPDATE_INSTALL_REGISTRATION_INVALID".to_string());
        }
        return Ok(current_dir.to_path_buf());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, current_exe);
        Err("gogoke automatic updates are currently available on Windows only".to_string())
    }
}

#[cfg(target_os = "windows")]
fn staged_index_generation(release_set: &Path, expected_version: &str) -> Result<String, String> {
    crate::resource_trust::verify_install_set(release_set)?;
    let index = std::fs::read(release_set.join("resource-index.json"))
        .map_err(|_| "GOGOKE_UPDATE_INDEX_UNAVAILABLE".to_string())?;
    let index: serde_json::Value =
        serde_json::from_slice(&index).map_err(|_| "GOGOKE_UPDATE_INDEX_INVALID".to_string())?;
    if index.get("version").and_then(|value| value.as_str()) != Some(expected_version) {
        return Err("GOGOKE_UPDATE_INDEX_VERSION_MISMATCH".to_string());
    }
    let generation = index
        .get("generationId")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "GOGOKE_UPDATE_INDEX_GENERATION_MISSING".to_string())?;
    if generation.len() != 64
        || !generation
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    {
        return Err("GOGOKE_UPDATE_INDEX_GENERATION_INVALID".to_string());
    }
    Ok(generation.to_string())
}

fn ready_path_from_args() -> Result<Option<PathBuf>, String> {
    let Some(value) = std::env::args().find_map(|argument| {
        argument
            .strip_prefix(UPDATE_READY_PREFIX)
            .map(ToOwned::to_owned)
    }) else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    let expected_parent = std::env::temp_dir();
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if path.parent() != Some(expected_parent.as_path())
        || !file_name.starts_with("gogoke-update-")
        || !file_name.ends_with(".ready")
    {
        return Err("update readiness path is outside the gogoke temporary namespace".to_string());
    }
    Ok(Some(path))
}

fn publish_update_ready(path: &Path, contents: &[u8]) -> Result<(), String> {
    publish_update_ready_with_hook(path, contents, |_final_path, _partial_path| Ok(()))
}

fn publish_update_ready_with_hook<F>(
    path: &Path,
    contents: &[u8],
    before_publish: F,
) -> Result<(), String>
where
    F: FnOnce(&Path, &Path) -> Result<(), String>,
{
    let parent = path
        .parent()
        .ok_or_else(|| "update readiness path has no parent directory".to_string())?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "update readiness path has no valid file name".to_string())?;
    if path.exists() {
        return Err("gogoke update readiness receipt already exists".to_string());
    }
    let partial = parent.join(format!(".{file_name}.{}.part", Uuid::new_v4().simple()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| {
                format!("could not create gogoke update readiness receipt: {error}")
            })?;
        file.write_all(contents)
            .and_then(|_| file.sync_all())
            .map_err(|error| {
                format!("could not persist gogoke update readiness receipt: {error}")
            })?;
        drop(file);
        before_publish(path, &partial)?;
        std::fs::rename(&partial, path).map_err(|error| {
            format!("could not publish gogoke update readiness receipt: {error}")
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    result
}

pub async fn signal_update_ready(app: &AppHandle) -> Result<(), String> {
    let Some(path) = ready_path_from_args()? else {
        return Ok(());
    };
    crate::public_runtime::product_entry::verify_product_startup(app).await?;
    #[cfg(target_os = "windows")]
    let ready = {
        let resources = app
            .try_state::<crate::resource_trust::ResourceState>()
            .ok_or_else(|| "GOGOKE_UPDATE_VERIFIED_IDENTITY_UNAVAILABLE".to_string())?;
        let resources = resources.current()?;
        serde_json::to_vec(&serde_json::json!({
            "version": resources.version.as_str(),
            "generationId": resources.generation_id.as_str(),
            "setId": resources.set_id.as_str(),
        }))
        .map_err(|_| "GOGOKE_UPDATE_READY_ENCODE_FAILED".to_string())?
    };
    #[cfg(not(target_os = "windows"))]
    let ready = b"UNSUPPORTED_PLATFORM".to_vec();
    publish_update_ready(&path, &ready)?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn resource_page_url(window: &WebviewWindow, set_id: &str) -> Result<Url, String> {
    let mut url = window
        .url()
        .map_err(|_| "GOGOKE_UPDATE_WEBVIEW_URL_UNAVAILABLE".to_string())?;
    let trusted_origin = (url.scheme() == "gogoke-resource" && url.host_str() == Some("localhost"))
        || ((url.scheme() == "http" || url.scheme() == "https")
            && url.host_str() == Some("gogoke-resource.localhost"));
    if !trusted_origin || url.path() != "/index.html" {
        return Err("GOGOKE_UPDATE_WEBVIEW_ORIGIN_INVALID".to_string());
    }
    url.set_query(Some(&format!("{RESOURCE_SET_QUERY}={set_id}")));
    Ok(url)
}

#[cfg(target_os = "windows")]
async fn signal_resource_ready(
    app: &AppHandle,
    window: &WebviewWindow,
    set_id: Option<&str>,
) -> Result<(), String> {
    let mut pending = RESOURCE_READY.lock().await;
    let Some(waiter) = pending.as_ref() else {
        return Ok(());
    };
    if window.label() != "main" || set_id != Some(waiter.set_id.as_str()) {
        return Err("GOGOKE_UPDATE_RESOURCE_PAGE_IDENTITY_MISMATCH".to_string());
    }
    let actual_url = window
        .url()
        .map_err(|_| "GOGOKE_UPDATE_WEBVIEW_URL_UNAVAILABLE".to_string())?;
    let expected_url = resource_page_url(window, &waiter.set_id)?;
    if actual_url != expected_url {
        return Err("GOGOKE_UPDATE_RESOURCE_PAGE_IDENTITY_MISMATCH".to_string());
    }
    let resources = app
        .try_state::<crate::resource_trust::ResourceState>()
        .ok_or_else(|| "GOGOKE_UPDATE_VERIFIED_IDENTITY_UNAVAILABLE".to_string())?
        .current()?;
    if resources.domain != crate::resource_trust::Domain::Formal
        || resources.set_id != waiter.set_id
        || resources.version != waiter.version
        || resources.generation_id != waiter.generation_id
    {
        return Err("GOGOKE_UPDATE_RESOURCE_IDENTITY_MISMATCH".to_string());
    }
    resources.verify_runtime_files()?;
    crate::public_runtime::product_entry::verify_product_startup(app).await?;
    let waiter = pending
        .take()
        .ok_or_else(|| "GOGOKE_UPDATE_RESOURCE_RECEIPT_LOST".to_string())?;
    waiter
        .sender
        .send(())
        .map_err(|_| "GOGOKE_UPDATE_RESOURCE_RECEIPT_LOST".to_string())
}

#[tauri::command]
pub async fn gogoke_update_signal_ready(
    app: AppHandle,
    window: WebviewWindow,
    resource_set_id: Option<String>,
) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    signal_resource_ready(&app, &window, resource_set_id.as_deref()).await?;
    #[cfg(not(target_os = "windows"))]
    let _ = (window, resource_set_id);
    signal_update_ready(&app).await
}

#[tauri::command]
pub fn gogoke_update_take_failure(app: AppHandle) -> Result<Option<String>, String> {
    consume_update_failure(&app)
}

async fn release_identity(
    client: &Client,
    current_version: &str,
) -> Result<Option<ReleaseIdentity>, String> {
    let Some(feed) = get_release_feed(client).await? else {
        return Ok(None);
    };
    let release: GithubRelease = serde_json::from_slice(&feed)
        .map_err(|error| format!("invalid gogoke release feed: {error}"))?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version_text = release
        .tag_name
        .trim_start_matches(|character| character == 'v' || character == 'V');
    let candidate =
        Version::parse(version_text).map_err(|_| "release tag is not SemVer".to_string())?;
    let current = Version::parse(current_version)
        .map_err(|_| "current app version is not SemVer".to_string())?;
    if candidate <= current {
        return Ok(None);
    }
    let find_asset = |name: &str| {
        let matches: Vec<_> = release
            .assets
            .iter()
            .filter(|asset| asset.name == name)
            .collect();
        if matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        }
    };
    let Some(checksums) = find_asset(CHECKSUMS_NAME) else {
        return Ok(None);
    };
    let Some(signature) = find_asset(SIGNATURE_NAME) else {
        return Ok(None);
    };
    let manifest_bytes = get_bytes(client, &checksums.browser_download_url, MAX_FEED_BYTES).await?;
    let signature_bytes =
        get_bytes(client, &signature.browser_download_url, MAX_FEED_BYTES).await?;
    verify_manifest(&manifest_bytes, &signature_bytes)?;
    let manifest = parse_formal_manifest(&manifest_bytes)?;
    if manifest.version != version_text {
        return Err("signed manifest version does not match release tag".to_string());
    }
    if release.assets.len() != manifest.hashes.len() + 2 {
        return Err("release asset set differs from signed manifest".to_string());
    }
    let mut assets = Vec::with_capacity(manifest.hashes.len());
    for (name, sha256) in &manifest.hashes {
        let asset = find_asset(name).ok_or("signed release asset is missing or duplicate")?;
        validate_url(&asset.browser_download_url)?;
        assets.push(VerifiedAsset {
            name: name.clone(),
            url: asset.browser_download_url.clone(),
            sha256: sha256.clone(),
        });
    }
    assets.sort_by(|left, right| left.name.cmp(&right.name));
    let asset_name = if manifest.release_type == "full" {
        installer_name(version_text)
    } else {
        "gogoke-resources.windows.zip".to_string()
    };
    let digest = manifest.hashes[&asset_name].clone();
    validate_url(&release.html_url)?;
    Ok(Some(ReleaseIdentity {
        offer: GogokeUpdateOffer {
            version: version_text.to_string(),
            release_type: manifest.release_type,
            asset: asset_name,
            sha256: digest,
            published_at: release.published_at,
            notes_url: release.html_url,
            notes: release.body,
        },
        assets,
        manifest: manifest_bytes,
        signature: signature_bytes,
    }))
}

fn client() -> Result<Client, String> {
    Client::builder()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                return attempt.stop();
            }
            let url = attempt.url();
            if url.scheme() == "https"
                && ALLOWED_HOSTS.contains(&url.host_str().unwrap_or_default())
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|error| format!("could not initialize update client: {error}"))
}

fn current_version(app: &AppHandle) -> Result<String, String> {
    #[cfg(target_os = "windows")]
    if let Some(resources) = app.try_state::<crate::resource_trust::ResourceState>() {
        return Ok(resources.current()?.version);
    }
    #[cfg(debug_assertions)]
    {
        return Ok(env!("CARGO_PKG_VERSION").to_string());
    }
    #[allow(unreachable_code)]
    Err("GOGOKE_UPDATE_VERIFIED_IDENTITY_UNAVAILABLE".to_string())
}

#[tauri::command]
pub fn gogoke_product_version(app: AppHandle) -> Result<String, String> {
    #[cfg(not(target_os = "windows"))]
    {
        return Ok(app.package_info().version.to_string());
    }
    #[cfg(target_os = "windows")]
    current_version(&app)
}

#[tauri::command]
pub async fn gogoke_update_check(app: AppHandle) -> Result<Option<GogokeUpdateOffer>, String> {
    if !cfg!(target_os = "windows") {
        return Ok(None);
    }
    let _guard = UPDATE_LOCK.lock().await;
    #[cfg(target_os = "windows")]
    require_formal_domain(&app)?;
    let client = client()?;
    let Some(identity) = release_identity(&client, &current_version(&app)?).await? else {
        return Ok(None);
    };
    let path = staged_release_directory(&app, &identity)?;
    if let Ok(prepared) = read_update_state(&app) {
        if prepared.offer.version == identity.offer.version
            && prepared.offer.release_type == identity.offer.release_type
            && prepared.offer.asset == identity.offer.asset
            && prepared.offer.sha256 == identity.offer.sha256
            && staged_assets_match(&path, &identity)?
        {
            return Ok(Some(identity.offer));
        }
    }
    for asset in &identity.assets {
        let limit = if asset.name == identity.offer.asset && identity.offer.release_type == "full" {
            MAX_INSTALLER_BYTES
        } else if asset.name == "resource-index.json" {
            4 << 20
        } else {
            MAX_RESOURCE_BYTES
        };
        let bytes = get_bytes(&client, &asset.url, limit).await?;
        if format!("{:x}", Sha256::digest(&bytes)) != asset.sha256 {
            return Err("downloaded release asset does not match Owner manifest".to_string());
        }
        let destination = path.join(&asset.name);
        let partial = destination.with_extension("part");
        std::fs::write(&partial, bytes)
            .map_err(|error| format!("could not stage gogoke update: {error}"))?;
        if destination.exists() {
            std::fs::remove_file(&destination)
                .map_err(|error| format!("could not replace staged gogoke asset: {error}"))?;
        }
        std::fs::rename(&partial, &destination)
            .map_err(|error| format!("could not publish staged gogoke asset: {error}"))?;
    }
    std::fs::write(path.join(CHECKSUMS_NAME), &identity.manifest)
        .map_err(|error| format!("could not stage Owner manifest: {error}"))?;
    std::fs::write(path.join(SIGNATURE_NAME), &identity.signature)
        .map_err(|error| format!("could not stage Owner signature: {error}"))?;
    if !staged_assets_match(&path, &identity)? {
        return Err("staged release set is not the exact Owner signed asset set".to_string());
    }
    let offer = identity.offer;
    write_update_state(
        &app,
        &PreparedUpdateState {
            schema: 1,
            status: "prepared".to_string(),
            offer: offer.clone(),
            prepared_at: chrono::Utc::now().to_rfc3339(),
            last_error: None,
        },
    )?;
    Ok(Some(offer))
}

#[cfg(target_os = "windows")]
fn require_formal_domain(
    app: &AppHandle,
) -> Result<crate::resource_trust::VerifiedResources, String> {
    let current = app
        .try_state::<crate::resource_trust::ResourceState>()
        .ok_or_else(|| "GOGOKE_UPDATE_VERIFIED_IDENTITY_UNAVAILABLE".to_string())?
        .current()?;
    if current.domain != crate::resource_trust::Domain::Formal {
        return Err("GOGOKE_UPDATE_FORMAL_DOMAIN_REQUIRED".to_string());
    }
    Ok(current)
}

#[cfg(target_os = "windows")]
async fn apply_resource_update(
    app: &AppHandle,
    identity: &ReleaseIdentity,
    release_set: &Path,
) -> Result<(), String> {
    let state = app
        .try_state::<crate::resource_trust::ResourceState>()
        .ok_or_else(|| "GOGOKE_UPDATE_VERIFIED_IDENTITY_UNAVAILABLE".to_string())?;
    let old = require_formal_domain(app)?;
    let _lifecycle_guard = crate::resource_trust::acquire_lifecycle_lock(&old.install_root)?;
    let new = crate::resource_trust::stage_resource_update(release_set, &old)?;
    if new.domain != crate::resource_trust::Domain::Formal
        || new.version != identity.offer.version
        || new.set_id == old.set_id
    {
        return Err("GOGOKE_UPDATE_RESOURCE_SET_IDENTITY_MISMATCH".to_string());
    }

    // The product gate covers the entire live switch. A committed task that
    // already entered must finish before this guard is acquired.
    let _product_guard = crate::public_runtime::product_entry::acquire_product_gate().await;
    if state.current()?.set_id != old.set_id {
        return Err("GOGOKE_UPDATE_ACTIVE_SET_CHANGED".to_string());
    }
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "GOGOKE_UPDATE_MAIN_WEBVIEW_UNAVAILABLE".to_string())?;
    let new_url = resource_page_url(&window, &new.set_id)?;
    let old_url = resource_page_url(&window, &old.set_id)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    {
        let mut pending = RESOURCE_READY.lock().await;
        if pending.is_some() {
            return Err("GOGOKE_UPDATE_RESOURCE_SWITCH_ALREADY_PENDING".to_string());
        }
        *pending = Some(ResourceReadyWaiter {
            set_id: new.set_id.clone(),
            version: new.version.clone(),
            generation_id: new.generation_id.clone(),
            sender,
        });
    }
    let applying = PreparedUpdateState {
        schema: 1,
        status: "applying".to_string(),
        offer: identity.offer.clone(),
        prepared_at: chrono::Utc::now().to_rfc3339(),
        last_error: None,
    };
    let outcome = async {
        write_update_state(app, &applying)?;
        state.replace(new.clone())?;
        window
            .navigate(new_url)
            .map_err(|error| format!("could not load new resource generation: {error}"))?;
        tokio::time::timeout(std::time::Duration::from_secs(30), receiver)
            .await
            .map_err(|_| "new resource generation did not report readiness".to_string())?
            .map_err(|_| "new resource generation readiness was cancelled".to_string())?;
        crate::resource_trust::activate_resource_set(&new.install_root, &new.set_id)?;
        let installed = PreparedUpdateState {
            status: "installed".to_string(),
            ..applying.clone()
        };
        // The pointer is committed. A cache-state write cannot undo that fact.
        let _ = write_update_state(app, &installed);
        Ok::<(), String>(())
    }
    .await;
    RESOURCE_READY.lock().await.take();
    if let Err(error) = outcome {
        let state_restored = state.replace(old);
        let page_restored = window.navigate(old_url);
        let error = if state_restored.is_err() || page_restored.is_err() {
            format!("{error}; GOGOKE_UPDATE_RESOURCE_ROLLBACK_INCOMPLETE")
        } else {
            error
        };
        let failed = PreparedUpdateState {
            status: "failed".to_string(),
            last_error: Some(error.clone()),
            ..applying
        };
        let _ = write_update_state(app, &failed);
        let _ = std::fs::write(update_directory(app)?.join("update-failure.log"), &error);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub async fn gogoke_update_install(app: AppHandle, version: String) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("gogoke automatic updates are currently available on Windows only".to_string());
    }
    let _guard = UPDATE_LOCK.lock().await;
    #[cfg(target_os = "windows")]
    require_formal_domain(&app)?;
    let prepared = read_update_state(&app)?;
    if prepared.offer.version != version {
        return Err("the requested gogoke update is not the prepared version".to_string());
    }
    let client = client()?;
    let identity = release_identity(&client, &current_version(&app)?)
        .await?
        .ok_or_else(|| "the prepared gogoke update is no longer available".to_string())?;
    if identity.offer.version != version
        || identity.offer.release_type != prepared.offer.release_type
        || identity.offer.sha256 != prepared.offer.sha256
    {
        return Err("the prepared gogoke update identity is stale".to_string());
    }
    let release_set = staged_release_directory(&app, &identity)?;
    if !staged_assets_match(&release_set, &identity)? {
        return Err("the prepared release set no longer matches its Owner manifest".to_string());
    }
    #[cfg(target_os = "windows")]
    if identity.offer.release_type == "resources" {
        return apply_resource_update(&app, &identity, &release_set).await;
    }
    let installer = release_set.join(&identity.offer.asset);
    #[cfg(target_os = "windows")]
    let expected_generation = staged_index_generation(&release_set, &version)?;
    #[cfg(not(target_os = "windows"))]
    let expected_generation = String::new();

    let update_dir = update_directory(&app)?;
    let coordinator = update_dir.join("gogoke-update-coordinator.ps1");
    std::fs::write(&coordinator, UPDATE_COORDINATOR.as_bytes())
        .map_err(|error| format!("could not stage the gogoke update coordinator: {error}"))?;
    let current_exe = std::env::current_exe()
        .map_err(|error| format!("could not resolve the current gogoke executable: {error}"))?;
    let target_dir = installed_target(&app, &current_exe)?;
    let token = Uuid::new_v4().simple().to_string();
    let ready_file = std::env::temp_dir().join(format!("gogoke-update-{token}.ready"));
    let failure_log = update_dir.join("update-failure.log");
    let retention_notice = update_dir.join("update-retained-backup.log");
    let state_file = update_state_path(&app)?;
    let lock_file = update_dir.join("update-apply.lock");
    if ready_file.exists() {
        std::fs::remove_file(&ready_file)
            .map_err(|error| format!("could not clear a stale update receipt: {error}"))?;
    }
    let powershell = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut command = Command::new(powershell);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-WindowStyle",
        "Hidden",
        "-File",
    ]);
    command
        .arg(&coordinator)
        .arg("-Installer")
        .arg(&installer)
        .arg("-ReleaseSetDirectory")
        .arg(&release_set)
        .arg("-ParentPid")
        .arg(std::process::id().to_string())
        .arg("-CurrentExe")
        .arg(&current_exe)
        .arg("-TargetDir")
        .arg(&target_dir)
        .arg("-ReadyFile")
        .arg(&ready_file)
        .arg("-ExpectedVersion")
        .arg(&version)
        .arg("-ExpectedGenerationId")
        .arg(&expected_generation)
        .arg("-ExpectedSha256")
        .arg(&prepared.offer.sha256)
        .arg("-ExpectedManifestSha256")
        .arg(format!("{:x}", Sha256::digest(&identity.manifest)))
        .arg("-ExpectedSignatureSha256")
        .arg(format!("{:x}", Sha256::digest(&identity.signature)))
        .arg("-LockFile")
        .arg(&lock_file)
        .arg("-FailureLog")
        .arg(&failure_log)
        .arg("-StateFile")
        .arg(&state_file)
        .arg("-RetentionNoticeFile")
        .arg(&retention_notice);
    #[cfg(target_os = "windows")]
    command.creation_flags(0x0800_0000);
    let applying = PreparedUpdateState {
        schema: 1,
        status: "applying".to_string(),
        offer: identity.offer,
        prepared_at: prepared.prepared_at,
        last_error: None,
    };
    write_update_state(&app, &applying)?;
    if let Err(error) = command.spawn() {
        let failed = PreparedUpdateState {
            status: "failed".to_string(),
            last_error: Some(error.to_string()),
            ..applying
        };
        let _ = write_update_state(&app, &failed);
        return Err(format!(
            "could not start the gogoke update coordinator: {error}"
        ));
    }
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formal_manifest_requires_exact_release_shape() {
        let digest = "0123456789abcdef".repeat(4);
        let resources = format!(
            "# gogoke-Version: 1.2.3\n# gogoke-Release-Type: resources\n{digest}  resource-index.json\n{digest}  gogoke-resources.windows.zip\n"
        );
        assert_eq!(
            parse_formal_manifest(resources.as_bytes())
                .expect("resources release")
                .release_type,
            "resources"
        );
        let full = format!(
            "# gogoke-Version: 1.2.3\n# gogoke-Release-Type: full\n{digest}  resource-index.json\n{digest}  gogoke-resources.windows.zip\n{digest}  {}\n{digest}  {}\n",
            installer_name("1.2.3"), portable_name("1.2.3")
        );
        assert_eq!(
            parse_formal_manifest(full.as_bytes())
                .expect("full release")
                .release_type,
            "full"
        );
        assert!(parse_formal_manifest(
            format!("{resources}{digest}  {}\n", installer_name("1.2.3")).as_bytes()
        )
        .is_err());
        assert!(parse_formal_manifest(
            full.replace(&installer_name("1.2.3"), &installer_name("1.2.4"))
                .as_bytes()
        )
        .is_err());
    }

    #[test]
    fn formal_manifest_rejects_duplicate_or_candidate_header() {
        let digest = "0123456789abcdef".repeat(4);
        let base = format!(
            "# gogoke-Version: 1.2.3\n# gogoke-Release-Type: resources\n{digest}  resource-index.json\n{digest}  gogoke-resources.windows.zip\n"
        );
        assert!(
            parse_formal_manifest(format!("{base}{digest}  resource-index.json\n").as_bytes())
                .is_err()
        );
        assert!(parse_formal_manifest(
            base.replace(
                "# gogoke-Release-Type: resources\n",
                "# gogoke-Release-Type: resources\n# gogoke-Purpose: TEST_ONLY\n"
            )
            .as_bytes()
        )
        .is_err());
    }

    #[test]
    fn release_asset_redirect_host_is_inside_the_update_trust_boundary() {
        assert!(validate_url(
            "https://release-assets.githubusercontent.com/github-production-release-asset/file"
        )
        .is_ok());
        assert!(
            validate_url("https://release-assets.githubusercontent.com.evil.example/file").is_err()
        );
    }

    #[test]
    fn owner_signed_fixture_verifies_and_mutation_fails() {
        let manifest = include_bytes!("../testdata/gogoke-update/SHA256SUMS.windows");
        let signature = include_bytes!("../testdata/gogoke-update/SHA256SUMS.windows.sig");
        assert!(verify_manifest(manifest, signature).is_ok());

        let mut mutated = manifest.to_vec();
        let last = mutated.len() - 1;
        mutated[last] ^= 1;
        assert!(verify_manifest(&mutated, signature).is_err());
    }

    fn readiness_test_directory() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "gogoke-update-ready-test-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&directory).expect("create readiness test directory");
        directory
    }

    #[test]
    fn readiness_receipt_is_readable_immediately_after_publish() {
        let directory = readiness_test_directory();
        let path = directory.join("gogoke-update-test.ready");
        assert!(!path.exists());
        let hook_called = std::cell::Cell::new(false);

        publish_update_ready_with_hook(&path, b"0.1.1", |final_path, partial_path| {
            hook_called.set(true);
            assert!(!final_path.exists());
            assert!(partial_path.is_file());
            assert_eq!(
                std::fs::read(partial_path).expect("read synced partial receipt"),
                b"0.1.1"
            );
            Ok(())
        })
        .expect("publish readiness receipt");

        assert!(hook_called.get());
        assert_eq!(
            std::fs::read(&path).expect("read readiness receipt"),
            b"0.1.1"
        );
        let partials = std::fs::read_dir(&directory)
            .expect("read readiness test directory")
            .map(|entry| entry.expect("read readiness entry").path())
            .filter(|entry| entry != &path)
            .collect::<Vec<_>>();
        assert!(partials.is_empty(), "partial receipt was not cleaned up");
        std::fs::remove_dir_all(directory).expect("remove readiness test directory");
    }

    #[test]
    fn readiness_receipt_refuses_to_overwrite_existing_final() {
        let directory = readiness_test_directory();
        let path = directory.join("gogoke-update-test.ready");
        std::fs::write(&path, b"old-version").expect("write existing readiness receipt");

        let error = publish_update_ready(&path, b"new-version")
            .expect_err("existing readiness receipt must fail closed");

        assert!(error.contains("already exists"));
        assert_eq!(
            std::fs::read(&path).expect("read existing readiness receipt"),
            b"old-version"
        );
        std::fs::remove_dir_all(directory).expect("remove readiness test directory");
    }

    #[test]
    fn readiness_publish_failure_preserves_final_type_and_cleans_partial() {
        let directory = readiness_test_directory();
        let path = directory.join("gogoke-update-test.ready");

        let error = publish_update_ready_with_hook(&path, b"new-version", |final_path, _| {
            std::fs::create_dir(final_path).expect("create competing final directory");
            Ok(())
        })
        .expect_err("competing final path must make publish fail closed");

        assert!(error.contains("publish gogoke update readiness receipt"));
        assert!(path.is_dir());
        let partials = std::fs::read_dir(&directory)
            .expect("read readiness test directory")
            .map(|entry| entry.expect("read readiness entry").path())
            .filter(|entry| entry != &path)
            .collect::<Vec<_>>();
        assert!(partials.is_empty(), "partial receipt was not cleaned up");
        std::fs::remove_dir_all(directory).expect("remove readiness test directory");
    }
}
