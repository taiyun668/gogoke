use futures_util::StreamExt;
use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};
use reqwest::{Client, Url};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{AppHandle, Manager};
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
const RELEASE_PUBLIC_KEY: &str = include_str!("../gogoke-release-public-key.txt");
const UPDATE_COORDINATOR: &str = include_str!("../update/gogoke-update-coordinator.ps1");
const UPDATE_READY_PREFIX: &str = "--gogoke-update-ready=";

static UPDATE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
    asset: String,
    sha256: String,
    published_at: String,
    notes_url: String,
    notes: String,
}

#[derive(Clone, Debug)]
struct ReleaseIdentity {
    offer: GogokeUpdateOffer,
    installer_url: String,
}

#[derive(Debug, Deserialize, Serialize)]
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
        .header(
            "User-Agent",
            format!("gogoke/{}", env!("CARGO_PKG_VERSION")),
        )
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
        .header(
            "User-Agent",
            format!("gogoke/{}", env!("CARGO_PKG_VERSION")),
        )
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

fn manifest_version(manifest: &str) -> Option<&str> {
    let values: Vec<_> = manifest
        .lines()
        .filter_map(|line| line.strip_prefix("# gogoke-Version:").map(str::trim))
        .collect();
    (values.len() == 1 && !values[0].is_empty()).then_some(values[0])
}

fn checksum_for(manifest: &str, asset: &str) -> Option<String> {
    let values: Vec<_> = manifest
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let digest = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            if parts.next().is_none()
                && name == asset
                && digest.len() == 64
                && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                Some(digest.to_ascii_lowercase())
            } else {
                None
            }
        })
        .collect();
    (values.len() == 1).then(|| values[0].clone())
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

fn staged_installer(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(update_directory(app)?.join("installer.exe"))
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
    let current_dir = current_exe
        .parent()
        .ok_or_else(|| "current gogoke executable has no parent directory".to_string())?;
    if current_dir.join("uninstall.exe").is_file() {
        return Ok(current_dir.to_path_buf());
    }
    Ok(app
        .path()
        .local_data_dir()
        .map_err(|error| format!("could not resolve local application data: {error}"))?
        .join("Programs")
        .join("gogoke"))
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

pub fn signal_update_ready(app: &AppHandle) -> Result<(), String> {
    let Some(path) = ready_path_from_args()? else {
        return Ok(());
    };
    publish_update_ready(&path, env!("CARGO_PKG_VERSION").as_bytes())?;
    let state_path = update_state_path(app)?;
    if let Ok(payload) = std::fs::read(&state_path) {
        if let Ok(mut state) = serde_json::from_slice::<PreparedUpdateState>(&payload) {
            state.status = "installed".to_string();
            state.last_error = None;
            let _ = write_update_state(app, &state);
        }
    }
    Ok(())
}

#[tauri::command]
pub fn gogoke_update_signal_ready(app: AppHandle) -> Result<(), String> {
    signal_update_ready(&app)
}

#[tauri::command]
pub fn gogoke_update_take_failure(app: AppHandle) -> Result<Option<String>, String> {
    consume_update_failure(&app)
}

async fn release_identity(client: &Client) -> Result<Option<ReleaseIdentity>, String> {
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
    let current = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|_| "current app version is not SemVer".to_string())?;
    if candidate <= current {
        return Ok(None);
    }
    let asset_name = installer_name(version_text);
    let find_asset = |name: &str| {
        let matches: Vec<_> = release
            .assets
            .iter()
            .filter(|asset| asset.name == name)
            .collect();
        (matches.len() == 1).then_some(matches[0])
    };
    let Some(installer) = find_asset(&asset_name) else {
        return Ok(None);
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
    let manifest = std::str::from_utf8(&manifest_bytes)
        .map_err(|_| "release checksum manifest is not UTF-8".to_string())?;
    if manifest_version(manifest) != Some(version_text) {
        return Err("signed manifest version does not match release tag".to_string());
    }
    let digest = checksum_for(manifest, &asset_name)
        .ok_or_else(|| "signed manifest does not identify one exact installer".to_string())?;
    validate_url(&installer.browser_download_url)?;
    validate_url(&release.html_url)?;
    Ok(Some(ReleaseIdentity {
        offer: GogokeUpdateOffer {
            version: version_text.to_string(),
            asset: asset_name,
            sha256: digest,
            published_at: release.published_at,
            notes_url: release.html_url,
            notes: release.body,
        },
        installer_url: installer.browser_download_url.clone(),
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

#[tauri::command]
pub async fn gogoke_update_check(app: AppHandle) -> Result<Option<GogokeUpdateOffer>, String> {
    if !cfg!(target_os = "windows") {
        return Ok(None);
    }
    let _guard = UPDATE_LOCK.lock().await;
    let client = client()?;
    let Some(identity) = release_identity(&client).await? else {
        return Ok(None);
    };
    let path = staged_installer(&app)?;
    if let Ok(prepared) = read_update_state(&app) {
        if prepared.offer.version == identity.offer.version
            && prepared.offer.asset == identity.offer.asset
            && prepared.offer.sha256 == identity.offer.sha256
            && path.is_file()
            && sha256(&path)? == identity.offer.sha256
        {
            return Ok(Some(identity.offer));
        }
    }
    let bytes = get_bytes(&client, &identity.installer_url, MAX_INSTALLER_BYTES).await?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if digest != identity.offer.sha256 {
        return Err("downloaded installer does not match signed manifest".to_string());
    }
    let partial = path.with_extension("part");
    std::fs::write(&partial, bytes)
        .map_err(|error| format!("could not stage gogoke update: {error}"))?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("could not replace the staged gogoke update: {error}"))?;
    }
    std::fs::rename(&partial, &path)
        .map_err(|error| format!("could not publish staged gogoke update: {error}"))?;
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

#[tauri::command]
pub async fn gogoke_update_install(app: AppHandle, version: String) -> Result<(), String> {
    if !cfg!(target_os = "windows") {
        return Err("gogoke automatic updates are currently available on Windows only".to_string());
    }
    let _guard = UPDATE_LOCK.lock().await;
    let prepared = read_update_state(&app)?;
    if prepared.offer.version != version {
        return Err("the requested gogoke update is not the prepared version".to_string());
    }
    let client = client()?;
    let identity = release_identity(&client)
        .await?
        .ok_or_else(|| "the prepared gogoke update is no longer available".to_string())?;
    if identity.offer.version != version {
        return Err("the prepared gogoke update version is stale".to_string());
    }
    let installer = staged_installer(&app)?;
    if sha256(&installer)? != identity.offer.sha256 {
        return Err(
            "the prepared gogoke installer no longer matches its signed manifest".to_string(),
        );
    }

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
        .arg("-ExpectedSha256")
        .arg(&prepared.offer.sha256)
        .arg("-LockFile")
        .arg(&lock_file)
        .arg("-FailureLog")
        .arg(&failure_log);
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
    fn manifest_binding_requires_one_exact_asset_and_version() {
        let manifest = "# gogoke-Version: 1.2.3\n0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  gogoke-1.2.3-windows-x64-unsigned-setup.exe\n";
        assert_eq!(manifest_version(manifest), Some("1.2.3"));
        assert!(checksum_for(manifest, &installer_name("1.2.3")).is_some());
        assert!(checksum_for(manifest, &installer_name("1.2.4")).is_none());
    }

    #[test]
    fn duplicate_checksum_is_ambiguous() {
        let line = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  gogoke-1.2.3-windows-x64-unsigned-setup.exe\n";
        assert!(checksum_for(&format!("{line}{line}"), &installer_name("1.2.3")).is_none());
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
