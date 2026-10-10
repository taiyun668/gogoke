//! Real fixed official image setup in each test's private root.
//! Formal evidence remains CI-only. Owner-authorized local development also
//! requires an explicit opt-in, actual SAC=0 and a canonical D-drive TEMP.
//! Every root still passes the production archive check and H version probe.
use super::*;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

static ARCHIVES: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

fn require_fixture_environment() {
    if std::env::var("GITHUB_ACTIONS").as_deref() == Ok("true") {
        return;
    }
    assert_eq!(std::env::var("GOGOKE_LOCAL_NATIVE_DEVELOPMENT").as_deref(), Ok("true"),
        "managed CLI fixture needs cloud CI or explicit authorized local development");
    #[link(name = "advapi32")]
    extern "system" {
        fn RegGetValueW(key: *mut std::ffi::c_void, subkey: *const u16,
            value: *const u16, flags: u32, kind: *mut u32,
            data: *mut std::ffi::c_void, bytes: *mut u32) -> i32;
    }
    let subkey: Vec<u16> = "SYSTEM\\CurrentControlSet\\Control\\CI\\Policy\0".encode_utf16().collect();
    let value: Vec<u16> = "VerifiedAndReputablePolicyState\0".encode_utf16().collect();
    let mut kind = 0u32;
    let mut state = u32::MAX;
    let mut bytes = 4u32;
    let code = unsafe { RegGetValueW(0x80000002u32 as i32 as isize as *mut _,
        subkey.as_ptr(), value.as_ptr(), 0x00000010, &mut kind,
        (&mut state as *mut u32).cast(), &mut bytes) };
    assert_eq!((code, kind, bytes, state), (0, 4, 4, 0),
        "local native development requires actual SAC DWORD=0; raw registry result");
    let temp = fs::canonicalize(std::env::temp_dir()).expect("canonical local development TEMP");
    let drive = temp.components().next();
    assert!(matches!(drive, Some(std::path::Component::Prefix(prefix))
        if matches!(prefix.kind(), std::path::Prefix::Disk(b'D' | b'd')
            | std::path::Prefix::VerbatimDisk(b'D' | b'd'))),
        "local fixture data and downloads must remain on physical D drive");
}

fn official_url(driver: &str, version: &str) -> &'static str {
    match (driver, version) {
        ("codex", "0.160.0") =>
            "https://registry.npmjs.org/@openai/codex/-/codex-0.160.0-win32-x64.tgz",
        ("claude", "2.1.196") =>
            "https://registry.npmjs.org/@anthropic-ai/claude-code-win32-x64/-/claude-code-win32-x64-2.1.196.tgz",
        ("opencode", "1.18.32") =>
            "https://registry.npmjs.org/opencode-windows-x64/-/opencode-windows-x64-1.18.32.tgz",
        ("grok", "1.0.41") => "https://x.ai/cli/grok-1.0.41-windows-x86_64.exe",
        _ => panic!("no fixed official test archive for {driver} {version}"),
    }
}

fn archive(driver: &str, version: &str, expected: &str) -> PathBuf {
    require_fixture_environment();
    let mut archives = ARCHIVES.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if let Some(path) = archives.get(driver) { return path.clone(); }
    let cache = std::env::temp_dir().join(format!("gogoke-managed-cli-fixture-{}", std::process::id()));
    fs::create_dir_all(&cache).unwrap();
    let path = cache.join(format!("{driver}-{version}.download"));
    let status = Command::new("curl.exe").args(["--fail", "--location", "--silent", "--show-error",
        "--output"]).arg(&path).arg(official_url(driver, version))
        .status().expect("curl.exe fixed official archive");
    assert!(status.success(), "fixed official archive download failed: {status}");
    assert_eq!(crate::store::digest::sha256_hex(&fs::read(&path).unwrap()), expected,
        "fixed official archive bytes changed");
    archives.insert(driver.to_owned(), path.clone());
    path
}

fn command(product: &mut ProductDatabase<'_>, command: &str, driver: &str,
    stage: &str, request: Option<&str>, expected: &str) {
    let mut fields = format!(
        r#"{{"schema":"gogoke.37.managed-cli.v1","command":"{command}","driverId":"{driver}","stageName":"{stage}""#);
    if let Some(request) = request { fields.push_str(&format!(r#", "requestId":"{request}""#)); }
    fields.push('}');
    let reply = product.dispatch_user_managed_cli(fields.as_bytes())
        .unwrap_or_else(|error| panic!("managed CLI {command} failed: {error:?}"));
    let reply = String::from_utf8(reply).unwrap();
    assert!(reply.contains(&format!(r#""state":"{expected}""#)),
        "managed CLI {command} did not reach {expected}: {reply}");
}

pub(super) fn prepared_image(root: &RootLock, driver: &str) -> (String, PathBuf) {
    let pin = instance::read_fixed_official_cli(driver).expect("fixed official CLI pin");
    let original = archive(driver, pin.version, pin.archive_sha256);
    let staging = instance::managed_cli_root(root).expect("private managed CLI staging root");
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let stage = format!("{}-{}-fixture-{nonce}", pin.driver, pin.version);
    let folder = staging.join(&stage);
    let content = folder.join("content");
    fs::create_dir(&folder).unwrap();
    fs::create_dir(&content).unwrap();
    let image = pin.image_relative.split('/').fold(content.clone(), |path, part| path.join(part));
    fs::create_dir_all(image.parent().unwrap()).unwrap();
    if pin.raw_image {
        fs::copy(&original, &image).unwrap();
    } else {
        let source = folder.join("source.download");
        fs::copy(&original, &source).unwrap();
        let status = Command::new("tar.exe").arg("-xzf").arg(&source).arg("-C").arg(&content)
            .status().expect("tar.exe fixed official archive");
        assert!(status.success(), "fixed official archive extraction failed: {status}");
    }
    instance::inspect_staged_official_cli(root, driver, &stage)
        .expect("native verified the per-root original archive and executable");
    (stage, image)
}

pub(super) fn ready(product: &mut ProductDatabase<'_>, root: &RootLock, driver: &str) {
    let (stage, _) = prepared_image(root, driver);
    staged_probe(product,driver,&stage);
    command(product, "migrate", driver, &stage, None, "READY");
    let copy = instance::read_managed_cli(&product.connection, root, driver)
        .unwrap().expect("managed CLI copy row");
    assert_eq!(copy.state, "READY");
    assert_eq!(copy.stage_name.as_deref(), Some(stage.as_str()));
}

pub(super) fn staged_probe(product: &mut ProductDatabase<'_>, driver: &str, stage: &str) {
    command(product, "stage", driver, &stage, None, "STAGED");
    command(product, "probe", driver, &stage, Some("fixture-probe"), "PROBED");
}
