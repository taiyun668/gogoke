//! Native, read-only preparation of the fixed non-Codex login commands.
//! The caller proves User origin; this module verifies Owner and F identity,
//! then returns process data. It never starts a child or reads credentials.

use super::{recipe, BrowserBehavior, EnvironmentValue, LoginProvider, RecipeAvailability, StatusContract};
use crate::process::{NativeBinding, PrepareRequest, ProcessLaunch};
use crate::root::RootLock;
use crate::store::atomic::{Json, JsonString, Statement};
use crate::store::authority::{read_product_identity, OwnerIssuer};
use crate::store::instance::{reconcile_register_replay,
    RegistryError, RegistrationReplay, ResolvedDirectory};
use crate::store::instance::registry::resolve_registered_provider_home;
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::session_transport::decode_request;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct ProviderLoginPreparationError(pub(crate) String);

impl ProviderLoginPreparationError {
    fn source(label: &str, error: impl std::fmt::Debug) -> Self {
        Self(format!("provider login {label}: {error:?}"))
    }
    fn denied(reason: &'static str) -> Self { Self(format!("provider login: {reason}")) }
}

/// A login command may be available while an independent status contract is
/// still unknown. In that case a zero exit from login proves no account state.
pub(crate) enum LoginPreparation {
    Ready(PreparedProviderLogin),
    Unsupported { driver_id: String, reason: &'static str },
}

pub(crate) struct PreparedProviderLogin {
    pub(crate) instance_id: String,
    pub(crate) driver_id: String,
    pub(crate) version: String,
    pub(crate) program_digest: String,
    pub(crate) application: PathBuf,
    pub(crate) home: ResolvedDirectory,
    pub(crate) login: PrepareRequest,
    pub(crate) status: StatusObservation,
    pub(crate) browser: BrowserBehavior,
}

pub(crate) enum StatusObservation {
    Documented(PreparedStatusObservation),
    OpenCodeCredentialList(PrepareRequest),
    GrokModelsAuthenticationHeading(PrepareRequest),
    Unknown(&'static str),
}

pub(crate) struct PreparedStatusObservation {
    pub(crate) request: PrepareRequest,
    pub(crate) logged_in_exit: i32,
    pub(crate) logged_out_exit: i32,
}

impl PreparedStatusObservation {
    pub(crate) fn classify_exit(&self, exit_code: i32) -> Option<bool> {
        if exit_code == self.logged_in_exit { Some(true) }
        else if exit_code == self.logged_out_exit { Some(false) }
        else { None }
    }
}

struct RegisteredPin {
    driver_id: String,
    digest: String,
    version: String,
    revision: u64,
}

fn registered_pin(db: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<RegisteredPin, ProviderLoginPreparationError> {
    let row = Statement::prepare(db.as_ptr(),
        "SELECT driver_id,program_digest,version,revision FROM main.gogoke_v37_instances WHERE instance_id=?1")
        .map_err(|error| ProviderLoginPreparationError::source("instance row", error))?;
    row.bind_text(1, instance_id)
        .map_err(|error| ProviderLoginPreparationError::source("instance bind", error))?;
    if !row.step_row().map_err(|error| ProviderLoginPreparationError::source("instance read", error))? {
        return Err(ProviderLoginPreparationError::denied("registered instance absent"));
    }
    let pin = RegisteredPin {
        driver_id: row.column_text(0).map_err(|error| ProviderLoginPreparationError::source("driver", error))?,
        digest: row.column_text(1).map_err(|error| ProviderLoginPreparationError::source("digest", error))?,
        version: row.column_text(2).map_err(|error| ProviderLoginPreparationError::source("version", error))?,
        revision: row.column_text(3).map_err(|error| ProviderLoginPreparationError::source("revision", error))?
            .parse().map_err(|error| ProviderLoginPreparationError::source("revision parse", error))?,
    };
    if pin.revision == 0 || row.step_row().map_err(|error| ProviderLoginPreparationError::source("duplicate instance", error))? {
        return Err(ProviderLoginPreparationError::denied("invalid instance row"));
    }
    Ok(pin)
}

fn hex_bytes(value: &str) -> Result<Vec<u8>, ProviderLoginPreparationError> {
    if value.is_empty() || value.len() > 262_144 || value.len() % 2 != 0 {
        return Err(ProviderLoginPreparationError::denied("invalid registration fingerprint size"));
    }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let digit = |byte: u8| (byte as char).to_digit(16)
            .ok_or(ProviderLoginPreparationError::denied("invalid registration fingerprint hex"));
        Ok(((digit(pair[0])? << 4) | digit(pair[1])?) as u8)
    }).collect()
}

fn registration_frame(value: &str) -> Result<Option<Vec<u8>>, ProviderLoginPreparationError> {
    let bytes = hex_bytes(value)?;
    let mut offset = 0usize;
    let mut fields = Vec::new();
    while offset < bytes.len() {
        if fields.len() >= 16 || bytes.len() - offset < 8 {
            return Err(ProviderLoginPreparationError::denied("invalid registration framing"));
        }
        let length = u64::from_be_bytes(bytes[offset..offset + 8].try_into()
            .map_err(|error| ProviderLoginPreparationError::source("registration length", error))?);
        offset += 8;
        let length = usize::try_from(length)
            .map_err(|error| ProviderLoginPreparationError::source("registration length", error))?;
        if length > 65_536 || length > bytes.len() - offset {
            return Err(ProviderLoginPreparationError::denied("invalid registration field length"));
        }
        fields.push(&bytes[offset..offset + length]);
        offset += length;
    }
    if fields.len() == 5 { Ok(Some(fields[0].to_vec())) } else { Ok(None) }
}

fn verify_registration(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    instance_id: &str, driver_id: &str) -> Result<(), ProviderLoginPreparationError> {
    let rows = Statement::prepare(db.as_ptr(),
        "SELECT request_id,request_hex FROM main.gogoke_v37_instance_operations \
         WHERE target_id=?1 AND phase='APPLIED' ORDER BY request_id")
        .map_err(|error| ProviderLoginPreparationError::source("registration journal", error))?;
    rows.bind_text(1, instance_id)
        .map_err(|error| ProviderLoginPreparationError::source("registration bind", error))?;
    let mut source: Option<(String, Vec<u8>)> = None;
    while rows.step_row().map_err(|error| ProviderLoginPreparationError::source("registration read", error))? {
        let request_id = rows.column_text(0)
            .map_err(|error| ProviderLoginPreparationError::source("registration request ID", error))?;
        let fingerprint = rows.column_text(1)
            .map_err(|error| ProviderLoginPreparationError::source("registration fingerprint", error))?;
        let Some(frame) = registration_frame(&fingerprint)? else { continue; };
        let request = decode_request(&frame)
            .map_err(|error| ProviderLoginPreparationError::source("registration request", error))?;
        let declared_driver = match request.payload.get(&JsonString::from_str("driverId")) {
            Some(Json::String(value)) => value.to_well_formed_string(),
            _ => None,
        };
        if request.family != "K-INSTANCE" || request.operation != "register"
            || request.request_id != request_id || request.target_id != instance_id
            || request.domain_id != "global" || request.payload.len() != 1
            || declared_driver.as_deref() != Some(driver_id) {
            return Err(ProviderLoginPreparationError::denied("registration journal identity mismatch"));
        }
        if source.replace((request_id, frame)).is_some() {
            return Err(ProviderLoginPreparationError::denied("duplicate registration source"));
        }
    }
    drop(rows);
    let (request_id, frame) = source.ok_or(ProviderLoginPreparationError::denied("registration source absent"))?;
    match reconcile_register_replay(db, root, &request_id, instance_id, &frame)
        .map_err(|error: RegistryError| ProviderLoginPreparationError::source("registration replay", error))? {
        RegistrationReplay::Replayed => Ok(()),
        RegistrationReplay::Unseen | RegistrationReplay::Pending =>
            Err(ProviderLoginPreparationError::denied("registration has no current physical receipt")),
    }
}

fn host_directory_value(name: &str) -> Result<String, ProviderLoginPreparationError> {
    let value = std::env::var(name)
        .map_err(|error| ProviderLoginPreparationError::source(name, error))?;
    if !Path::new(&value).is_absolute() || value.contains('\0') {
        return Err(ProviderLoginPreparationError::denied("invalid host environment directory"));
    }
    Ok(value)
}

fn login_environment(home: &Path, provider: LoginProvider)
    -> Result<Vec<(String, String)>, ProviderLoginPreparationError> {
    let system_root = host_directory_value("SystemRoot")?;
    // Fixed Windows CLIs resolve their browser helper with where.exe. Supply
    // only the system executable directory, never the caller's search path.
    let system_executables = Path::new(&system_root).join("System32");
    let system_executables = system_executables.to_str()
        .ok_or(ProviderLoginPreparationError::denied("system directory is not Unicode"))?;
    let home_text = home.to_str().ok_or(ProviderLoginPreparationError::denied("instance home is not Unicode"))?;
    let mut environment = vec![
        ("SystemRoot".into(), system_root.clone()), ("WINDIR".into(), system_root),
        ("PATH".into(), system_executables.into()), ("PATHEXT".into(), ".EXE".into()),
        ("TEMP".into(), home_text.into()), ("TMP".into(), home_text.into()),
    ];
    for intent in recipe(provider).environment {
        let value = match intent.value {
            EnvironmentValue::InstanceHome | EnvironmentValue::InstanceHomeChild(_) =>
                intent.value_for(home).ok_or(ProviderLoginPreparationError::denied("invalid environment recipe"))?,
            EnvironmentValue::PreserveUserValue => host_directory_value(intent.name)?,
            EnvironmentValue::RemoveInherited => continue,
        };
        environment.push((intent.name.into(), value));
    }
    Ok(environment)
}

fn launch(application: &Path, home: &Path, environment: &[(String, String)],
    argv: &[&str], binding: &NativeBinding) -> PrepareRequest {
    let mut child = ProcessLaunch::new(application);
    child.arguments = argv.iter().map(|argument| (*argument).to_owned()).collect();
    child.current_directory = Some(home.to_owned());
    child.environment = Some(environment.to_vec());
    child.protocol_stdio = true;
    child.persistent_protocol_stdio = true;
    PrepareRequest { launch: child, binding: binding.clone() }
}

/// Revalidate the registered physical home for cleanup after a durable stop.
/// This does not require the executable to remain installed and never opens
/// or enumerates credential files in that home.
pub(crate) fn resolve_registered_login_home(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, instance_id: &str, expected_driver: &str)
    -> Result<ResolvedDirectory, ProviderLoginPreparationError> {
    read_product_identity(db, owner)
        .map_err(|error| ProviderLoginPreparationError::source("Owner identity", error))?;
    let pin = registered_pin(db, instance_id)?;
    if pin.driver_id != expected_driver {
        return Err(ProviderLoginPreparationError::denied("registered driver changed"));
    }
    verify_registration(db, root, instance_id, expected_driver)?;
    let (path, identity) = resolve_registered_provider_home(db, root, instance_id, expected_driver)
        .map_err(|error| ProviderLoginPreparationError::source("registered home", error))?;
    Ok(ResolvedDirectory { path, identity })
}

/// Read F's original registration and current physical objects on the same
/// verified DB/root/Owner identity. The caller owns process dispatch, raw
/// output custody, cancellation, browser presentation and later re-observation.
pub(crate) fn prepare_registered_provider_login(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, instance_id: &str)
    -> Result<LoginPreparation, ProviderLoginPreparationError> {
    read_product_identity(db, owner)
        .map_err(|error| ProviderLoginPreparationError::source("Owner identity", error))?;
    let pin = registered_pin(db, instance_id)?;
    let provider = match pin.driver_id.as_str() {
        "claude" => LoginProvider::Claude,
        "opencode" => LoginProvider::OpenCode,
        "grok" => LoginProvider::Grok,
        "antigravity" => LoginProvider::Antigravity,
        "codex" => LoginProvider::Codex,
        _ => return Err(ProviderLoginPreparationError::denied("unrecognized registered driver")),
    };
    let recipe = recipe(provider);
    verify_registration(db, root, instance_id, &pin.driver_id)?;
    let (home_path, home_identity) = resolve_registered_provider_home(db, root, instance_id, &pin.driver_id)
        .map_err(|error| ProviderLoginPreparationError::source("registered home", error))?;
    let home = ResolvedDirectory { path: home_path, identity: home_identity };
    match recipe.availability {
        RecipeAvailability::Unsupported(reason) => return Ok(LoginPreparation::Unsupported {
            driver_id: pin.driver_id, reason,
        }),
        RecipeAvailability::OutOfScope => return Ok(LoginPreparation::Unsupported {
            driver_id: pin.driver_id, reason: "existing Codex login path owns this driver",
        }),
        RecipeAvailability::Supported => (),
    }
    if pin.version != recipe.pinned_version {
        return Err(ProviderLoginPreparationError::denied("registered version differs from fixed login recipe"));
    }
    let application = super::program_source::locate_bound_instance_program(db,instance_id,
        &pin.driver_id, &pin.digest, &pin.version)
        .map_err(|error| ProviderLoginPreparationError::source("pinned executable", error))?;
    if !application.is_absolute() || application.file_name().and_then(|name| name.to_str())
        != Some(match provider { LoginProvider::Claude => "claude.exe",
            LoginProvider::OpenCode => "opencode.exe", LoginProvider::Grok => "grok.exe",
            _ => return Err(ProviderLoginPreparationError::denied("unsupported provider launch")) }) {
        return Err(ProviderLoginPreparationError::denied("catalog executable identity mismatch"));
    }
    let environment = login_environment(&home.path, provider)?;
    let binding = NativeBinding { binary_digest_sha256: pin.digest.clone(),
        profile_id: instance_id.to_owned(), domain_id: "global".into(),
        generation: pin.revision.to_string() };
    let login = launch(&application, &home.path, &environment, recipe.argv, &binding);
    let status = match (recipe.status_argv, recipe.status_contract) {
        (Some(argv), StatusContract::DocumentedExitCodes { logged_in, logged_out }) =>
            StatusObservation::Documented(PreparedStatusObservation {
                request: launch(&application, &home.path, &environment, argv, &binding),
                logged_in_exit: logged_in, logged_out_exit: logged_out,
            }),
        (Some(argv), StatusContract::OpenCodeCredentialList) =>
            StatusObservation::OpenCodeCredentialList(launch(
                &application, &home.path, &environment, argv, &binding)),
        (Some(argv), StatusContract::GrokModelsAuthenticationHeading) =>
            StatusObservation::GrokModelsAuthenticationHeading(launch(
                &application, &home.path, &environment, argv, &binding)),
        _ => StatusObservation::Unknown("no fixed-version independent status result contract"),
    };
    // Preserve the physical identity in the returned evidence. A later launch
    // must re-resolve F and let ProcessCustodian check the exact image bytes.
    Ok(LoginPreparation::Ready(PreparedProviderLogin { instance_id: instance_id.to_owned(),
        driver_id: pin.driver_id, version: pin.version, program_digest: pin.digest,
        application, home, login, status, browser: recipe.browser }))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn registered_provider_login_system_lookup_is_explicit_and_not_inherited() {
        let home = std::env::temp_dir().join(format!("gogoke-login-lookup-{}-{}",
            std::process::id(), std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&home).unwrap();
        let environment = login_environment(&home, LoginProvider::Claude).unwrap();
        let system = Path::new(&host_directory_value("SystemRoot").unwrap()).join("System32");
        assert_eq!(environment.iter().find(|(key, _)| key == "PATH").unwrap().1,
            system.to_str().unwrap());
        assert_eq!(environment.iter().find(|(key, _)| key == "PATHEXT").unwrap().1, ".EXE");
        assert!(environment.iter().all(|(key, _)| !matches!(key.as_str(),
            "BROWSER" | "NODE_OPTIONS" | "ANTHROPIC_API_KEY" | "CLAUDE_CODE_OAUTH_TOKEN")));
        for key in ["HOME", "USERPROFILE", "CLAUDE_CONFIG_DIR"] {
            assert_eq!(environment.iter().find(|(name, _)| name == key).unwrap().1,
                home.to_str().unwrap());
        }
        // This is the exact inner command used by the fixed Claude image,
        // not a browser launch or an account/network authentication attempt.
        let lookup = |entries: &[(String, String)]| std::process::Command::new(system.join("where.exe"))
            .arg("rundll32").current_dir(&home).env_clear()
            .envs(entries.iter().map(|(key, value)| (key, value)))
            .output().unwrap();
        let original: Vec<_> = environment.iter()
            .filter(|(key, _)| !matches!(key.as_str(), "PATH" | "PATHEXT"))
            .cloned().collect();
        let rejected = lookup(&original);
        assert!(!rejected.status.success(), "lookup unexpectedly inherited system search state");
        let found = lookup(&environment);
        assert!(found.status.success(), "system lookup: {}", String::from_utf8_lossy(&found.stderr));
        let actual = String::from_utf8(found.stdout).unwrap();
        assert_eq!(std::fs::canonicalize(actual.trim()).unwrap(),
            std::fs::canonicalize(system.join("rundll32.exe")).unwrap());
        std::fs::remove_dir(home).unwrap();
    }
}
