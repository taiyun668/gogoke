//! Private F.1 Codex login preparation and native account observation.
//!
//! The Owner enters credentials in the official CLI. This module never reads
//! `auth.json`, a keyring, or any account field, and never starts a model turn.
//! The caller must establish User origin before invoking this private action.

use super::*;
use crate::process::{AppContainerProfile, CompatModule};
use crate::process::{DurableStopConfirmation, NativeBinding, NativeStopProof, OriginBoundFrame,
    PrepareRequest, PreparedCustody, ProcessCustodyError, ProcessLaunch, StopBudgets};
use crate::root::{inspect_root, RootIdentity};
#[cfg(all(test, windows))]
#[path = "v37_login_trace.rs"]
mod directed_trace;
use crate::store::atomic::Parser;
use crate::store::instance::{InstanceObservation, ObservationRequest};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const REPARSE_POINT: u32 = 0x400;
const ACCOUNT_READ_EVIDENCE: &str = "CREDENTIAL_PRESENT_NO_VALIDITY_CHECK";
const RPC_DEADLINE: Duration = Duration::from_secs(15);
const MAX_RPC_FRAMES: usize = 16;
const INSTANCE_LIST_SCHEMA: &str = "gogoke.37.instance-list.v1";
const MAX_INSTANCE_LIST_FRAME: usize = 256;
const MAX_INSTANCE_LIST_ENTRIES: usize = 1024;
const MAX_INSTANCE_LIST_BYTES: usize = 256 * 1024;
// The pinned CLI's login file layer otherwise records only flow startup.
// This existing connector target logs TCP destinations/progress/errors, not
// HTTP headers, bodies or device codes. The CLI uses its configured login log,
// which defaults to the private F home.
const OWNER_LOGIN_CONNECT_LOG: &str = concat!(
    "codex_cli=info,codex_core=info,codex_login=info,",
    "hyper_util::client::legacy::connect::http=trace",
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NativeAccountState {
    Unknown,
    LoggedOut,
    CredentialPresent,
}

impl NativeAccountState {
    fn durable(self) -> InstanceObservation {
        match self {
            Self::Unknown => InstanceObservation::LoginUnknown,
            Self::LoggedOut => InstanceObservation::LoggedOut,
            Self::CredentialPresent => InstanceObservation::LoggedIn,
        }
    }
    fn public_state(self) -> &'static str {
        match self {
            Self::Unknown => "UNKNOWN",
            Self::LoggedOut => "LOGGED_OUT",
            Self::CredentialPresent => "LOGGED_IN",
        }
    }
    fn evidence(self) -> &'static str {
        match self {
            Self::CredentialPresent => ACCOUNT_READ_EVIDENCE,
            Self::LoggedOut => "NATIVE_ACCOUNT_READ_NO_ACCOUNT",
            Self::Unknown => "NATIVE_ACCOUNT_READ_INDETERMINATE",
        }
    }
}

/// Two fixed launch specifications for the same pinned CLI and isolated home.
/// `login` is started only by an explicit Owner action; its stdout is delivered
/// on the Owner-private User pipe. `account_read` uses fixed app-server RPC.
/// Preparation creates only host-owned directories and data, with no process.
pub(super) struct PreparedOwnerLogin {
    pub(super) login: PrepareRequest,
    pub(super) account_read: PrepareRequest,
    pub(super) runtime_home: PathBuf,
    pub(super) runtime_identity: RootIdentity,
}

/// In-memory Owner action state. ProductDatabase owns one of these beside its
/// ProcessCustodian. The custody row remains the durable restart fact; after a
/// host restart PREPARED/ACTIVE is UNKNOWN and the same request is not replayed
/// into a new CLI process.
pub(super) enum OwnerLoginSession {
    Active(ActiveOwnerLogin),
    PendingAccount(PendingAccountRead),
    Final {
        instance_id: String,
        request_id: String,
        expected_revision: u64,
        state: String,
        output: String,
    },
}

struct PendingAccountCustody {
    operation_id: Option<String>,
    prepared: Option<PreparedCustody>,
    runtime_home: Option<PathBuf>,
    runtime_identity: Option<RootIdentity>,
    proof: Option<NativeStopProof>,
    durable_revision: Option<u64>,
    abort_prepared: bool,
    frame: Option<OriginBoundFrame>,
    request: Option<V37Request>,
}

struct PendingAccountRead {
    instance_id: String,
    request_id: String,
    expected_revision: u64,
    output: String,
    latest_error: Option<String>,
    custody: PendingAccountCustody,
}

struct AccountObservationFailure {
    error: OrchestrationError,
    pending: Option<PendingAccountCustody>,
}

impl From<OrchestrationError> for AccountObservationFailure {
    fn from(error: OrchestrationError) -> Self { Self { error, pending: None } }
}

impl From<ProcessCustodyError> for AccountObservationFailure {
    fn from(error: ProcessCustodyError) -> Self { Self { error: error.into(), pending: None } }
}

fn account_prepare_failure(launch: &PreparedOwnerLogin, error: ProcessCustodyError) -> AccountObservationFailure {
    // The factory preserves failed-launch handles only for LaunchCleanup.
    // Every other preparation error occurred before launch or after a proven abort.
    let unconfirmed = matches!(&error, ProcessCustodyError::LaunchCleanup { .. });
    let cleanup = if unconfirmed { None } else {
        Some(remove_owned_runtime(&launch.runtime_home, &launch.runtime_identity))
    };
    AccountObservationFailure {
        error: OrchestrationError::V37StoreFailure(format!("CLI process prepare: {error:?}; cleanup: {cleanup:?}")),
        pending: unconfirmed.then(|| PendingAccountCustody {
            operation_id: None, prepared: None,
            runtime_home: Some(launch.runtime_home.clone()), runtime_identity: Some(launch.runtime_identity.clone()),
            proof: None, durable_revision: None, abort_prepared: false, frame: None, request: None,
        }),
    }
}

fn activation_was_aborted(error: &ProcessCustodyError, tombstoned: bool) -> bool {
    // Tombstones also exist for an unconfirmed abort. Resume without
    // LaunchCleanup is returned only after the factory confirms the abort.
    tombstoned && matches!(error, ProcessCustodyError::Resume(_))
}

pub(super) struct ActiveOwnerLogin {
    instance_id: String,
    request_id: String,
    expected_revision: u64,
    operation_id: String,
    prepared: PreparedCustody,
    runtime_home: PathBuf,
    runtime_identity: RootIdentity,
    output: String,
    halted: bool,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum OwnerLoginAction { Begin, Status, Cancel, Refresh }

struct OwnerLoginCommand {
    action: OwnerLoginAction,
    instance_id: String,
    request_id: String,
    expected_revision: u64,
}

fn owner_login_command(frame: &[u8]) -> Result<OwnerLoginCommand> {
    if frame.len() > 4096 { return Err(OrchestrationError::Invalid("owner login frame size")); }
    let text = std::str::from_utf8(frame)
        .map_err(|_| OrchestrationError::Invalid("owner login utf8"))?;
    let value = Parser::parse(text).map_err(OrchestrationError::Atomic)?;
    let mut fields = object(value).ok_or(OrchestrationError::Invalid("owner login object"))?;
    if fields.len() != 5 {
        return Err(OrchestrationError::Invalid("owner login fields"));
    }
    let string = |fields: &mut BTreeMap<JsonString, Json>, key: &'static str| -> Result<String> {
        match fields.remove(&JsonString::from_str(key)) {
            Some(Json::String(value)) => value.to_well_formed_string()
                .ok_or(OrchestrationError::Invalid(key)),
            _ => Err(OrchestrationError::Invalid(key)),
        }
    };
    if string(&mut fields, "schema")? != "gogoke.37.owner-login.v1" {
        return Err(OrchestrationError::Invalid("owner login schema"));
    }
    let action = match string(&mut fields, "action")?.as_str() {
        "begin" => OwnerLoginAction::Begin,
        "status" => OwnerLoginAction::Status,
        "cancel" => OwnerLoginAction::Cancel,
        "refresh" => OwnerLoginAction::Refresh,
        _ => return Err(OrchestrationError::Invalid("owner login action")),
    };
    let instance_id = string(&mut fields, "instanceId")?;
    let request_id = string(&mut fields, "requestId")?;
    if request_id.is_empty() || request_id.len() > 64
        || !request_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || instance_id.is_empty() || instance_id.len() > 64
        || !instance_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(OrchestrationError::Invalid("owner login identity"));
    }
    let expected_revision = match fields.remove(&JsonString::from_str("expectedRevision")) {
        Some(Json::Number(value)) => value.parse::<u64>()
            .map_err(|_| OrchestrationError::Invalid("owner login revision"))?,
        _ => return Err(OrchestrationError::Invalid("owner login revision")),
    };
    if expected_revision == 0 || !fields.is_empty() {
        return Err(OrchestrationError::Invalid("owner login revision"));
    }
    Ok(OwnerLoginCommand { action, instance_id, request_id, expected_revision })
}

fn has_top_level_schema(frame: &[u8], limit: usize, schema: &str) -> bool {
    if frame.len() > limit { return false; }
    let Ok(text) = std::str::from_utf8(frame) else { return false; };
    let Ok(Json::Object(fields)) = Parser::parse(text) else { return false; };
    matches!(fields.get(&JsonString::from_str("schema")),
        Some(Json::String(value)) if value.to_well_formed_string().as_deref() == Some(schema))
}

/// A bounded routing hint only. `owner_login_command` still validates every
/// field after the trusted User pipe has established its process origin.
pub(super) fn is_owner_login_frame(frame: &[u8]) -> bool {
    has_top_level_schema(frame, 4096, "gogoke.37.owner-login.v1")
}

/// Routing hint only; the private User ingress validates the complete frame.
pub(super) fn is_owner_instance_list_frame(frame: &[u8]) -> bool {
    has_top_level_schema(frame, MAX_INSTANCE_LIST_FRAME, INSTANCE_LIST_SCHEMA)
}

impl<'root> ProductDatabase<'root> {
    /// Read the durable F.1 instance registry after the parent verifies User
    /// origin. The list exposes no account data, credential path or home path.
    pub(super) fn dispatch_owner_instance_list_frame(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if frame.len() > MAX_INSTANCE_LIST_FRAME {
            return Err(OrchestrationError::Invalid("owner instance list frame size"));
        }
        let text = std::str::from_utf8(frame)
            .map_err(|_| OrchestrationError::Invalid("owner instance list utf8"))?;
        let value = Parser::parse(text).map_err(OrchestrationError::Atomic)?;
        let mut fields = object(value)
            .ok_or(OrchestrationError::Invalid("owner instance list object"))?;
        if fields.len() != 1 || !matches!(fields.remove(&JsonString::from_str("schema")),
            Some(Json::String(schema)) if schema.to_well_formed_string().as_deref() == Some(INSTANCE_LIST_SCHEMA)) {
            return Err(OrchestrationError::Invalid("owner instance list fields"));
        }
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT instance_id FROM main.gogoke_v37_instances ORDER BY instance_id")?;
        let mut ids = Vec::new();
        while query.step_row()? {
            if ids.len() >= MAX_INSTANCE_LIST_ENTRIES {
                return Err(OrchestrationError::Invalid("owner instance list size"));
            }
            let instance_id = query.column_text(0)?;
            if instance_id.is_empty() || instance_id.len() > 64
                || !instance_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')) {
                return Err(OrchestrationError::Invalid("owner instance list row"));
            }
            ids.push(instance_id);
        }
        drop(query);
        let mut instances = Vec::with_capacity(ids.len());
        for instance_id in ids {
            let row = self.read_registered_instance(&instance_id)?
                .ok_or(OrchestrationError::Invalid("owner instance list row"))?;
            if row.driver_id.is_empty() || row.driver_id.len() > 128
                || row.driver_id.chars().any(char::is_control)
                || row.version.is_empty() || row.version.len() > 128
                || row.version.chars().any(char::is_control) {
                return Err(OrchestrationError::Invalid("owner instance list row"));
            }
            let source = self.registration_source(&instance_id, &row.driver_id)?;
            let install_state = match source.as_ref() {
                Some(source) => match self.current_install_fact(&row, source, &instance_id)? {
                    InstallFact::Installed => "INSTALLED",
                    InstallFact::Missing => "MISSING",
                    InstallFact::Unknown => "UNKNOWN",
                },
                None => "UNKNOWN",
            };
            let login_state = match source.as_ref() {
                Some(source) if self.registered_home_is_current(source, &instance_id)? => {
                    let state = row.login_state.as_str();
                    if state == "UNKNOWN" || (matches!(state, "LOGGED_IN" | "LOGGED_OUT")
                        && self.current_login_observation(&instance_id, row.revision, state)?) {
                        state
                    } else { "UNKNOWN" }
                }
                _ => "UNKNOWN",
            };
            let revision = row.revision.to_string();
            let string = |value: &str| Json::String(JsonString::from_str(value));
            instances.push(Json::Object(BTreeMap::from([
                (JsonString::from_str("instanceId"), string(&instance_id)),
                (JsonString::from_str("driverId"), string(&row.driver_id)),
                (JsonString::from_str("version"), string(&row.version)),
                (JsonString::from_str("installState"), string(&install_state)),
                (JsonString::from_str("loginState"), string(&login_state)),
                (JsonString::from_str("revision"), string(&revision)),
            ])));
        }
        let response = Json::Object(BTreeMap::from([
            (JsonString::from_str("schema"), Json::String(JsonString::from_str(INSTANCE_LIST_SCHEMA))),
            (JsonString::from_str("instances"), Json::Array(instances)),
        ])).canonical().into_bytes();
        if response.len() > MAX_INSTANCE_LIST_BYTES {
            return Err(OrchestrationError::Invalid("owner instance list response size"));
        }
        Ok(response)
    }
}

fn protocol_timed_out(error: &ProcessCustodyError) -> bool {
    match error {
        ProcessCustodyError::ProtocolPipe(source) =>
            source.kind() == std::io::ErrorKind::TimedOut,
        ProcessCustodyError::ProtocolEvidence { cause, .. } => protocol_timed_out(cause),
        _ => false,
    }
}

fn protocol_eof(error: &ProcessCustodyError) -> bool {
    match error {
        ProcessCustodyError::ProtocolPipe(source) =>
            source.kind() == std::io::ErrorKind::UnexpectedEof,
        ProcessCustodyError::ProtocolEvidence { cause, .. } => protocol_eof(cause),
        _ => false,
    }
}

fn owner_login_reply(command: &OwnerLoginCommand, state: &str, output: &str) -> Vec<u8> {
    owner_login_reply_with_settled(command, state, output, false)
}

fn owner_login_final_reply(command: &OwnerLoginCommand, state: &str, output: &str) -> Vec<u8> {
    owner_login_reply_with_settled(command, state, output, true)
}

fn owner_login_reply_with_settled(command: &OwnerLoginCommand, state: &str,
    output: &str, settled: bool) -> Vec<u8> {
    Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), Json::String(JsonString::from_str("gogoke.37.owner-login.v1"))),
        (JsonString::from_str("instanceId"), Json::String(JsonString::from_str(&command.instance_id))),
        (JsonString::from_str("requestId"), Json::String(JsonString::from_str(&command.request_id))),
        (JsonString::from_str("state"), Json::String(JsonString::from_str(state))),
        (JsonString::from_str("settled"), Json::Bool(settled)),
        // Only this Owner-private response carries device-auth stdout. The
        // value is never stored in the instance or coordination journal.
        (JsonString::from_str("output"), Json::String(JsonString::from_str(output))),
    ])).canonical().into_bytes()
}

fn same_owner_login(session: &OwnerLoginSession, command: &OwnerLoginCommand) -> bool {
    let (instance, request, revision) = match session {
        OwnerLoginSession::Active(active) => (&active.instance_id, &active.request_id,
            active.expected_revision),
        OwnerLoginSession::PendingAccount(pending) => (&pending.instance_id, &pending.request_id,
            pending.expected_revision),
        OwnerLoginSession::Final { instance_id, request_id, expected_revision, .. } =>
            (instance_id, request_id, *expected_revision),
    };
    instance == &command.instance_id && request == &command.request_id
        && revision == command.expected_revision
}

fn owner_login_operation_id(command: &OwnerLoginCommand) -> String {
    let identity = format!("{}\n{}\n{}", command.instance_id,
        command.request_id, command.expected_revision);
    let digest = crate::store::digest::sha256_hex(identity.as_bytes());
    format!("owner-login-{}", &digest[..40])
}

fn observation_hex(request: &V37Request, state: NativeAccountState,
    unchanged: bool) -> String {
    let revision = request.expected_revision.to_string();
    let mut fields: Vec<&[u8]> = vec![
        if unchanged { &b"read-login"[..] } else { &b"observe"[..] },
        &request.raw_bytes,
        request.target_id.as_bytes(),
        revision.as_bytes(),
        b"login",
        state.public_state().as_bytes(),
    ];
    if unchanged { fields.push(b"NO_REVISION_CHANGE"); }
    let mut encoded = String::new();
    for field in fields {
        for byte in (field.len() as u64).to_be_bytes().iter().chain(field.iter()) {
            use std::fmt::Write as _;
            write!(&mut encoded, "{byte:02x}").expect("write to String");
        }
    }
    encoded
}

fn state_from_journal(value: &[u8]) -> Option<NativeAccountState> {
    match value {
        b"UNKNOWN" => Some(NativeAccountState::Unknown),
        b"LOGGED_OUT" => Some(NativeAccountState::LoggedOut),
        b"LOGGED_IN" => Some(NativeAccountState::CredentialPresent),
        _ => None,
    }
}

fn checked_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(OrchestrationError::Io)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(OrchestrationError::AccessDenied);
    }
    inspect_root(path).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("login directory identity: {error:?}")))?;
    Ok(())
}

fn runtime_home(instance_home: &Path) -> Result<(PathBuf, RootIdentity)> {
    checked_directory(instance_home)?;
    let path = instance_home.join("gogoke-login-runtime");
    // This fixed host-owned path is exclusive. A residue means custody needs
    // reconciliation; a clock-based retry must not hide the prior attempt.
    fs::create_dir(&path).map_err(OrchestrationError::Io)?;
    checked_directory(&path)?;
    let identity = inspect_root(&path).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("login runtime identity: {error:?}")))?.identity;
    Ok((path, identity))
}

fn remove_owned_runtime(path: &Path, expected: &RootIdentity) -> Result<()> {
    checked_directory(path)?;
    let found = inspect_root(path).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("login cleanup identity: {error:?}")))?;
    if &found.identity != expected { return Err(OrchestrationError::AccessDenied); }
    fn remove_contents(path: &Path, depth: usize) -> Result<()> {
        if depth > 32 { return Err(OrchestrationError::AccessDenied); }
        for entry in fs::read_dir(path).map_err(OrchestrationError::Io)? {
            let entry = entry.map_err(OrchestrationError::Io)?;
            let child = entry.path();
            let metadata = fs::symlink_metadata(&child).map_err(OrchestrationError::Io)?;
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err(OrchestrationError::AccessDenied);
            }
            if metadata.is_dir() {
                remove_contents(&child, depth + 1)?;
                fs::remove_dir(&child).map_err(OrchestrationError::Io)?;
            } else if metadata.is_file() {
                fs::remove_file(&child).map_err(OrchestrationError::Io)?;
            } else {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        Ok(())
    }
    remove_contents(path, 0)?;
    fs::remove_dir(path).map_err(OrchestrationError::Io)
}

fn clean_environment(instance_home: &Path, runtime: &Path) -> Result<Vec<(String, String)>> {
    let system_root = std::env::var("SystemRoot")
        .map_err(|error| OrchestrationError::V37StoreFailure(
            format!("SystemRoot unavailable: {error}")))?;
    if !Path::new(&system_root).is_absolute() || system_root.contains('\0') {
        return Err(OrchestrationError::AccessDenied);
    }
    let runtime = runtime.to_string_lossy().into_owned();
    let instance = instance_home.to_string_lossy().into_owned();
    Ok(vec![
        ("SystemRoot".into(), system_root.clone()),
        ("WINDIR".into(), system_root),
        ("HOME".into(), runtime.clone()),
        ("USERPROFILE".into(), runtime.clone()),
        ("LOCALAPPDATA".into(), runtime.clone()),
        ("APPDATA".into(), runtime.clone()),
        ("TEMP".into(), runtime.clone()),
        ("TMP".into(), runtime),
        ("CODEX_HOME".into(), instance),
        // Pin the finite filter instead of inheriting a caller's broad trace.
        ("RUST_LOG".into(), OWNER_LOGIN_CONNECT_LOG.into()),
    ])
}

fn owner_login_profile_name(instance_id: &str, home_identity: &RootIdentity) -> String {
    // The registered physical home, rather than a wire path or request ID,
    // determines the isolation domain across login and account/read launches.
    let digest = crate::store::digest::sha256_hex(
        format!("{}\n{instance_id}", home_identity.opaque()).as_bytes());
    format!("Gogoke37.OwnerLogin.{}", &digest[..40])
}

fn grant_owner_login_scope(profile: &AppContainerProfile, home: &instance::ResolvedDirectory,
    runtime: &Path, runtime_identity: &RootIdentity, program: &Path) -> Result<()> {
    // The runtime is a child of the exact F home. Its inherited ACE is
    // verified with the rest of that tree; granting it an extra explicit ACE
    // would make the bound-tree witness ambiguous.
    let actual_runtime = inspect_root(runtime).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("login runtime scope: {error:?}")))?;
    if &actual_runtime.identity != runtime_identity || runtime.parent() != Some(home.path.as_path()) {
        return Err(OrchestrationError::AccessDenied);
    }
    let program_identity = AppContainerProfile::capture_program_identity(program)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "login program identity: {error}")))?;
    profile.grant_bound_tree(&home.path, &home.identity, true)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "login instance scope: {error}")))?;
    profile.grant_bound_program(program, &program_identity)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "login program scope: {error}")))?;
    profile.verify_bound_tree_grant(&home.path, &home.identity, true)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "login instance scope verification: {error}")))?;
    profile.verify_bound_program_grant(program, &program_identity)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!(
            "login program scope verification: {error}")))?;
    let actual_runtime = inspect_root(runtime).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("login runtime verification: {error:?}")))?;
    if &actual_runtime.identity != runtime_identity {
        return Err(OrchestrationError::AccessDenied);
    }
    Ok(())
}

impl<'root> ProductDatabase<'root> {
    fn prior_owner_login_custody(&self, command: &OwnerLoginCommand) -> Result<bool> {
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE operation_id=?1")?;
        query.bind_text(1, &owner_login_operation_id(command))?;
        let found = query.step_row()?;
        if found && query.step_row()? { return Err(OrchestrationError::OperationConflict); }
        Ok(found)
    }

    fn prior_login_state_request(&self, request: &V37Request) -> Result<Option<Vec<u8>>> {
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex,phase FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
        query.bind_text(1, &request.request_id)?;
        if !query.step_row()? { return Ok(None); }
        let fingerprint = query.column_text(0)?;
        let phase = query.column_text(1)?;
        if query.step_row()? { return Err(OrchestrationError::OperationConflict); }
        let fields = decode_framed_hex(&fingerprint)?;
        if !matches!(fields.len(), 6 | 7)
            || (fields[0].as_slice() != b"observe" && fields[0].as_slice() != b"read-login")
            || fields[1].as_slice() != request.raw_bytes.as_slice()
            || fields[2].as_slice() != request.target_id.as_bytes()
            || fields[3].as_slice() != request.expected_revision.to_string().as_bytes()
            || fields[4].as_slice() != b"login"
            || (fields.len() == 7 && fields[6].as_slice() != b"NO_REVISION_CHANGE")
        {
            return Err(OrchestrationError::OperationConflict);
        }
        if phase != "APPLIED" {
            return Ok(Some(encode_receipt(request, V37Status::Unknown,
                request.expected_revision, request.expected_revision, Default::default())));
        }
        let state = state_from_journal(&fields[5])
            .ok_or(OrchestrationError::OperationConflict)?;
        let next = if fields.len() == 6 {
            request.expected_revision.checked_add(1)
                .ok_or(OrchestrationError::OperationConflict)?
        } else { request.expected_revision };
        Ok(Some(encode_receipt(request, V37Status::Replayed,
            request.expected_revision, next, login_observation_result(state))))
    }

    fn record_unchanged_login_state(&mut self, request: &V37Request,
        state: NativeAccountState) -> Result<Vec<u8>> {
        let fingerprint = observation_hex(request, state, true);
        self.connection.execute("BEGIN IMMEDIATE")
            .map_err(|error| OrchestrationError::Atomic(error.into()))?;
        let result = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let row = self.read_registered_instance(&request.target_id)?
                .ok_or(OrchestrationError::AccessDenied)?;
            if row.revision != request.expected_revision || row.login_state != state.public_state() {
                return Err(OrchestrationError::OperationConflict);
            }
            let insert = Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'APPLIED')")?;
            insert.bind_text(1, &request.request_id)?;
            insert.bind_text(2, &fingerprint)?;
            insert.bind_text(3, &request.target_id)?;
            insert.step_done()?;
            Ok(())
        })();
        match result {
            Ok(()) => self.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause)?,
            Err(error) => {
                self.connection.execute("ROLLBACK")
                    .map_err(OrchestrationError::CommitUnknownWithCause)?;
                return Err(error);
            }
        }
        Ok(encode_receipt(request, V37Status::Applied,
            request.expected_revision, request.expected_revision,
            login_observation_result(state)))
    }

    /// Owner-private action ingress. The parent must call this only from the
    /// authenticated User pipe; this JSON shape is not a K-INSTANCE operation.
    pub(super) fn dispatch_owner_login_frame(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let command = owner_login_command(frame)?;
        if command.action == OwnerLoginAction::Refresh {
            if matches!(self.owner_login, Some(OwnerLoginSession::Active(_) |
                OwnerLoginSession::PendingAccount(_))) {
                return Err(OrchestrationError::OperationConflict);
            }
            let state = self.owner_login_account_state(&command)?;
            return Ok(owner_login_reply(&command, &state, ""));
        }
        if let Some(session) = &self.owner_login {
            if !same_owner_login(session, &command) {
                if matches!(session, OwnerLoginSession::Active(_) |
                    OwnerLoginSession::PendingAccount(_)) {
                    return Err(OrchestrationError::OperationConflict);
                }
                if command.action != OwnerLoginAction::Begin {
                    return Err(OrchestrationError::OperationConflict);
                }
                self.owner_login = None;
            }
        }
        if self.owner_login.is_none() && self.prior_owner_login_custody(&command)? {
            return Ok(owner_login_reply(&command, "UNKNOWN", ""));
        }
        match command.action {
            OwnerLoginAction::Begin => self.begin_owner_device_login(&command),
            OwnerLoginAction::Status => self.status_owner_device_login(&command),
            OwnerLoginAction::Cancel => self.cancel_owner_device_login(&command),
            OwnerLoginAction::Refresh => unreachable!("refresh handled before login custody"),
        }
    }

    fn settle_owner_login_preflight_error(&mut self, command: &OwnerLoginCommand,
        error: OrchestrationError) -> OrchestrationError {
        // No process has been prepared and no custody row exists on these
        // paths. Keep the original failure for the caller and same-request
        // status, while allowing a later explicit begin with a new request.
        self.owner_login = Some(OwnerLoginSession::Final {
            instance_id: command.instance_id.clone(),
            request_id: command.request_id.clone(),
            expected_revision: command.expected_revision,
            state: "UNKNOWN".into(),
            output: format!("{error:?}"),
        });
        error
    }

    fn preserve_owner_login_failure(&mut self, command: &OwnerLoginCommand,
        failure: AccountObservationFailure) -> OrchestrationError {
        if let Some(custody) = failure.pending {
            self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                instance_id:command.instance_id.clone(),request_id:command.request_id.clone(),
                expected_revision:command.expected_revision,output:format!("{:?}",failure.error),
                latest_error:None,custody,
            }));
            failure.error
        } else { self.settle_owner_login_preflight_error(command, failure.error) }
    }

    fn begin_owner_device_login(&mut self, command: &OwnerLoginCommand) -> Result<Vec<u8>> {
        if let Some(session) = &self.owner_login {
            return Ok(match session {
                OwnerLoginSession::Active(active) => owner_login_reply(command,
                    if active.halted { "UNKNOWN" } else { "PENDING" }, &active.output),
                OwnerLoginSession::PendingAccount(pending) =>
                    owner_login_reply(command, "UNKNOWN", &pending.output),
                OwnerLoginSession::Final { state, output, .. } =>
                    owner_login_final_reply(command, state, output),
            });
        }
        let current = self.user_instance_revision(&command.instance_id)
            .map_err(|error| self.settle_owner_login_preflight_error(command, error))?;
        if current != command.expected_revision {
            return Err(self.settle_owner_login_preflight_error(command,
                OrchestrationError::OperationConflict));
        }
        let launch = self.prepare_owner_codex_login(&command.instance_id)
            .map_err(|error| self.settle_owner_login_preflight_error(command, error))?;
        self.start_owner_device_login(command, launch, |custodian, prepared| custodian.activate(prepared))
    }

    fn start_owner_device_login(&mut self, command: &OwnerLoginCommand, launch: PreparedOwnerLogin,
        activate: impl FnOnce(&mut crate::process::ProcessCustodian, &PreparedCustody)
            -> std::result::Result<PreparedCustody, ProcessCustodyError>) -> Result<Vec<u8>> {
        let operation_id = owner_login_operation_id(command);
        let prepared = match self.process_custodian.prepare(&launch.login) {
            Ok(prepared) => prepared,
            Err(error) => return Err(self.preserve_owner_login_failure(command, account_prepare_failure(&launch, error))),
        };
        let custody = |proof: Option<NativeStopProof>, abort_prepared| PendingAccountCustody {
            operation_id:Some(operation_id.clone()),prepared:Some(prepared.clone()),
            runtime_home:Some(launch.runtime_home.clone()),runtime_identity:Some(launch.runtime_identity.clone()),
            proof,durable_revision:None,abort_prepared,frame:None,request:None,
        };
        if let Err(error) = authority::record_prepared_process(
            &mut self.connection, &operation_id, &prepared,
        ) {
            let aborted = self.process_custodian.abort_prepared(&prepared);
            let retained = aborted.is_err();
            let cleaned = if retained { None } else { Some(remove_owned_runtime(&launch.runtime_home, &launch.runtime_identity)) };
            return Err(self.preserve_owner_login_failure(command, AccountObservationFailure {
                error:OrchestrationError::V37StoreFailure(format!("owner login prepare record: {error:?}; abort: {aborted:?}; cleanup: {cleaned:?}")),
                pending:retained.then(|| custody(None,true)),
            }));
        }
        #[cfg(all(test, windows))]
        let _trace = directed_trace::before_activation(&prepared);
        if let Err(error) = activate(&mut self.process_custodian, &prepared) {
            let released = activation_was_aborted(&error,self.process_custodian.is_tombstoned(&prepared.ticket));
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            let cleaned = if released { Some(remove_owned_runtime(&launch.runtime_home,&launch.runtime_identity)) } else { None };
            return Err(self.preserve_owner_login_failure(command, AccountObservationFailure {
                error:OrchestrationError::V37StoreFailure(format!("owner login activate: {error:?}; unknown record: {unknown:?}; cleanup: {cleaned:?}")),
                pending:(!released).then(|| custody(None,true)),
            }));
        }
        if let Err(error) = authority::mark_process_active(
            &mut self.connection, &operation_id, &prepared,
        ) {
            let stop = self.process_custodian.stop(&prepared.ticket,
                StopBudgets::production(), || Ok(()));
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            let cause=OrchestrationError::V37StoreFailure(format!("owner login active record: {error:?}; stop: {stop:?}; unknown record: {unknown:?}"));
            return Err(self.preserve_owner_login_failure(command, AccountObservationFailure {
                error:cause,pending:Some(custody(stop.ok(),false)),
            }));
        }
        self.owner_login = Some(OwnerLoginSession::Active(ActiveOwnerLogin {
            instance_id: command.instance_id.clone(),
            request_id: command.request_id.clone(),
            expected_revision: command.expected_revision,
            operation_id,
            prepared,
            runtime_home: launch.runtime_home,
            runtime_identity: launch.runtime_identity,
            output: String::new(),
            halted: false,
        }));
        Ok(owner_login_reply(command, "PENDING", ""))
    }

    fn status_owner_device_login(&mut self, command: &OwnerLoginCommand) -> Result<Vec<u8>> {
        let session = self.owner_login.take()
            .ok_or(OrchestrationError::OperationConflict)?;
        let mut active = match session {
            OwnerLoginSession::Final { instance_id, request_id, expected_revision, state, output } => {
                let reply = owner_login_final_reply(command, &state, &output);
                self.owner_login = Some(OwnerLoginSession::Final {
                    instance_id, request_id, expected_revision, state, output,
                });
                return Ok(reply);
            }
            OwnerLoginSession::Active(active) => active,
            OwnerLoginSession::PendingAccount(pending) =>
                return self.progress_pending_account(command, pending),
        };
        if active.halted {
            let reply = owner_login_reply(command, "UNKNOWN", &active.output);
            self.owner_login = Some(OwnerLoginSession::Active(active));
            return Ok(reply);
        }
        let read = self.process_custodian.read_persistent_child_frame(
            &active.prepared.ticket, Duration::from_millis(250));
        match read {
            Ok(output) => {
                if output.custody() != &active.prepared {
                    active.halted = true;
                    self.owner_login = Some(OwnerLoginSession::Active(active));
                    return Err(OrchestrationError::AccessDenied);
                }
                if active.output.len().saturating_add(output.bytes().len()) > 65_536 {
                    let finished = self.finish_owner_device_login(command, active, true);
                    return match finished {
                        Ok(_) => Err(OrchestrationError::Invalid("owner login output limit")),
                        Err(error) => Err(error),
                    };
                }
                active.output.push_str(&String::from_utf8_lossy(output.bytes()));
                let reply = owner_login_reply(command, "PENDING", &active.output);
                self.owner_login = Some(OwnerLoginSession::Active(active));
                Ok(reply)
            }
            Err(error) if protocol_timed_out(&error) => {
                let exited = match self.process_custodian.active(&active.prepared.ticket) {
                    Some(process) => process.wait(Duration::ZERO),
                    None => Err(std::io::Error::new(std::io::ErrorKind::NotFound,
                        "owned login process absent")),
                };
                let exited = match exited {
                    Ok(exited) => exited,
                    Err(error) => {
                        active.halted = true;
                        self.owner_login = Some(OwnerLoginSession::Active(active));
                        return Err(OrchestrationError::Io(error));
                    }
                };
                if exited { self.finish_owner_device_login(command, active, false) }
                else {
                    let reply = owner_login_reply(command, "PENDING", &active.output);
                    self.owner_login = Some(OwnerLoginSession::Active(active));
                    Ok(reply)
                }
            }
            Err(error) if protocol_eof(&error) => {
                let exited = match self.process_custodian.active(&active.prepared.ticket) {
                    Some(process) => process.wait(Duration::ZERO),
                    None => Err(std::io::Error::new(std::io::ErrorKind::NotFound,
                        "owned login process absent")),
                };
                let exited = match exited {
                    Ok(exited) => exited,
                    Err(wait_error) => {
                        active.halted = true;
                        self.owner_login = Some(OwnerLoginSession::Active(active));
                        return Err(OrchestrationError::V37StoreFailure(format!(
                            "owner login output: {error:?}; wait: {wait_error}")));
                    }
                };
                if exited { self.finish_owner_device_login(command, active, false) }
                else {
                    let finished = self.finish_owner_device_login(command, active, false);
                    match finished {
                        Ok(_) => Err(OrchestrationError::Process(error)),
                        Err(stop_error) => Err(OrchestrationError::V37StoreFailure(format!(
                            "owner login stdout closed: {error:?}; stop: {stop_error:?}"))),
                    }
                }
            }
            Err(error) => {
                let finished = self.finish_owner_device_login(command, active, false);
                match finished {
                    Ok(_) => Err(OrchestrationError::Process(error)),
                    Err(stop_error) => Err(OrchestrationError::V37StoreFailure(format!(
                        "owner login output: {error:?}; stop: {stop_error:?}"))),
                }
            }
        }
    }

    fn cancel_owner_device_login(&mut self, command: &OwnerLoginCommand) -> Result<Vec<u8>> {
        let session = self.owner_login.take()
            .ok_or(OrchestrationError::OperationConflict)?;
        match session {
            OwnerLoginSession::Final { instance_id, request_id, expected_revision, state, output } => {
                let reply = owner_login_final_reply(command, &state, &output);
                self.owner_login = Some(OwnerLoginSession::Final {
                    instance_id, request_id, expected_revision, state, output,
                });
                Ok(reply)
            }
            OwnerLoginSession::Active(active) if active.halted => {
                let reply = owner_login_reply(command, "UNKNOWN", &active.output);
                self.owner_login = Some(OwnerLoginSession::Active(active));
                Ok(reply)
            }
            OwnerLoginSession::Active(active) =>
                self.finish_owner_device_login(command, active, true),
            OwnerLoginSession::PendingAccount(pending) =>
                self.progress_pending_account(command, pending),
        }
    }

    fn release_pending_account_custody(&mut self, custody: &mut PendingAccountCustody) -> Result<bool> {
        let Some(prepared) = custody.prepared.as_ref() else { return Ok(false); };
        if custody.abort_prepared {
            self.process_custodian.abort_prepared(prepared)?;
            return Ok(true);
        }
        let Some(operation_id) = custody.operation_id.as_deref() else { return Ok(false); };
        if custody.proof.is_none() {
            custody.proof = Some(self.process_custodian.stop(&prepared.ticket,
                StopBudgets::production(), || Ok(()))?);
        }
        let Some(proof) = custody.proof.as_ref() else { return Ok(false); };
        if custody.durable_revision.is_none() {
            custody.durable_revision = Some(authority::mark_process_stopped(
                &mut self.connection, operation_id, proof)?);
        }
        let Some(revision) = custody.durable_revision else { return Ok(false); };
        self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
            identity: prepared.identity.clone(), proof_hash: proof.proof_hash(),
            durable_revision: revision,
        })?;
        Ok(true)
    }

    fn progress_pending_account(&mut self, command: &OwnerLoginCommand,
        mut pending: PendingAccountRead) -> Result<Vec<u8>> {
        let released = match self.release_pending_account_custody(&mut pending.custody) {
            Ok(released) => released,
            Err(error) => { pending.latest_error = Some(format!("{error:?}")); false }
        };
        if !released {
            let output = match &pending.latest_error {
                Some(error) => format!("{}\nCLI process reconciliation: {error}", pending.output),
                None => pending.output.clone(),
            };
            let reply = owner_login_reply(command, "UNKNOWN", &output);
            self.owner_login = Some(OwnerLoginSession::PendingAccount(pending));
            return Ok(reply);
        }
        if let (Some(runtime), Some(identity)) = (&pending.custody.runtime_home,
            &pending.custody.runtime_identity) {
            if let Err(error) = remove_owned_runtime(runtime, identity) {
                pending.output.push_str(&format!("\naccount/read runtime cleanup: {error:?}"));
            }
        }
        let state = match (&pending.custody.request, &pending.custody.prepared,
            &pending.custody.frame) {
            (Some(request), Some(prepared), Some(frame)) => {
                match self.record_trusted_account_read(request, prepared, frame)
                    .and_then(|receipt| owner_login_state_from_receipt(&receipt)) {
                    Ok(state) => state,
                    Err(error) => {
                        pending.output.push_str(&format!("\naccount/read observation: {error:?}"));
                        "UNKNOWN".to_owned()
                    }
                }
            }
            _ => "UNKNOWN".to_owned(),
        };
        let reply = owner_login_final_reply(command, &state, &pending.output);
        self.owner_login = Some(OwnerLoginSession::Final {
            instance_id: pending.instance_id, request_id: pending.request_id,
            expected_revision: pending.expected_revision, state, output: pending.output,
        });
        Ok(reply)
    }

    fn finish_owner_device_login(&mut self, command: &OwnerLoginCommand,
        mut active: ActiveOwnerLogin, cancelled: bool) -> Result<Vec<u8>> {
        let stop = self.process_custodian.stop(&active.prepared.ticket,
            StopBudgets::production(), || Ok(()));
        let proof = match stop {
            Ok(proof) => proof,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &active.operation_id, &active.prepared);
                active.halted = true;
                self.owner_login = Some(OwnerLoginSession::Active(active));
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "owner login stop: {error:?}; unknown record: {unknown:?}")));
            }
        };
        // Capture the exact child's retained stderr before durable confirmation
        // releases its pipes. Exit failure is not an account-state observation.
        let login_failure = if !cancelled && proof.exit_code != Some(0) {
            let stderr = self.process_custodian.active(&active.prepared.ticket)
                .ok_or(OrchestrationError::AccessDenied)?.stderr_tail();
            Some(format!("owner login process exited: code={:?}; STDERR_TAIL: {stderr}",
                proof.exit_code))
        } else { None };
        let revision = match authority::mark_process_stopped(
            &mut self.connection, &active.operation_id, &proof,
        ) {
            Ok(revision) => revision,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &active.operation_id, &active.prepared);
                active.halted = true;
                self.owner_login = Some(OwnerLoginSession::Active(active));
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "owner login stop record: {error:?}; proof: {proof:?}; unknown record: {unknown:?}")));
            }
        };
        if let Err(error) = self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: active.prepared.ticket.clone(),
            custodian_nonce: active.prepared.custodian_nonce.clone(),
            identity: active.prepared.identity.clone(),
            proof_hash: proof.proof_hash(),
            durable_revision: revision,
        }) {
            active.halted = true;
            self.owner_login = Some(OwnerLoginSession::Active(active));
            return Err(error.into());
        }
        let cleanup = remove_owned_runtime(&active.runtime_home, &active.runtime_identity);
        let state_result = if cleanup.is_ok() && login_failure.is_none() {
            self.owner_login_account_state(command)
        } else {
            Ok("UNKNOWN".to_owned())
        };
        if let Some(OwnerLoginSession::PendingAccount(pending)) = &mut self.owner_login {
            if !active.output.is_empty() {
                pending.output = format!("{}\n{}", active.output, pending.output);
            }
            return Err(state_result.err().expect("pending account custody carries an error"));
        }
        let state = state_result.as_ref().cloned().unwrap_or_else(|_| "UNKNOWN".to_owned());
        if let Err(error) = &state_result {
            if !active.output.is_empty() { active.output.push('\n'); }
            active.output.push_str(&format!("automatic account/read failed: {error:?}"));
        }
        if let Some(failure) = &login_failure {
            if !active.output.is_empty() { active.output.push('\n'); }
            active.output.push_str(failure);
        }
        let reply = owner_login_final_reply(command, &state, &active.output);
        self.owner_login = Some(OwnerLoginSession::Final {
            instance_id: active.instance_id,
            request_id: active.request_id,
            expected_revision: active.expected_revision,
            state,
            output: active.output,
        });
        if let Some(failure) = login_failure {
            // Owner-private only: neither this diagnostic nor device codes are
            // copied into the public ledger or the service response channel.
            return Err(OrchestrationError::V37StoreFailure(format!(
                "{failure}; runtime cleanup: {cleanup:?}")));
        }
        if let Err(error) = cleanup { return Err(error); }
        if let Err(error) = state_result { return Err(error); }
        if cancelled { return Ok(reply); }
        Ok(reply)
    }

    fn owner_login_account_state(&mut self, command: &OwnerLoginCommand) -> Result<String> {
        let request_id = format!("{}-account-read", command.request_id);
        let raw_bytes = format!("owner-login-account-read:{}:{}",
            command.instance_id, command.request_id).into_bytes();
        let observation = V37Request {
            raw_bytes,
            family: "K-INSTANCE".into(),
            operation: "login-state".into(),
            request_id,
            target_id: command.instance_id.clone(),
            domain_id: "global".into(),
            expected_revision: command.expected_revision,
            payload: BTreeMap::new(),
        };
        let receipt = match self.dispatch_owner_login_observation_inner(&observation) {
            Ok(receipt) => receipt,
            Err(failure) => {
                if let Some(custody) = failure.pending {
                    self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                        instance_id: command.instance_id.clone(),
                        request_id: command.request_id.clone(),
                        expected_revision: command.expected_revision,
                        output: format!("{:?}", failure.error),
                        latest_error: None,
                        custody,
                    }));
                }
                return Err(failure.error);
            }
        };
        owner_login_state_from_receipt(&receipt)
    }

    /// Called only after private UserOriginProof/Owner admission by the parent.
    /// The exact pinned program and registered home are resolved again here.
    pub(super) fn prepare_owner_codex_login(&mut self, instance_id: &str) -> Result<PreparedOwnerLogin> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let row = self.read_registered_instance(instance_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if row.driver_id != "codex" || row.version != "0.149.0" {
            return Err(OrchestrationError::AccessDenied);
        }
        let source = self.registration_source(instance_id, "codex")?
            .ok_or(OrchestrationError::AccessDenied)?;
        if !self.registered_home_is_current(&source, instance_id)? {
            return Err(OrchestrationError::AccessDenied);
        }
        let home = instance::resolve_codex_instance_home(&self.connection, self.root, instance_id)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!(
                "login registered home: {error:?}")))?;
        checked_directory(&home.path)?;
        let program = instance::locate_pinned_program(&row.driver_id,
            &row.program_digest, &row.version)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!(
                "login pinned program: {error:?}")))?;
        let digest = row.program_digest.strip_prefix("sha256:")
            .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(OrchestrationError::AccessDenied)?;
        let profile_name = owner_login_profile_name(instance_id, &home.identity);
        let profile = AppContainerProfile::ensure_for_cli(&profile_name, true)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!(
                "login isolation profile: {error}")))?;
        let (runtime, runtime_identity) = runtime_home(&home.path)?;
        let scope = (|| -> Result<_> {
            let mut environment = clean_environment(&home.path, &runtime)?;
            grant_owner_login_scope(&profile, &home, &runtime, &runtime_identity, &program)?;
            let module = CompatModule::prepare(self.root, &home.path, &home.identity,
                &profile, &profile_name).map_err(|source|
                    OrchestrationError::V37StoreFailure(format!("login path compatibility: {source}")))?;
            module.extend_environment(&mut environment);
            Ok((environment, module))
        })();
        let (environment, module) = match scope {
            Ok(prepared) => prepared,
            Err(error) => {
                let cleanup = remove_owned_runtime(&runtime, &runtime_identity);
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "login isolation preparation: {error:?}; cleanup: {cleanup:?}")));
            }
        };
        let binding = NativeBinding {
            binary_digest_sha256: format!("sha256:{}", digest.to_ascii_lowercase()),
            profile_id: instance_id.to_owned(),
            domain_id: "global".to_owned(),
            generation: row.revision.to_string(),
        };
        let mut login = ProcessLaunch::new(program.clone());
        login.arguments = vec!["login".into(), "--device-auth".into()];
        login.current_directory = Some(runtime.clone());
        login.environment = Some(environment.clone());
        // Official device-auth text travels only through the Owner-private
        // stdout pipe. No credential bytes are sent to stdin.
        login.protocol_stdio = true;
        login.persistent_protocol_stdio = true;
        login.app_container_profile = Some(profile_name.clone());
        login.app_container_internet_client = true;
        login.app_container_cli_identity_services = true;
        login.path_compat = Some(module.clone());
        let mut account_read = ProcessLaunch::new(program);
        account_read.arguments = vec![
            "-c".into(), "features.memories=false".into(),
            "-c".into(), "memories.generate_memories=false".into(),
            "-c".into(), "memories.use_memories=false".into(),
            "app-server".into(),
        ];
        account_read.current_directory = Some(runtime.clone());
        account_read.environment = Some(environment);
        account_read.protocol_stdio = true;
        account_read.persistent_protocol_stdio = true;
        account_read.app_container_profile = Some(profile_name);
        account_read.app_container_internet_client = true;
        account_read.app_container_cli_identity_services = true;
        account_read.path_compat = Some(module);
        Ok(PreparedOwnerLogin {
            login: PrepareRequest { launch: login, binding: binding.clone() },
            account_read: PrepareRequest { launch: account_read, binding },
            runtime_home: runtime,
            runtime_identity,
        })
    }

    /// The parent supplies a frame read from this exact native process after
    /// the fixed initialize/initialized/account-read sequence. It must first
    /// persist the process lifecycle and stop fact through ProcessCustodian.
    /// This method does not accept any account value or caller-provided state.
    pub(super) fn record_trusted_account_read(
        &mut self,
        request: &V37Request,
        prepared: &PreparedCustody,
        frame: &OriginBoundFrame,
    ) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let current = self.user_instance_revision(&request.target_id)?;
        if request.family != "K-INSTANCE" || request.operation != "login-state"
            || request.domain_id != "global" || !request.payload.is_empty()
        {
            return Ok(encode_receipt(request, V37Status::Denied, current, current,
                Default::default()));
        }
        if request.expected_revision != current {
            return Ok(encode_receipt(request, V37Status::Stale, current, current,
                Default::default()));
        }
        if frame.custody() != prepared
            || prepared.binding.profile_id != request.target_id
            || prepared.binding.domain_id != "global"
            || prepared.binding.generation != current.to_string()
        {
            return Ok(encode_receipt(request, V37Status::Denied, current, current,
                Default::default()));
        }
        let row = self.read_registered_instance(&request.target_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if row.driver_id != "codex"
            || row.program_digest != prepared.binding.binary_digest_sha256
        {
            return Ok(encode_receipt(request, V37Status::Denied, current, current,
                Default::default()));
        }
        let source = self.registration_source(&request.target_id, "codex")?
            .ok_or(OrchestrationError::AccessDenied)?;
        if !self.registered_home_is_current(&source, &request.target_id)? {
            return Ok(encode_receipt(request, V37Status::Unknown, current, current,
                Default::default()));
        }
        let state = parse_account_read(frame.bytes());
        if row.login_state == state.public_state()
            && (state == NativeAccountState::Unknown
                || self.current_login_observation(&request.target_id, current,
                    &row.login_state)?)
        {
            return self.record_unchanged_login_state(request, state);
        }
        let observation = ObservationRequest {
            request_id: &request.request_id,
            request_bytes: &request.raw_bytes,
            instance_id: &request.target_id,
            expected_revision: i64::try_from(current)
                .map_err(|error| OrchestrationError::V37StoreFailure(format!(
                    "login revision overflow: {error}")))?,
            observation: state.durable(),
        };
        let (status, next) = match instance::record_observation(
            &mut self.connection, self.root, &observation,
        ) {
            Ok(RegistrationDisposition::Applied) => (V37Status::Applied, current + 1),
            Ok(RegistrationDisposition::Replayed) => (V37Status::Replayed, current),
            Err(RegistryError::InstanceConflict) => (V37Status::Stale, current),
            Err(RegistryError::RequestConflict) => (V37Status::Conflict, current),
            Err(RegistryError::Invalid(_)) => (V37Status::Denied, current),
            Err(error) => return Err(OrchestrationError::V37StoreFailure(format!(
                "native login observation: {error:?}"))),
        };
        Ok(encode_receipt(request, status, current, next,
            login_observation_result(state)))
    }

    /// Trusted User action only. The parent dispatches this after live
    /// UserOriginProof; it never accepts a process path, argv, or account
    /// contents from the wire. No model request is made.
    pub(super) fn dispatch_owner_login_observation(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        if matches!(self.owner_login, Some(OwnerLoginSession::Active(_) |
            OwnerLoginSession::PendingAccount(_))) {
            return Err(OrchestrationError::OperationConflict);
        }
        match self.dispatch_owner_login_observation_inner(request) {
            Ok(receipt) => Ok(receipt),
            Err(failure) => {
                if let Some(custody) = failure.pending {
                    self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                        instance_id: request.target_id.clone(),
                        request_id: request.request_id.clone(),
                        expected_revision: request.expected_revision,
                        output: format!("{:?}", failure.error),
                        latest_error: None,
                        custody,
                    }));
                }
                Err(failure.error)
            }
        }
    }

    fn dispatch_owner_login_observation_inner(&mut self, request: &V37Request)
        -> std::result::Result<Vec<u8>, AccountObservationFailure> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let current = self.user_instance_revision(&request.target_id)?;
        if request.family != "K-INSTANCE" || request.operation != "login-state"
            || request.domain_id != "global" || !request.payload.is_empty()
        {
            return Ok(encode_receipt(request, V37Status::Denied, current, current,
                Default::default()));
        }
        if let Some(prior) = self.prior_login_state_request(request)? {
            return Ok(prior);
        }
        if request.expected_revision != current {
            return Ok(encode_receipt(request, V37Status::Stale, current, current,
                Default::default()));
        }
        // This resolves the pin/home and scopes only; no process is prepared.
        let prepared_login = self.prepare_owner_codex_login(&request.target_id)?;
        let operation_hash = crate::store::digest::sha256_hex(&request.raw_bytes);
        let operation_id = format!("login-observe-{}", &operation_hash[..40]);
        let prepared = match self.process_custodian.prepare(&prepared_login.account_read) {
            Ok(prepared) => prepared,
            Err(error) => return Err(account_prepare_failure(&prepared_login, error)),
        };
        let pending = |proof: Option<NativeStopProof>, durable_revision: Option<u64>, abort_prepared| {
            PendingAccountCustody {
                operation_id: Some(operation_id.clone()), prepared: Some(prepared.clone()),
                runtime_home: Some(prepared_login.runtime_home.clone()),
                runtime_identity: Some(prepared_login.runtime_identity.clone()),
                proof, durable_revision, abort_prepared, frame: None,
                request: Some(V37Request {
                    raw_bytes: request.raw_bytes.clone(), family: request.family.clone(),
                    operation: request.operation.clone(), request_id: request.request_id.clone(),
                    target_id: request.target_id.clone(), domain_id: request.domain_id.clone(),
                    // This private operation rejects every nonempty payload above.
                    expected_revision: request.expected_revision, payload: BTreeMap::new(),
                }),
            }
        };
        if let Err(error) = authority::record_prepared_process(
            &mut self.connection, &operation_id, &prepared,
        ) {
            // The child is still suspended. Abort it if the durable PREPARED
            // row could not be established; preserve both original errors.
            if let Err(abort) = self.process_custodian.abort_prepared(&prepared) {
                return Err(AccountObservationFailure { error: OrchestrationError::V37StoreFailure(format!(
                    "login prepare record: {error:?}; abort: {abort:?}")),
                    pending: Some(pending(None, None, true)) });
            }
            remove_owned_runtime(&prepared_login.runtime_home,
                &prepared_login.runtime_identity)?;
            return Err(error.into());
        }
        #[cfg(all(test, windows))]
        let _trace = directed_trace::before_activation(&prepared);
        if let Err(error) = self.process_custodian.activate(&prepared) {
            let released = activation_was_aborted(&error, self.process_custodian.is_tombstoned(&prepared.ticket));
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            let error = match unknown {
                Ok(()) => error.into(),
                Err(record_error) => OrchestrationError::V37StoreFailure(format!(
                    "login activate: {error:?}; unknown record: {record_error:?}")),
            };
            if released {
                let cleanup = remove_owned_runtime(&prepared_login.runtime_home, &prepared_login.runtime_identity);
                return Err(AccountObservationFailure { error: OrchestrationError::V37StoreFailure(
                    format!("login activation aborted: {error:?}; cleanup: {cleanup:?}")), pending: None });
            }
            return Err(AccountObservationFailure { error, pending: Some(pending(None, None, true)) });
        }
        if let Err(error) = authority::mark_process_active(
            &mut self.connection, &operation_id, &prepared,
        ) {
            let stop = self.process_custodian.stop(&prepared.ticket,
                StopBudgets::production(), || Ok(()));
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            return Err(AccountObservationFailure { error: OrchestrationError::V37StoreFailure(format!(
                "login active record: {error:?}; stop: {stop:?}; unknown record: {unknown:?}")),
                pending: Some(pending(stop.ok(), None, false)) });
        }
        let execution = self.observe_account_via_active_cli(&prepared, &prepared_login.runtime_home);
        let close = self.process_custodian.close_child_input(&prepared.ticket)
            .map_err(|error| format!("account/read stdin close: {error}"));
        let stop = self.process_custodian.stop(&prepared.ticket,
            StopBudgets::production(), move || close);
        let proof = match stop {
            Ok(proof) => proof,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &operation_id, &prepared);
                let cause = OrchestrationError::V37StoreFailure(format!(
                    "login stop: {error:?}; protocol: {:?}; unknown record: {unknown:?}",
                    execution.as_ref().err()));
                let mut custody = pending(None, None, false);
                custody.frame = execution.ok();
                return Err(AccountObservationFailure { error: cause, pending: Some(custody) });
            }
        };
        let revision = match authority::mark_process_stopped(
            &mut self.connection, &operation_id, &proof,
        ) {
            Ok(revision) => revision,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &operation_id, &prepared);
                let cause = OrchestrationError::V37StoreFailure(format!(
                    "login stop record: {error:?}; proof: {proof:?}; protocol: {:?}; unknown record: {unknown:?}",
                    execution.as_ref().err()));
                let mut custody = pending(Some(proof), None, false);
                custody.frame = execution.ok();
                return Err(AccountObservationFailure { error: cause, pending: Some(custody) });
            }
        };
        if let Err(error) = self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: prepared.ticket.clone(),
            custodian_nonce: prepared.custodian_nonce.clone(),
            identity: prepared.identity.clone(),
            proof_hash: proof.proof_hash(),
            durable_revision: revision,
        }) {
            let mut custody = pending(Some(proof), Some(revision), false);
            custody.frame = execution.ok();
            return Err(AccountObservationFailure { error: error.into(), pending: Some(custody) });
        }
        remove_owned_runtime(&prepared_login.runtime_home,
            &prepared_login.runtime_identity)?;
        let frame = execution?;
        self.record_trusted_account_read(request, &prepared, &frame).map_err(Into::into)
    }

    fn observe_account_via_active_cli(&self, prepared: &PreparedCustody, cwd: &Path)
        -> Result<OriginBoundFrame> {
        use crate::store::session_transport::codex_rpc::{self, Command, RpcId, Reply};
        let process = self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::Invalid("login process absent"))?;
        process.write_persistent_frame(INITIALIZE)
            .map_err(|error| OrchestrationError::Process(
                self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                    ProcessCustodyError::ProtocolPipe(error))))?;
        let initialize = self.read_rpc_response(prepared, "1")?;
        let init_command = Command::Initialize { client_version: "0.1.0".into() };
        let init_id = RpcId::client(1).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native initialize ID: {error:?}")))?;
        match codex_rpc::decode(initialize.bytes(), Some((&init_id, &init_command))) {
            Ok(Reply::Initialized { .. }) => (),
            result => return Err(OrchestrationError::V37StoreFailure(format!("native initialize observation: {result:?}"))),
        }
        let process = self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::Invalid("login process absent"))?;
        process.write_persistent_frame(INITIALIZED)
            .map_err(|error| OrchestrationError::Process(
                self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                    ProcessCustodyError::ProtocolPipe(error))))?;
        // Measure the fixed CLI's effective memory settings in this actual
        // native-owned home. This is a config read, not a model invocation.
        let config_command = Command::ConfigRead { cwd: cwd.to_string_lossy().into_owned() };
        let config_id = RpcId::client(3).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native config ID: {error:?}")))?;
        let config_bytes = config_command.encode(Some(&config_id)).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native config request: {error:?}")))?;
        process.write_persistent_frame(&config_bytes).map_err(|error|
            OrchestrationError::Process(self.process_custodian.protocol_error_with_stderr(
                &prepared.ticket, ProcessCustodyError::ProtocolPipe(error))))?;
        let config = self.read_rpc_response(prepared, "3")?;
        match codex_rpc::decode(config.bytes(), Some((&config_id, &config_command))) {
            Ok(Reply::MemoryOff { .. }) => (),
            result => return Err(OrchestrationError::V37StoreFailure(format!("native effective memory observation: {result:?}"))),
        }
        process.write_persistent_frame(ACCOUNT_READ)
            .map_err(|error| OrchestrationError::Process(
                self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                    ProcessCustodyError::ProtocolPipe(error))))?;
        self.read_rpc_response(prepared, "2")
    }

    fn read_rpc_response(&self, prepared: &PreparedCustody, expected_id: &str)
        -> Result<OriginBoundFrame> {
        let started = Instant::now();
        for _ in 0..MAX_RPC_FRAMES {
            let remaining = RPC_DEADLINE.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Err(OrchestrationError::Invalid("account RPC deadline"));
            }
            let frame = self.process_custodian
                .read_persistent_child_frame(&prepared.ticket, remaining)?;
            if frame.custody() != prepared {
                return Err(OrchestrationError::AccessDenied);
            }
            match rpc_frame_identity(frame.bytes(), expected_id) {
                RpcIdentity::Notification => continue,
                RpcIdentity::Expected => return Ok(frame),
                RpcIdentity::RemoteError => return Err(OrchestrationError::Process(
                    self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                        ProcessCustodyError::ProtocolPipe(std::io::Error::new(
                            std::io::ErrorKind::Other, format!("account RPC native error: {}",
                                String::from_utf8_lossy(&frame.bytes()[frame.bytes().len().saturating_sub(4096)..]))))))),
                RpcIdentity::Wrong => return Err(OrchestrationError::Process(
                    self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                        ProcessCustodyError::ProtocolPipe(std::io::Error::new(
                            std::io::ErrorKind::InvalidData, "account RPC response identity"))))),
            }
        }
        Err(OrchestrationError::Invalid("account RPC frame limit"))
    }
}

/// Fixed account/read sequence. The caller sends these only through the
/// active ProcessCustodian pipe and accepts frames from that same custody.
pub(super) const INITIALIZE: &[u8] = b"{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"gogoke\",\"version\":\"0.1.0\"},\"capabilities\":{}}}\n";
pub(super) const INITIALIZED: &[u8] = b"{\"method\":\"initialized\",\"params\":{}}\n";
pub(super) const ACCOUNT_READ: &[u8] = b"{\"id\":2,\"method\":\"account/read\",\"params\":{\"refreshToken\":false}}\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RpcIdentity { Notification, Expected, RemoteError, Wrong }

fn rpc_frame_identity(frame: &[u8], expected_id: &str) -> RpcIdentity {
    if frame.len() > 65_536 { return RpcIdentity::Wrong; }
    let Ok(text) = std::str::from_utf8(frame) else { return RpcIdentity::Wrong; };
    let Ok(value) = Parser::parse(text.trim_end()) else { return RpcIdentity::Wrong; };
    let Some(mut object) = object(value) else { return RpcIdentity::Wrong; };
    match object.remove(&JsonString::from_str("id")) {
        Some(Json::Number(id)) if id == expected_id => {
            if object.contains_key(&JsonString::from_str("error")) {
                RpcIdentity::RemoteError
            } else if object.contains_key(&JsonString::from_str("result")) {
                RpcIdentity::Expected
            } else {
                RpcIdentity::Wrong
            }
        }
        Some(_) => RpcIdentity::Wrong,
        None if matches!(object.remove(&JsonString::from_str("method")),
            Some(Json::String(_))) => RpcIdentity::Notification,
        None => RpcIdentity::Wrong,
    }
}

fn object(value: Json) -> Option<BTreeMap<JsonString, Json>> {
    match value { Json::Object(fields) => Some(fields), _ => None }
}

fn owner_login_state_from_receipt(receipt: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(receipt).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("owner login receipt UTF8: {error}")))?;
    let value = Parser::parse(text).map_err(OrchestrationError::Atomic)?;
    let mut value = object(value).ok_or(OrchestrationError::Invalid("owner login receipt object"))?;
    let result = value.remove(&JsonString::from_str("result"))
        .ok_or(OrchestrationError::Invalid("owner login receipt result"))?;
    let mut result = object(result).ok_or(OrchestrationError::Invalid("owner login result object"))?;
    Ok(match result.remove(&JsonString::from_str("state")) {
        Some(Json::String(state)) => match state.to_well_formed_string().as_deref() {
            Some("LOGGED_IN") => "LOGGED_IN".into(),
            Some("LOGGED_OUT") => "LOGGED_OUT".into(),
            _ => "UNKNOWN".into(),
        },
        _ => "UNKNOWN".into(),
    })
}

/// Parse only account existence. All account identifiers and raw JSON are
/// discarded before any public receipt or durable record is constructed.
pub(super) fn parse_account_read(frame: &[u8]) -> NativeAccountState {
    if frame.len() > 65_536 { return NativeAccountState::Unknown; }
    let Ok(text) = std::str::from_utf8(frame) else { return NativeAccountState::Unknown; };
    let Ok(parsed) = Parser::parse(text.trim_end()) else { return NativeAccountState::Unknown; };
    let Some(mut envelope) = object(parsed) else { return NativeAccountState::Unknown; };
    let Some(Json::Number(id)) = envelope.remove(&JsonString::from_str("id")) else {
        return NativeAccountState::Unknown;
    };
    if id != "2" || envelope.contains_key(&JsonString::from_str("error")) {
        return NativeAccountState::Unknown;
    }
    let Some(result) = envelope.remove(&JsonString::from_str("result")) else {
        return NativeAccountState::Unknown;
    };
    let Some(mut result) = object(result) else { return NativeAccountState::Unknown; };
    match result.remove(&JsonString::from_str("account")) {
        Some(Json::Object(mut account)) => match account.remove(&JsonString::from_str("type")) {
            Some(Json::String(kind)) if matches!(kind.to_well_formed_string().as_deref(),
                Some("chatgpt") | Some("apiKey")) => NativeAccountState::CredentialPresent,
            _ => NativeAccountState::Unknown,
        },
        Some(Json::Null) => match result.remove(&JsonString::from_str("requiresOpenaiAuth")) {
            Some(Json::Bool(true)) => NativeAccountState::LoggedOut,
            _ => NativeAccountState::Unknown,
        },
        _ => NativeAccountState::Unknown,
    }
}

pub(super) fn login_observation_result(state: NativeAccountState) -> BTreeMap<JsonString, Json> {
    BTreeMap::from([
        (JsonString::from_str("state"), Json::String(JsonString::from_str(state.public_state()))),
        (JsonString::from_str("evidence"), Json::String(JsonString::from_str(state.evidence()))),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::same_open::route_b_test_guard;
    use crate::store::session_transport::{decode_receipt, decode_request};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn account_read_reports_existence_without_account_data_or_auth_validity() {
        let present = br#"{"id":2,"result":{"account":{"type":"chatgpt","email":"private@example.test"},"requiresOpenaiAuth":true}}"#;
        assert_eq!(parse_account_read(present), NativeAccountState::CredentialPresent);
        let result = login_observation_result(parse_account_read(present));
        assert_eq!(result.get(&JsonString::from_str("state")).map(Json::canonical),
            Some("\"LOGGED_IN\"".to_owned()));
        assert!(!Json::Object(result).canonical().contains("private@example.test"));
        assert_eq!(parse_account_read(br#"{"id":2,"result":{"account":null,"requiresOpenaiAuth":true}}"#),
            NativeAccountState::LoggedOut);
        assert_eq!(parse_account_read(br#"{"id":2,"result":{"account":null,"requiresOpenaiAuth":false}}"#),
            NativeAccountState::Unknown);
        assert_eq!(parse_account_read(br#"{"id":2,"error":{"message":"secret"}}"#),
            NativeAccountState::Unknown);
    }

    #[test]
    fn account_read_does_not_accept_other_rpc_or_unrecognized_account_type() {
        assert_eq!(parse_account_read(br#"{"id":1,"result":{"account":{"type":"chatgpt"}}}"#),
            NativeAccountState::Unknown);
        assert_eq!(parse_account_read(br#"{"id":2,"result":{"account":{"type":"other"}}}"#),
            NativeAccountState::Unknown);
        assert_eq!(parse_account_read(br#"{"id":2,"result":{"account":{},"requiresOpenaiAuth":true}}"#),
            NativeAccountState::Unknown);
    }

    #[test]
    fn rpc_sequence_filters_notifications_but_rejects_wrong_or_failed_response() {
        assert_eq!(rpc_frame_identity(br#"{"method":"account/updated","params":{}}"#, "2"),
            RpcIdentity::Notification);
        assert_eq!(rpc_frame_identity(br#"{"id":1,"result":{}}"#, "1"),
            RpcIdentity::Expected);
        assert_eq!(rpc_frame_identity(br#"{"id":1,"result":{}}"#, "2"),
            RpcIdentity::Wrong);
        assert_eq!(rpc_frame_identity(br#"{"id":2,"error":{"message":"private"}}"#, "2"),
            RpcIdentity::RemoteError);
        assert_eq!(rpc_frame_identity(br#"{"id":2,"method":"server/request"}"#, "2"),
            RpcIdentity::Wrong);
    }

    fn request(operation: &str, request_id: &str, expected: u64,
        payload: &str) -> V37Request {
        let raw = format!(
            "{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-INSTANCE\",\"operation\":\"{operation}\",\"requestId\":\"{request_id}\",\"targetId\":\"instanceA\",\"domainId\":\"global\",\"expectedRevision\":\"{expected}\",\"payload\":{payload}}}"
        );
        decode_request(raw.as_bytes()).unwrap()
    }

    fn scalar(product: &ProductDatabase<'_>, sql: &str) -> String {
        let statement = Statement::prepare(product.connection.as_ptr(), sql).unwrap();
        assert!(statement.step_row().unwrap());
        let value = statement.column_text(0).unwrap();
        assert!(!statement.step_row().unwrap());
        value
    }

    #[test]
    fn owner_instance_list_reads_only_registered_native_state_and_rejects_other_frames() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-instance-list-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        for id in ["instanceA", "instanceB", "instanceC"] {
            let raw = format!(
                "{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-INSTANCE\",\"operation\":\"register\",\"requestId\":\"register{id}\",\"targetId\":\"{id}\",\"domainId\":\"global\",\"expectedRevision\":\"0\",\"payload\":{{\"driverId\":\"codex\"}}}}"
            );
            let register = decode_request(raw.as_bytes()).unwrap();
            assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
                V37Status::Applied);
        }
        const LIST: &[u8] = br#"{"schema":"gogoke.37.instance-list.v1"}"#;
        assert!(is_owner_instance_list_frame(LIST));
        let other = br#"{"schema":"gogoke.37.operations.v1","family":"K-SEAT","operation":"tune","requestId":"a","targetId":"b","domainId":"g","expectedRevision":"1","payload":{"setting":"instruction","value":"gogoke.37.instance-list.v1 gogoke.37.owner-login.v1"}}"#;
        assert!(other.len() <= MAX_INSTANCE_LIST_FRAME);
        assert!(decode_request(other).is_ok(), "ordinary K-SEAT envelope is valid");
        assert!(!is_owner_instance_list_frame(other));
        assert!(!is_owner_login_frame(other));
        let registered = product.dispatch_owner_instance_list_frame(LIST).unwrap();
        assert_eq!(registered, br#"{"instances":[{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceA","loginState":"UNKNOWN","revision":"1","version":"0.149.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceB","loginState":"UNKNOWN","revision":"1","version":"0.149.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceC","loginState":"UNKNOWN","revision":"1","version":"0.149.0"}],"schema":"gogoke.37.instance-list.v1"}"#,
            "registration's stored UNKNOWN install state must resolve from the actual pinned program");
        for (id, observation, request_id) in [
            ("instanceA", InstanceObservation::LoggedOut, "logoutA"),
            ("instanceB", InstanceObservation::LoggedIn, "loginB"),
        ] {
            instance::record_observation(&mut product.connection, &root,
                &ObservationRequest { request_id, request_bytes: request_id.as_bytes(),
                    instance_id: id, expected_revision: 1, observation }).unwrap();
        }
        let observed = product.dispatch_owner_instance_list_frame(LIST).unwrap();
        assert_eq!(observed, br#"{"instances":[{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceA","loginState":"LOGGED_OUT","revision":"2","version":"0.149.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceB","loginState":"LOGGED_IN","revision":"2","version":"0.149.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceC","loginState":"UNKNOWN","revision":"1","version":"0.149.0"}],"schema":"gogoke.37.instance-list.v1"}"#);
        for invalid in [
            &br#"{"schema":"gogoke.37.instance-list.v1","extra":true}"#[..],
            &br#"{"schema":"gogoke.37.instance-list.v2"}"#[..],
            &br#"{"schema":"gogoke.37.instance-list.v1","schema":"gogoke.37.instance-list.v1"}"#[..],
        ] {
            assert!(product.dispatch_owner_instance_list_frame(invalid).is_err(),
                "only the exact private list frame is accepted");
        }
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_login_preflight_errors_settle_original_request_without_process_custody() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-login-preflight-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let stale_begin = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"preflightStale","expectedRevision":2}"#;
        let stale_status = br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"preflightStale","expectedRevision":2}"#;
        let stale_error = product.dispatch_owner_login_frame(stale_begin).unwrap_err();
        assert!(matches!(&stale_error, OrchestrationError::OperationConflict));
        let stale_reply = String::from_utf8(product.dispatch_owner_login_frame(stale_status).unwrap()).unwrap();
        assert!(stale_reply.contains("\"state\":\"UNKNOWN\""));
        assert!(stale_reply.contains("\"settled\":true"));
        assert!(stale_reply.contains(&format!("{stale_error:?}")));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "0");

        // The production table is STRICT INTEGER, so malformed revision text
        // cannot be written. Temporarily hide it to exercise an actual SQL
        // failure at user_instance_revision before any process preparation.
        product.connection.execute(
            "ALTER TABLE main.gogoke_v37_instances RENAME TO gogoke_v37_instances_unavailable").unwrap();
        let sql_begin = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"preflightSql","expectedRevision":1}"#;
        let sql_status = br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"preflightSql","expectedRevision":1}"#;
        let sql_error = product.dispatch_owner_login_frame(sql_begin).unwrap_err();
        assert!(format!("{sql_error:?}").contains("gogoke_v37_instances"));
        let sql_reply = String::from_utf8(product.dispatch_owner_login_frame(sql_status).unwrap()).unwrap();
        assert!(sql_reply.contains("\"state\":\"UNKNOWN\""));
        assert!(sql_reply.contains("\"settled\":true"));
        let mut sql_fields = object(Parser::parse(&sql_reply).unwrap()).unwrap();
        let Some(Json::String(sql_output)) = sql_fields.remove(&JsonString::from_str("output")) else {
            panic!("settled status must preserve the original SQL failure");
        };
        assert_eq!(sql_output.to_well_formed_string().unwrap(), format!("{sql_error:?}"));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "0");
        product.connection.execute(
            "ALTER TABLE main.gogoke_v37_instances_unavailable RENAME TO gogoke_v37_instances").unwrap();

        // A fresh request after Final reaches preparation. A mismatched test
        // pin fails before process preparation, without changing the real CLI.
        let update = Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instances SET version='0.148.0' WHERE instance_id='instanceA'").unwrap();
        update.step_done().unwrap();
        drop(update);
        for request_id in ["preflightPinA", "preflightPinB"] {
            let begin = format!("{{\"schema\":\"gogoke.37.owner-login.v1\",\"action\":\"begin\",\"instanceId\":\"instanceA\",\"requestId\":\"{request_id}\",\"expectedRevision\":1}}");
            let status = format!("{{\"schema\":\"gogoke.37.owner-login.v1\",\"action\":\"status\",\"instanceId\":\"instanceA\",\"requestId\":\"{request_id}\",\"expectedRevision\":1}}");
            let error = product.dispatch_owner_login_frame(begin.as_bytes()).unwrap_err();
            assert!(matches!(&error, OrchestrationError::AccessDenied));
            let reply = String::from_utf8(product.dispatch_owner_login_frame(status.as_bytes()).unwrap()).unwrap();
            assert!(reply.contains("\"state\":\"UNKNOWN\""));
            assert!(reply.contains("\"settled\":true"));
            assert!(reply.contains(&format!("{error:?}")));
            assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "0");
        }
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pinned_codex_owner_login_failure_preserves_exit_and_stderr_without_account_read() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-login-error-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity } =
            product.prepare_owner_codex_login("instanceA").unwrap();
        // The actual pinned CLI's parser emits stderr and exits 2 before any
        // device-auth request. No credentials, provider request or fake binary.
        login.launch.arguments = vec!["login".into(), "--gogoke-invalid-login-control".into()];
        let prepared = product.process_custodian.prepare(&login).unwrap();
        drop(login);
        drop(account_read);
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"ownerFailA","expectedRevision":1}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        assert!(product.process_custodian.active(&prepared.ticket).unwrap()
            .wait(Duration::from_secs(15)).unwrap(), "pinned CLI parser control did not exit");
        product.owner_login = Some(OwnerLoginSession::Active(ActiveOwnerLogin {
            instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
            expected_revision: command.expected_revision, operation_id,
            prepared, runtime_home: runtime_home.clone(), runtime_identity,
            output: String::new(), halted: false,
        }));
        let active = String::from_utf8(product.begin_owner_device_login(&command).unwrap()).unwrap();
        assert!(active.contains("\"state\":\"PENDING\""));
        assert!(active.contains("\"settled\":false"), "active login must retain its request");
        if let Some(OwnerLoginSession::Active(active)) = &mut product.owner_login {
            active.halted = true;
        }
        let halted = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(halted.contains("\"state\":\"UNKNOWN\""));
        assert!(halted.contains("\"settled\":false"), "halted active custody is not final");
        if let Some(OwnerLoginSession::Active(active)) = &mut product.owner_login {
            active.halted = false;
        }
        let error = product.status_owner_device_login(&command).unwrap_err();
        let diagnostic = format!("{error:?}");
        assert!(diagnostic.contains("code=Some(2)"), "actual exit code was not preserved");
        assert!(diagnostic.contains("STDERR_TAIL:"));
        assert!(diagnostic.contains("unexpected argument"), "actual CLI stderr was not preserved");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "1",
            "a failed login must not start a second CLI for account/read");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1");
        assert!(!runtime_home.exists(), "owned login runtime must still be cleaned");
        let replay = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(replay.contains("\"state\":\"UNKNOWN\""));
        assert!(replay.contains("\"settled\":true"), "final CLI failure must be distinguishable from halted active custody");
        assert!(replay.contains("unexpected argument"));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "1");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_first_cli_factory_failures_keep_original_results_and_custody() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-first-fail-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root,&path.join("state.sqlite")).unwrap();
        let register = request("register","registerA",0,r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,V37Status::Applied);
        for kind in ["digest","prepared","resume","active"] {
            product.owner_login = None;
            let frame = format!("{{\"schema\":\"gogoke.37.owner-login.v1\",\"action\":\"begin\",\"instanceId\":\"instanceA\",\"requestId\":\"first_{kind}\",\"expectedRevision\":1}}");
            let command = owner_login_command(frame.as_bytes()).unwrap();
            let mut launch = product.prepare_owner_codex_login("instanceA").unwrap();
            launch.login.launch.arguments = vec!["--version".into()];
            if kind == "digest" { launch.login.binding.binary_digest_sha256 = format!("sha256:{}","0".repeat(64)); }
            if kind == "prepared" {
                product.connection.execute("CREATE TRIGGER fail_first_prepare BEFORE INSERT ON gogoke_coordination_process_custody WHEN NEW.operation_id LIKE 'owner-login-%' BEGIN SELECT RAISE(ABORT,'controlled first prepare record failure'); END").unwrap();
            }
            if kind == "active" {
                product.connection.execute("CREATE TRIGGER fail_first_active BEFORE UPDATE OF state ON gogoke_coordination_process_custody WHEN NEW.state='ACTIVE' AND NEW.operation_id LIKE 'owner-login-%' BEGIN SELECT RAISE(ABORT,'controlled first active record failure'); END").unwrap();
            }
            let error = if kind == "resume" {
                product.start_owner_device_login(&command,launch,|custodian,prepared|custodian.activate_with_failed_resume_for_test(prepared)).unwrap_err()
            } else {
                product.start_owner_device_login(&command,launch,|custodian,prepared|custodian.activate(prepared)).unwrap_err()
            };
            let cause = format!("{error:?}");
            let status = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
            assert!(status.contains("\"settled\":true"),"{kind}: a proven released child must settle the original request");
            assert!(status.contains("\"state\":\"UNKNOWN\""));
            let fields = object(Parser::parse(&status).unwrap()).unwrap();
            let Json::String(output) = &fields[&JsonString::from_str("output")] else { panic!("retained output") };
            assert!(output.to_well_formed_string().unwrap().contains(&cause),"{kind}: original error retained after the first Err");
            if kind == "prepared" { product.connection.execute("DROP TRIGGER fail_first_prepare").unwrap(); }
            if kind == "active" { product.connection.execute("DROP TRIGGER fail_first_active").unwrap(); }
        }
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_account_preparation_distinguishes_confirmed_abort_from_unconfirmed_custody() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-account-prepare-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status, V37Status::Applied);
        let mut launch = product.prepare_owner_codex_login("instanceA").unwrap();
        // A wrong expected digest exercises the actual factory's pre-launch
        // refusal; the official CLI bytes are unchanged.
        launch.account_read.binding.binary_digest_sha256 = format!("sha256:{}", "0".repeat(64));
        let error = product.process_custodian.prepare(&launch.account_read).unwrap_err();
        assert!(matches!(&error, ProcessCustodyError::BindingMismatch("binaryDigestSha256")));
        let failure = account_prepare_failure(&launch, error);
        assert!(failure.pending.is_none(), "a confirmed pre-launch refusal must not occupy login custody");
        assert!(format!("{:?}", failure.error).contains("binaryDigestSha256"));
        drop(launch);
        let launch = product.prepare_owner_codex_login("instanceA").unwrap();
        let prepared = product.process_custodian.prepare(&launch.account_read).unwrap();
        let error = product.process_custodian.activate_with_failed_resume_for_test(&prepared).unwrap_err();
        assert!(activation_was_aborted(&error, product.process_custodian.is_tombstoned(&prepared.ticket)),
            "the real suspended child's confirmed failed-resume abort is releasable");
        assert!(product.process_custodian.active(&prepared.ticket).is_none());
        assert!(matches!(product.process_custodian.abort_prepared(&prepared), Err(ProcessCustodyError::DuplicateTicket(_))),
            "the factory has actually removed the prepared child");
        remove_owned_runtime(&launch.runtime_home, &launch.runtime_identity).unwrap();
        drop(launch);
        let uncertain = ProcessCustodyError::LaunchCleanup {
            cause: Box::new(ProcessCustodyError::Resume(std::io::Error::new(std::io::ErrorKind::Other, "controlled error"))),
            detail: "unconfirmed cleanup".into(),
        };
        assert!(!activation_was_aborted(&uncertain, true), "a tombstone alone is not an abort proof");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_login_retains_second_cli_custody_until_its_own_stop_is_confirmed() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-account-stop-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity } =
            product.prepare_owner_codex_login("instanceA").unwrap();
        // This first child uses the real pinned CLI and exits successfully
        // without authenticating. The automatic second child remains the
        // unchanged production app-server account/read command.
        login.launch.arguments = vec!["--version".into()];
        let prepared = product.process_custodian.prepare(&login).unwrap();
        drop(login);
        drop(account_read);
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"ownerAccountStop","expectedRevision":1}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        assert!(product.process_custodian.active(&prepared.ticket).unwrap()
            .wait(Duration::from_secs(15)).unwrap(), "pinned CLI version control did not exit");
        product.connection.execute("CREATE TRIGGER fail_account_stop BEFORE UPDATE OF state ON gogoke_coordination_process_custody WHEN NEW.state='STOPPED' AND NEW.operation_id LIKE 'login-observe-%' BEGIN SELECT RAISE(ABORT,'controlled account stop record failure'); END").unwrap();
        let active = ActiveOwnerLogin {
            instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
            expected_revision: command.expected_revision, operation_id,
            prepared, runtime_home, runtime_identity, output: String::new(), halted: false,
        };
        let error = product.finish_owner_device_login(&command, active, false).unwrap_err();
        assert!(format!("{error:?}").contains("controlled account stop record failure"));
        let pending = product.status_owner_device_login(&command).unwrap();
        let pending = String::from_utf8(pending).unwrap();
        assert!(pending.contains("\"state\":\"UNKNOWN\""));
        assert!(pending.contains("\"settled\":false"));
        assert!(pending.contains("controlled account stop record failure"));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "2",
            "the second real CLI must retain its own custody row");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1",
            "only the first CLI is durably stopped while the trigger blocks account/read");
        let new_begin = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"newWhileHeld","expectedRevision":1}"#;
        assert!(matches!(product.dispatch_owner_login_frame(new_begin),
            Err(OrchestrationError::OperationConflict)));
        product.connection.execute("DROP TRIGGER fail_account_stop").unwrap();
        let settled = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(settled.contains("\"settled\":true"));
        assert!(settled.contains("\"state\":\"LOGGED_OUT\""),
            "the retained real account/read frame must produce the native state");
        assert!(settled.contains("controlled account stop record failure"),
            "the original transient failure remains Owner-private");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "2");
        let update = Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instances SET version='0.148.0' WHERE instance_id='instanceA'").unwrap();
        update.step_done().unwrap();
        drop(update);
        let next_begin = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"newAfterRelease","expectedRevision":2}"#;
        assert!(matches!(product.dispatch_owner_login_frame(next_begin),
            Err(OrchestrationError::AccessDenied)), "a new request reaches its own preflight after release");
        // A later login child can stop successfully while account/read fails
        // before preparation. Its retained Final must keep that raw cause.
        product.connection.execute("UPDATE main.gogoke_v37_instances SET version='0.149.0' WHERE instance_id='instanceA'").unwrap();
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity } =
            product.prepare_owner_codex_login("instanceA").unwrap();
        login.launch.arguments = vec!["--version".into()];
        let prepared = product.process_custodian.prepare(&login).unwrap();
        drop(login); drop(account_read);
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"ownerAccountPreflight","expectedRevision":2}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        assert!(product.process_custodian.active(&prepared.ticket).unwrap().wait(Duration::from_secs(15)).unwrap());
        product.connection.execute("UPDATE main.gogoke_v37_instances SET version='0.148.0' WHERE instance_id='instanceA'").unwrap();
        let active = ActiveOwnerLogin { instance_id:command.instance_id.clone(),request_id:command.request_id.clone(),
            expected_revision:2,operation_id,prepared,runtime_home,runtime_identity,output:String::new(),halted:false };
        let error = product.finish_owner_device_login(&command, active, false).unwrap_err();
        assert!(matches!(&error, OrchestrationError::AccessDenied));
        let final_readback = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(final_readback.contains("\"settled\":true"));
        assert!(final_readback.contains("automatic account/read failed: AccessDenied"),
            "the same original native result retains the failure after the first Err is gone");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pinned_codex_isolated_credential_file_lifecycle_uses_cli_without_host_reads() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-isolated-auth-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied, "real pinned CLI is required, not a skipped control");
        let home = instance::resolve_codex_instance_home(&product.connection, &root, "instanceA").unwrap();
        // Exercise the CLI's own credential-file creation and replacement with
        // invalid synthetic markers, without a model call or real login. The
        // host does not open or copy the credential bytes, including in tests.
        for (index, marker) in [b"gogoke-synthetic-credential-one\n".as_slice(),
            b"gogoke-synthetic-credential-two\n".as_slice()].iter().enumerate() {
            let mut scope = product.prepare_owner_codex_login("instanceA").unwrap();
            scope.login.launch.arguments = vec!["login".into(), "--with-api-key".into()];
            let prepared = product.process_custodian.prepare(&scope.login).unwrap();
            let operation_id = format!("synthetic-cli-credential-{index}");
            authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
            let _trace = directed_trace::before_activation(&prepared);
            product.process_custodian.activate(&prepared).unwrap();
            authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
            product.process_custodian.active(&prepared.ticket).unwrap().write_persistent_frame(marker).unwrap();
            product.process_custodian.close_child_input(&prepared.ticket).unwrap();
            assert!(product.process_custodian.active(&prepared.ticket).unwrap()
                .wait(Duration::from_secs(15)).unwrap(), "CLI credential-file action did not exit");
            let proof = product.process_custodian.stop(&prepared.ticket,
                StopBudgets::production(), || Ok(())).unwrap();
            assert_eq!(proof.exit_code, Some(0), "CLI failure: {}",
                product.process_custodian.active(&prepared.ticket).unwrap().stderr_tail());
            let revision = authority::mark_process_stopped(&mut product.connection, &operation_id, &proof).unwrap();
            product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
                ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
                identity: prepared.identity.clone(), proof_hash: proof.proof_hash(), durable_revision: revision,
            }).unwrap();
            remove_owned_runtime(&scope.runtime_home, &scope.runtime_identity).unwrap();
            assert!(home.path.join("auth.json").is_file(), "official CLI must create its own file store");
        }
        let observed = product.dispatch_owner_login_observation(
            &request("login-state", "observeSyntheticPresence", 1, "{}")).unwrap();
        assert!(String::from_utf8(observed).unwrap().contains("\"state\":\"LOGGED_IN\""),
            "CLI presence observation is distinct from authentication validity");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pinned_codex_empty_home_reports_native_logout_and_durable_stop() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-real-login-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut product = ProductDatabase::open(&root, &database).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        let registered = decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap();
        assert_eq!(registered.status, V37Status::Applied,
            "CI must install the exact 0.149.0 native catalog, not skip the test");
        let home = instance::resolve_codex_instance_home(&product.connection,
            &root, "instanceA").unwrap();
        let runtime = home.path.join("gogoke-login-runtime");
        let scoped = product.prepare_owner_codex_login("instanceA").unwrap();
        let login_profile = scoped.login.launch.app_container_profile.as_deref().unwrap();
        assert_eq!(scoped.account_read.launch.app_container_profile.as_deref(),
            Some(login_profile), "device auth and account/read must share one isolated identity");
        assert!(scoped.login.launch.app_container_internet_client);
        assert!(scoped.account_read.launch.app_container_internet_client);
        assert!(scoped.login.launch.app_container_cli_identity_services);
        assert!(scoped.account_read.launch.app_container_cli_identity_services);
        assert_eq!(scoped.login.launch.application,
            scoped.account_read.launch.application);
        assert_eq!(scoped.login.launch.environment,
            scoped.account_read.launch.environment);
        assert_eq!(scoped.login.launch.current_directory.as_deref(), Some(runtime.as_path()));
        assert_eq!(scoped.account_read.launch.current_directory.as_deref(), Some(runtime.as_path()));
        remove_owned_runtime(&scoped.runtime_home, &scoped.runtime_identity).unwrap();
        drop(scoped); // release the prepared module/home locks before fixture teardown
        let query = request("login-state", "queryBeforeObservation", 1, "{}");
        let before = product.dispatch_user_request(&query).unwrap();
        assert!(String::from_utf8(before).unwrap().contains("\"state\":\"UNKNOWN\""));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "0",
            "K-INSTANCE login-state is a read; it must not start a provider process");
        let first = request("login-state", "loginReadA", 1, "{}");
        let observed_bytes = product.dispatch_owner_login_observation(&first).unwrap();
        let observed = decode_receipt(&observed_bytes).unwrap();
        assert_eq!(observed.status, V37Status::Applied);
        assert_eq!(observed.revision, 2);
        let observed_text = String::from_utf8(observed_bytes).unwrap();
        assert!(observed_text.contains("\"state\":\"LOGGED_OUT\""),
            "empty pinned CODEX_HOME must be reported by native account/read");
        assert!(!observed_text.contains("account"));
        assert!(!observed_text.contains("email"));
        assert!(!observed_text.contains("token"));
        assert_eq!(scalar(&product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1");
        let query = request("login-state", "queryAfterObservation", 2, "{}");
        let after = product.dispatch_user_request(&query).unwrap();
        assert!(String::from_utf8(after).unwrap().contains("\"state\":\"LOGGED_OUT\""));
        assert!(!runtime.exists());
        let replay = decode_receipt(&product.dispatch_owner_login_observation(&first).unwrap()).unwrap();
        assert_eq!(replay.status, V37Status::Replayed);
        assert_eq!(replay.revision, 2);
        assert_eq!(scalar(&product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1",
            "exact replay cannot restart the pinned CLI");
        let second = request("login-state", "loginReadB", 2, "{}");
        let unchanged = decode_receipt(&product.dispatch_owner_login_observation(&second).unwrap()).unwrap();
        assert_eq!(unchanged.status, V37Status::Applied);
        assert_eq!(unchanged.revision, 2, "same native login state does not advance F revision");
        assert!(!runtime.exists());
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}
