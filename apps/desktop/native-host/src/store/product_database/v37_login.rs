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
#[path = "v37_login_cache.rs"]
mod login_cache;
#[path = "v37_login_provider.rs"]
mod provider_runtime;
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
/// `login` is started only by an explicit Owner action. Its matched app-server
/// authUrl and vendor errors are delivered on the Owner-private User pipe.
/// `account_read` uses a separate fixed LPAC app-server RPC.
/// Preparation creates only host-owned directories and data, with no process.
pub(super) struct PreparedOwnerLogin {
    pub(super) login: PrepareRequest,
    pub(super) account_read: PrepareRequest,
    pub(super) runtime_home: PathBuf,
    pub(super) runtime_identity: RootIdentity,
    pub(super) registered_driver: String,
    pub(super) registered_home_identity: RootIdentity,
    pub(super) credential_custody: Option<std::sync::Arc<crate::process::CredentialBinding>>,
}

/// In-memory Owner action state. ProductDatabase owns one of these beside its
/// ProcessCustodian. The custody row remains the durable restart fact; after a
/// host restart PREPARED/ACTIVE is UNKNOWN and the same request is not replayed
/// into a new CLI process.
pub(super) enum OwnerLoginSession {
    Active(ActiveOwnerLogin),
    PendingFirstStop(PendingFirstStop),
    PendingAccount(PendingAccountRead),
    Final {
        instance_id: String,
        request_id: String,
        expected_revision: u64,
        state: String,
        output: String,
    },
}

struct PendingFirstStop {
    active: ActiveOwnerLogin,
    proof: NativeStopProof,
    durable_revision: Option<u64>,
    cancelled: bool,
    login_failure: Option<String>,
    primary_error: String,
    latest_error: Option<String>,
}

struct PendingAccountCustody {
    operation_id: Option<String>,
    prepared: Option<PreparedCustody>,
    runtime_home: Option<PathBuf>,
    runtime_identity: Option<RootIdentity>,
    registered_driver: Option<String>,
    registered_home_identity: Option<RootIdentity>,
    proof: Option<NativeStopProof>,
    durable_revision: Option<u64>,
    abort_prepared: bool,
    released: bool,
    frame: Option<OriginBoundFrame>,
    backend_source: Option<instance::BackendSource>,
    credential_custody: Option<std::sync::Arc<crate::process::CredentialBinding>>,
    request: Option<V37Request>,
}

struct PendingAccountRead {
    instance_id: String,
    request_id: String,
    expected_revision: u64,
    output: String,
    latest_error: Option<String>,
    custody: PendingAccountCustody,
    continuation: Option<ConfirmedLoginContinuation>,
}

struct ConfirmedLoginContinuation {
    provider: Option<instance::provider_login::PreparedProviderLogin>,
    completion_frame: bool,
    cancelled: bool,
    login_failure: Option<String>,
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
            registered_driver: Some(launch.registered_driver.clone()),
            registered_home_identity: Some(launch.registered_home_identity.clone()),
            proof: None, durable_revision: None, abort_prepared: false, released: false, frame: None, backend_source: None,
            credential_custody: launch.credential_custody.clone(), request: None,
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
    stderr_seen: usize,
    halted: bool,
    rpc: Option<LoginRpc>,
    provider: Option<instance::provider_login::PreparedProviderLogin>,
    provider_completion_frame: bool,
}

struct LoginRpc {
    phase: LoginRpcPhase,
    login_id: Option<String>,
    early_completion: Option<Vec<u8>>,
    response_started: Instant,
    stdout_seen: usize,
    frames_seen: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LoginRpcPhase { SendInitialize, Initialize, Start, Completion, Complete }

const LOGIN_START: &[u8] = b"{\"id\":4,\"method\":\"account/login/start\",\"params\":{\"type\":\"chatgpt\"}}\n";

fn rpc_object(frame: &[u8]) -> std::result::Result<BTreeMap<JsonString, Json>, String> {
    let text = std::str::from_utf8(frame).map_err(|error| format!("login RPC UTF-8: {error}"))?;
    object(Parser::parse(text.trim_end()).map_err(|error| format!("login RPC JSON: {error:?}"))?)
        .ok_or_else(|| "login RPC object required".into())
}

fn rpc_string(fields: &mut BTreeMap<JsonString, Json>, name: &str) -> Option<String> {
    match fields.remove(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string(),
        _ => None,
    }
}

fn login_rpc_vendor_error(frame: &[u8]) -> Option<String> {
    rpc_object(frame).ok()?.remove(&JsonString::from_str("error"))
        .map(|error| error.canonical())
}

fn login_start_result(frame: &[u8]) -> std::result::Result<(String, String), String> {
    let mut fields = rpc_object(frame)?;
    if !matches!(fields.remove(&JsonString::from_str("id")), Some(Json::Number(id)) if id == "4") {
        return Err("login start RPC id mismatch".into());
    }
    if let Some(error) = fields.remove(&JsonString::from_str("error")) {
        return Err(format!("login start vendor error: {}", error.canonical()));
    }
    let mut result = object(fields.remove(&JsonString::from_str("result"))
        .ok_or_else(|| "login start result missing".to_owned())?)
        .ok_or_else(|| "login start result object required".to_owned())?;
    if rpc_string(&mut result, "type").as_deref() != Some("chatgpt") {
        return Err("login start type mismatch".into());
    }
    let login_id = rpc_string(&mut result, "loginId")
        .filter(|value| !value.is_empty() && value.len() <= 128
            && value.bytes().all(|byte| byte.is_ascii_hexdigit() || byte == b'-'))
        .ok_or_else(|| "login start loginId missing".to_owned())?;
    let auth_url = rpc_string(&mut result, "authUrl")
        .filter(|value| value.starts_with("https://auth.openai.com/oauth/authorize?")
            && !value.chars().any(|ch| ch == '\r' || ch == '\n') && value.len() <= 65_536)
        .ok_or_else(|| "login start complete authUrl missing".to_owned())?;
    Ok((login_id, auth_url))
}

fn login_completion(frame: &[u8], expected_id: &str)
    -> std::result::Result<Option<String>, String> {
    let mut fields = rpc_object(frame)?;
    if fields.contains_key(&JsonString::from_str("id")) {
        return Err("login completion carried RPC id".into());
    }
    if rpc_string(&mut fields, "method").as_deref() != Some("account/login/completed") {
        return Err("login completion method mismatch".into());
    }
    let mut params = object(fields.remove(&JsonString::from_str("params"))
        .ok_or_else(|| "login completion params missing".to_owned())?)
        .ok_or_else(|| "login completion params object required".to_owned())?;
    if rpc_string(&mut params, "loginId").as_deref() != Some(expected_id) {
        return Err("login completion loginId mismatch".into());
    }
    match params.remove(&JsonString::from_str("success")) {
        Some(Json::Bool(true)) => Ok(None),
        Some(Json::Bool(false)) => Ok(Some(rpc_string(&mut params, "error")
            .unwrap_or_else(|| "login completion reported failure".into()))),
        _ => Err("login completion success missing".into()),
    }
}

fn append_complete_login_stderr(output: &mut String, stderr_seen: &mut usize,
    bytes: &[u8]) -> std::result::Result<(), String> {
    if *stderr_seen > bytes.len() { return Err("owner login stderr capture regressed".into()); }
    let unread = &bytes[*stderr_seen..];
    let Some(last_newline) = unread.iter().rposition(|byte| *byte == b'\n') else { return Ok(()); };
    let complete = &unread[..=last_newline];
    let text = std::str::from_utf8(complete).map_err(|error|
        format!("owner login stderr is not UTF-8: {error}"))?;
    let separator = usize::from(!output.is_empty() && !output.ends_with('\n'));
    if output.len().saturating_add(separator).saturating_add(complete.len()) > 65_536 {
        return Err("owner login output limit".into());
    }
    if separator != 0 { output.push('\n'); }
    output.push_str(text);
    *stderr_seen += complete.len();
    Ok(())
}

fn owner_login_failure(cancelled: bool, exit_code: Option<u32>, stderr: &str,
    capture_failure: Option<String>) -> Option<String> {
    if cancelled { return None; }
    if exit_code != Some(0) {
        let mut error = format!("owner login process exited: code={exit_code:?}; STDERR_TAIL: {stderr}");
        if let Some(capture) = capture_failure { error.push_str(&format!("; {capture}")); }
        Some(error)
    } else {
        capture_failure
    }
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
    fn legacy_account_custody(&self, instance_id: &str) -> Result<Vec<instance::LegacyCustodyRow>> {
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,''),stop_proof_hash IS NULL FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 ORDER BY operation_id")?;
        query.bind_text(1, instance_id)?;
        let mut rows = Vec::new();
        while query.step_row()? {
            rows.push(instance::LegacyCustodyRow {
                operation_id: query.column_text(0)?, ticket: query.column_text(1)?,
                custodian_nonce: query.column_text(2)?, pid: query.column_text(3)?,
                creation_time_100ns: query.column_text(4)?, image_path: query.column_text(5)?,
                binary_digest_sha256: query.column_text(6)?, profile_id: query.column_text(7)?,
                domain_id: query.column_text(8)?, generation: query.column_text(9)?,
                state: query.column_text(10)?, stop_proof_hash: if query.column_text(12)? == "1" {
                    None
                } else { Some(query.column_text(11)?) },
            });
        }
        Ok(rows)
    }

    fn legacy_account_profile_names(&self, instance_id: &str, home: &RootIdentity,
        rows: &[instance::LegacyCustodyRow]) -> Result<Vec<String>> {
        let mut names = vec![owner_login_profile_name(instance_id, home)];
        for row in rows {
            if row.profile_id != instance_id { return Err(OrchestrationError::AccessDenied); }
            if row.domain_id == "global" { continue; }
            let query = Statement::prepare(self.connection.as_ptr(),
                "SELECT domain_id,session_id,seat_incarnation,generation,instance_id FROM main.gogoke_v37_h_process_episode WHERE process_operation_id=?1")?;
            query.bind_text(1, &row.operation_id)?;
            if !query.step_row()? { return Err(OrchestrationError::Invalid("legacy profile original H episode missing")); }
            let domain = query.column_text(0)?;
            let session = query.column_text(1)?;
            let incarnation = query.column_text(2)?;
            let generation = query.column_text(3)?;
            if domain != row.domain_id || generation != row.generation
                || query.column_text(4)? != instance_id || incarnation.is_empty() || query.step_row()? {
                return Err(OrchestrationError::AccessDenied);
            }
            let digest = crate::store::digest::sha256_hex(format!("{}\n{domain}\n{session}\n{incarnation}\n{generation}",
                self.root.canonical_root().identity.opaque()).as_bytes());
            names.push(format!("Gogoke37.Session.{}", &digest[..40]));
        }
        names.sort(); names.dedup();
        Ok(names)
    }

    /// This is an ACL recovery qualification, never a stop or quiescence fact.
    /// It runs before creating a new account runtime directory so the saved
    /// physical inventory cannot contain that attempt's transient substitute.
    fn recover_legacy_account_baseline(&mut self, instance_id: &str,
        home: &instance::ResolvedDirectory, profile: &AppContainerProfile) -> Result<()> {
        use crate::process::{CredentialBinding, CredentialError, LegacyAclInventory, NativeBootIdentity};
        use instance::{LegacyAclStep, LegacyStepPhase};
        fn failure<T>(result: std::result::Result<T, impl std::fmt::Debug>) -> Result<T> {
            result.map_err(|error| OrchestrationError::V37StoreFailure(format!("legacy boot ACL recovery: {error:?}")))
        }
        failure(instance::initialize_legacy_fence_schema(&mut self.connection))?;
        let prior = failure(instance::read_legacy_fence(&self.connection, instance_id))?;
        let source = home.path.join("auth.json");
        let (source_identity, source_links) = match CredentialBinding::observe_source_metadata(self.root, &source, &home.identity) {
            Ok(value) => value,
            Err(CredentialError::Io { source, .. }) if source.raw_os_error() == Some(2) && prior.is_none() => return Ok(()),
            Err(error) => return failure(Err::<(), _>(error)),
        };
        let legacy = failure(profile.has_legacy_owner_login_grant(&home.path, &home.identity))?;
        if prior.is_none() {
            if !legacy { return Ok(()); }
            let unresolved = Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE domain_id='global' AND profile_id=?1 AND (state<>'STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1")?;
            unresolved.bind_text(1, instance_id)?;
            if !unresolved.step_row()? { return Ok(()); }
        }
        let boot = failure(NativeBootIdentity::observe())?.canonical_hex();
        let aliases = failure(instance::read_credential_aliases(&self.connection, instance_id))?;
        let mut scopes = Vec::new();
        for alias in aliases.iter().filter(|row| row.state != "REMOVED") {
            if !matches!(alias.state.as_str(), "ACTIVE" | "DORMANT") || alias.source_file_identity != source_identity {
                return Err(OrchestrationError::Invalid("legacy recovery alias intent or source changed"));
            }
            let directory = failure(instance::resolve_private_history_directory(&self.connection, self.root, &alias.history_id))?;
            if directory.identity != alias.directory_identity { return Err(OrchestrationError::AccessDenied); }
            scopes.push(crate::process::CredentialAliasScope { root: directory.path, root_identity: directory.identity });
        }
        let binding = failure(CredentialBinding::open_registered(self.root, &source,
            &home.identity, &source_identity, &scopes))?;
        if let Some(retained) = self.recovered_credential_holders.get(&(instance_id.into(), source_identity.opaque())) {
            if !std::sync::Arc::ptr_eq(retained, &binding) { return Err(OrchestrationError::AccessDenied); }
            let original = prior.as_ref().ok_or(OrchestrationError::AccessDenied)?;
            let live = self.legacy_account_custody(instance_id)?;
            if original.custody.iter().any(|old| !live.iter().any(|row| row == old))
                || live.iter().any(|row| !original.custody.iter().any(|old| old.operation_id == row.operation_id)
                    && matches!(row.state.as_str(), "UNKNOWN" | "PREPARED")) {
                return Err(OrchestrationError::Invalid("legacy retained holder does not retire current uncertain custody"));
            }
            failure(binding.verify_registered_aliases(&scopes))?;
            if failure(binding.acl_prepared_in_this_holder())? { return Ok(()); }
        }
        let object = failure(instance::read_credential_object(&self.connection, instance_id))?;
        if object.as_ref().is_some_and(|row| row.root_identity != *self.connection.root_identity()
            || row.home_identity != home.identity || row.file_identity != source_identity
            || row.source_parent_identity != home.identity) { return Err(OrchestrationError::AccessDenied); }
        let original_rows = match &prior { Some(row) => row.custody.clone(), None => self.legacy_account_custody(instance_id)? };
        let names = self.legacy_account_profile_names(instance_id, &home.identity, &original_rows)?;
        let provenance = crate::store::digest::sha256_hex(names.join("\n").as_bytes());
        let completed = match &prior {
            Some(_) => failure(instance::read_legacy_step(&self.connection, instance_id, LegacyAclStep::Baseline))?
                .filter(|step| step.phase == LegacyStepPhase::Applied),
            None => None,
        };
        if let (Some(record), Some(step)) = (&prior, completed) {
            if record.original_boot == boot || record.database_identity != self.connection.identity().opaque()
                || record.root_identity != self.connection.root_identity().opaque()
                || record.home_identity != home.identity.opaque() || record.source_identity != source_identity.opaque()
                || record.source_parent_identity != home.identity.opaque() || record.acl_provenance_digest != provenance {
                return Err(OrchestrationError::AccessDenied);
            }
            let live = self.legacy_account_custody(instance_id)?;
            if original_rows.iter().any(|original| !live.iter().any(|row| row == original))
                || live.iter().any(|row| !original_rows.iter().any(|old| old.operation_id == row.operation_id)
                    && (row.state != "STOPPED" || row.stop_proof_hash.as_deref().map_or(true, str::is_empty))) {
                return Err(OrchestrationError::Invalid("legacy recovery retains current unknown or active custody"));
            }
            let profiles = failure(instance::read_credential_profiles(&self.connection, instance_id))?;
            if profiles.iter().any(|row| row.state != "REVOKED") {
                return Err(OrchestrationError::Invalid("legacy cold baseline retains active credential holders"));
            }
            let permitted = vec![(failure(profile.sid_identity())?, 0x0012_0089)];
            let (inventory, receipt) = failure(LegacyAclInventory::restore_for_adoption(self.root,
                &home.path, &home.identity, &binding, &names, &record.native_snapshot,
                &record.native_snapshot_digest, &scopes, &permitted))?;
            if inventory.source_target_digest() != step.target_source_acl_digest
                || receipt.baseline_core_digest != inventory.baseline_core_digest() {
                return Err(OrchestrationError::AccessDenied);
            }
            self.recovered_credential_holders.insert((instance_id.into(), source_identity.opaque()), binding);
            return Ok(());
        }
        if source_links != 1 || !scopes.is_empty() {
            return Err(OrchestrationError::Invalid("legacy recovery requires original single-link source without aliases"));
        }
        let inventory = match &prior {
            Some(row) => failure(LegacyAclInventory::restore(self.root, &home.path, &home.identity,
                &binding, &names, &row.native_snapshot, &row.native_snapshot_digest))?,
            None => failure(LegacyAclInventory::capture(self.root, &home.path, &home.identity, &binding, &names))?,
        };
        let mut record = match prior {
            Some(row) => row,
            None => {
                let capture = instance::LegacyFenceCapture {
                instance_id: instance_id.into(), original_boot: boot.clone(),
                database_identity: self.connection.identity().clone(), root_identity: self.connection.root_identity().clone(),
                home_identity: home.identity.clone(), source_identity: source_identity.clone(),
                source_parent_identity: home.identity.clone(), source_revision: object.as_ref().map(|row| row.revision),
                source_link_count: u64::from(source_links), registered_alias_count: 0, custody: original_rows,
                home_acl_digest: inventory.home_original_digest(), source_acl_digest: inventory.source_original_digest(),
                acl_provenance_digest: provenance.clone(), native_snapshot: inventory.encode_snapshot(),
                native_snapshot_digest: inventory.original_digest(),
                };
                failure(instance::capture_legacy_fence(&mut self.connection, &capture))?
            },
        };
        if record.original_boot == boot {
            return Err(OrchestrationError::Invalid("legacy account scope requires original stopped observer custody; exact metadata fence saved, Windows system restart required"));
        }
        // Replaying a pending intent does not run begin again. Recheck its
        // original process/holder boundary before any native ACL operation,
        // not only at the subsequent finish transaction.
        let live = self.legacy_account_custody(instance_id)?;
        if record.custody.iter().any(|original| !live.iter().any(|row| row == original))
            || live.iter().any(|row| !record.custody.iter().any(|old| old.operation_id == row.operation_id)
                && (row.state != "STOPPED" || row.stop_proof_hash.as_deref().map_or(true, str::is_empty))) {
            return Err(OrchestrationError::Invalid("legacy pending recovery retains current unknown or active custody"));
        }
        if failure(instance::read_credential_profiles(&self.connection, instance_id))?.iter().any(|row| row.state != "REVOKED") {
            return Err(OrchestrationError::Invalid("legacy pending recovery retains credential holder intent"));
        }
        let mut proof = instance::LegacyPhysicalProof {
            database_identity: self.connection.identity().clone(), root_identity: self.connection.root_identity().clone(),
            home_identity: home.identity.clone(), source_identity: source_identity.clone(), source_parent_identity: home.identity.clone(),
            source_revision: object.as_ref().map(|row| row.revision), source_link_count: u64::from(source_links),
            registered_alias_count: 0, acl_provenance_digest: provenance,
            actual_home_acl_digest: inventory.home_original_digest(), actual_source_acl_digest: inventory.source_original_digest(),
        };
        let request_base = format!("legacy-acl-{}", crate::store::digest::sha256_hex(instance_id.as_bytes()));
        let home_step = match failure(instance::read_legacy_step(&self.connection, instance_id, LegacyAclStep::Home))? {
            Some(step) => step,
            None => failure(instance::begin_legacy_acl_step(&mut self.connection, &instance::LegacyStepRequest {
                instance_id, step: LegacyAclStep::Home, request_id: &format!("{request_base}-home"), current_boot: &boot,
                expected_revision: record.revision, target_home_acl_digest: &inventory.home_target_digest(),
                target_source_acl_digest: &inventory.source_after_home_digest(), proof: &proof,
            }))?,
        };
        let prior_baseline = failure(instance::read_legacy_step(&self.connection, instance_id, LegacyAclStep::Baseline))?;
        let home_receipt = if home_step.phase == LegacyStepPhase::Applied && prior_baseline.is_some() {
            // Reconstruct the already-committed HOME receipt from its exact
            // immutable targets. The native baseline reconciler independently
            // verifies all current HOME objects and either precise source state.
            crate::process::LegacyHomeReceipt {
                home_observed_digest: home_step.target_home_acl_digest.clone(),
                source_observed_digest: home_step.target_source_acl_digest.clone(),
            }
        } else { failure(inventory.reconcile_home(self.root, &home.path, &home.identity, &binding))? };
        proof.actual_home_acl_digest = home_receipt.home_observed_digest.clone();
        proof.actual_source_acl_digest = home_receipt.source_observed_digest.clone();
        if home_step.phase == LegacyStepPhase::Pending {
            record = failure(instance::finish_legacy_acl_step(&mut self.connection, &instance::LegacyStepFinish {
                instance_id, step: LegacyAclStep::Home, request_id: &home_step.request_id,
                current_boot: &boot, expected_intent_revision: home_step.intent_revision, proof: &proof,
            }))?;
        }
        let baseline = match failure(instance::read_legacy_step(&self.connection, instance_id, LegacyAclStep::Baseline))? {
            Some(step) => step,
            None => failure(instance::begin_legacy_acl_step(&mut self.connection, &instance::LegacyStepRequest {
                instance_id, step: LegacyAclStep::Baseline, request_id: &format!("{request_base}-baseline"), current_boot: &boot,
                expected_revision: record.revision, target_home_acl_digest: &inventory.home_target_digest(),
                target_source_acl_digest: &inventory.source_target_digest(), proof: &proof,
            }))?,
        };
        let receipt = failure(inventory.prepare_source_baseline(self.root, &home.path,
            &home.identity, &binding, &home_receipt))?;
        proof.actual_source_acl_digest = receipt.source_observed_digest;
        if baseline.phase == LegacyStepPhase::Pending {
            failure(instance::finish_legacy_acl_step(&mut self.connection, &instance::LegacyStepFinish {
                instance_id, step: LegacyAclStep::Baseline, request_id: &baseline.request_id,
                current_boot: &boot, expected_intent_revision: baseline.intent_revision, proof: &proof,
            }))?;
        }
        self.recovered_credential_holders.insert((instance_id.into(), source_identity.opaque()), binding);
        Ok(())
    }

    fn credential_instance_is_quiescent(&self,instance_id:&str)->Result<bool> {
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_coordination_process_custody c
             WHERE c.state<>'STOPPED' AND
               ((c.domain_id='global' AND c.profile_id=?1) OR c.operation_id IN
                (SELECT process_operation_id FROM main.gogoke_v37_h_claim WHERE instance_id=?1
                 UNION SELECT process_operation_id FROM main.gogoke_v37_h_process_episode WHERE instance_id=?1)) LIMIT 1")?;
        query.bind_text(1,instance_id)?;
        if query.step_row()? {return Ok(false);}
        Ok(instance::read_credential_profiles(&self.connection,instance_id)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("credential quiescent profiles: {error:?}")))?
            .iter().all(|profile|profile.state=="REVOKED"))
    }

    fn migrate_owner_account_scope(&self,profile:&AppContainerProfile,instance_id:&str,
        home:&instance::ResolvedDirectory,credential:Option<(&crate::process::CredentialBinding,
            &[crate::process::CredentialAliasScope])>)->Result<()> {
        if !profile.has_legacy_owner_login_grant(&home.path,&home.identity)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("legacy observer scope: {error:?}")))? {
            return Ok(());
        }
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_coordination_process_custody
             WHERE domain_id='global' AND profile_id=?1 AND
               (state<>'STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1")?;
        query.bind_text(1,instance_id)?;
        if query.step_row()? {return Err(OrchestrationError::Invalid("legacy account scope requires original stopped observer custody"));}
        profile.migrate_legacy_owner_login_grant(self.root,&home.path,&home.identity,credential)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("legacy account scope migration: {error:?}")))
    }

    fn grant_registered_account_observer(&mut self,profile:&AppContainerProfile,
        instance_id:&str,home:&instance::ResolvedDirectory,runtime:&Path,
        runtime_identity:&RootIdentity,program:&Path)
        ->Result<Option<std::sync::Arc<crate::process::CredentialBinding>>> {
        use crate::process::{CredentialAliasScope,CredentialBinding,CredentialError};
        let source=home.path.join("auth.json");
        let observed=CredentialBinding::observe_source_metadata(self.root,&source,&home.identity);
        let held=match observed {
            Err(CredentialError::Io {source,..}) if source.raw_os_error()==Some(2)=>{
                self.migrate_owner_account_scope(profile,instance_id,home,None)?;
                profile.grant_bound_owner_account_empty(self.root,&home.path,&home.identity,
                    runtime,runtime_identity).map_err(|error|OrchestrationError::V37StoreFailure(
                        format!("empty account observer scope: {error:?}")))?;
                None
            },
            Err(error)=>return Err(OrchestrationError::V37StoreFailure(format!("account source metadata: {error:?}"))),
            Ok((identity,_))=>{
                let registered=instance::read_credential_object(&self.connection,instance_id)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("account source registration: {error:?}")))?;
                let aliases=instance::read_credential_aliases(&self.connection,instance_id)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("account alias registration: {error:?}")))?;
                let mut scopes=Vec::new();
                for alias in aliases.iter().filter(|alias|alias.state!="REMOVED") {
                    if !matches!(alias.state.as_str(),"ACTIVE"|"DORMANT")
                        || alias.source_file_identity!=identity {
                        return Err(OrchestrationError::Invalid("account alias intent unresolved or source changed"));
                    }
                    let directory=instance::resolve_private_history_directory(&self.connection,self.root,&alias.history_id)
                        .map_err(|error|OrchestrationError::V37StoreFailure(format!("account alias scope: {error:?}")))?;
                    if directory.identity!=alias.directory_identity {
                        return Err(OrchestrationError::AccessDenied);
                    }
                    scopes.push(CredentialAliasScope {root:directory.path,root_identity:directory.identity});
                }
                if let Some(registered)=registered {
                    if registered.home_identity!=home.identity || registered.source_parent_identity!=home.identity {
                        return Err(OrchestrationError::AccessDenied);
                    }
                    if registered.file_identity!=identity
                        && (!scopes.is_empty() || !self.credential_instance_is_quiescent(instance_id)?) {
                        return Err(OrchestrationError::Invalid("account source replacement is not quiescent"));
                    }
                } else if !scopes.is_empty() {return Err(OrchestrationError::AccessDenied);}
                let binding=CredentialBinding::open_registered(self.root,&source,&home.identity,&identity,&scopes)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("account source custody: {error:?}")))?;
                self.migrate_owner_account_scope(profile,instance_id,home,Some((&binding,&scopes)))?;
                if scopes.is_empty() && !binding.acl_prepared_in_this_holder()
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("account source ACL witness: {error:?}")))? {
                    if !self.credential_instance_is_quiescent(instance_id)? {
                        return Err(OrchestrationError::Invalid("account source baseline requires stopped instance"));
                    }
                    AppContainerProfile::prepare_quiescent_owner_account_source(self.root,&home.path,&home.identity,&binding)
                        .map_err(|error|OrchestrationError::V37StoreFailure(format!("quiescent account source: {error:?}")))?;
                }
                profile.grant_bound_owner_account_observer(self.root,&home.path,&home.identity,
                    runtime,runtime_identity,&binding,&scopes).map_err(|error|OrchestrationError::V37StoreFailure(
                        format!("registered account observer scope: {error:?}")))?;
                Some(binding)
            },
        };
        let program_identity=AppContainerProfile::capture_program_identity(program)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("account program identity: {error:?}")))?;
        profile.grant_bound_program(program,&program_identity)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("account program scope: {error:?}")))?;
        profile.verify_bound_program_grant(program,&program_identity)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("account program scope readback: {error:?}")))?;
        Ok(held)
    }

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
            let mut fields=BTreeMap::from([
                (JsonString::from_str("instanceId"), string(&instance_id)),
                (JsonString::from_str("driverId"), string(&row.driver_id)),
                (JsonString::from_str("version"), string(&row.version)),
                (JsonString::from_str("installState"), string(&install_state)),
                (JsonString::from_str("loginState"), string(&login_state)),
                (JsonString::from_str("revision"), string(&revision)),
            ]);
            if let Some(version)=instance::known_new_version(&row.driver_id,&row.version) {
                fields.insert(JsonString::from_str("newVersion"),string(version));
            }
            let issues=self.read_user_instance_runtime_issues(&instance_id,&row.driver_id)?;
            if !issues.is_empty() {fields.insert(JsonString::from_str("runtimeIssues"),Json::Array(issues));}
            instances.push(Json::Object(fields));
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
        // Only this Owner-private response carries login stdout/stderr. The
        // value is never stored in the instance or coordination journal.
        (JsonString::from_str("output"), Json::String(JsonString::from_str(output))),
    ])).canonical().into_bytes()
}

fn pending_first_stop_output(pending: &PendingFirstStop) -> String {
    let mut output = pending.active.output.clone();
    for cause in [pending.login_failure.as_deref(), Some(pending.primary_error.as_str()),
        pending.latest_error.as_deref()] {
        if let Some(cause) = cause {
            if !output.is_empty() { output.push('\n'); }
            output.push_str(cause);
        }
    }
    output
}

fn same_owner_login(session: &OwnerLoginSession, command: &OwnerLoginCommand) -> bool {
    let (instance, request, revision) = match session {
        OwnerLoginSession::Active(active) => (&active.instance_id, &active.request_id,
            active.expected_revision),
        OwnerLoginSession::PendingFirstStop(pending) => (&pending.active.instance_id,
            &pending.active.request_id, pending.active.expected_revision),
        OwnerLoginSession::PendingAccount(pending) => (&pending.instance_id, &pending.request_id,
            pending.expected_revision),
        OwnerLoginSession::Final { instance_id, request_id, expected_revision, .. } =>
            (instance_id, request_id, *expected_revision),
    };
    instance == &command.instance_id && request == &command.request_id
        && revision == command.expected_revision
}

pub(super) fn pending_login_for_instance(session: &OwnerLoginSession, instance: &str) -> bool {
    match session {
        OwnerLoginSession::Active(active) => active.instance_id == instance,
        OwnerLoginSession::PendingFirstStop(pending) => pending.active.instance_id == instance,
        OwnerLoginSession::PendingAccount(pending) => pending.instance_id == instance,
        OwnerLoginSession::Final { .. } => false,
    }
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
    fn original_io(operation: &str, entry: &Path, error: std::io::Error) -> OrchestrationError {
        OrchestrationError::V37StoreFailure(format!(
            "login runtime cleanup {operation}: name={:?}; {error}; raw_os_error={:?}",
            entry.file_name(), error.raw_os_error()))
    }
    fn remove_contents(path: &Path, depth: usize) -> Result<()> {
        if depth > 32 { return Err(OrchestrationError::AccessDenied); }
        for entry in fs::read_dir(path).map_err(|error| original_io("read-directory", path, error))? {
            let entry = entry.map_err(OrchestrationError::Io)?;
            let child = entry.path();
            let metadata = fs::symlink_metadata(&child).map_err(|error| original_io("metadata", &child, error))?;
            if metadata.file_attributes() & REPARSE_POINT != 0 {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "login runtime cleanup refuses reparse child: name={:?}; attributes={:#x}",
                    child.file_name(), metadata.file_attributes())));
            }
            if metadata.is_dir() {
                remove_contents(&child, depth + 1)?;
                fs::remove_dir(&child).map_err(|error| original_io("remove-directory", &child, error))?;
            } else if metadata.is_file() {
                fs::remove_file(&child).map_err(|error| original_io("remove-file", &child, error))?;
            } else {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        Ok(())
    }
    remove_contents(path, 0)?;
    fs::remove_dir(path).map_err(|error| original_io("remove-runtime-root", path, error))
}

fn retain_primary_cleanup_error(primary: OrchestrationError, cleanup: Result<()>,
    context: &str) -> OrchestrationError {
    match cleanup {
        Ok(()) => primary,
        Err(error) => OrchestrationError::V37StoreFailure(format!(
            "{context}: {primary:?}; runtime cleanup: {error:?}")),
    }
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

/// Ordinary browser login runs with the native host's user token. Only the
/// registered instance home is redirected; browser profile directories stay
/// with this user while the host opens the returned authorization URL.
/// This is a finite allowlist, never the host's arbitrary provider variables.
fn ordinary_login_environment(instance_home: &Path, runtime: &Path) -> Result<Vec<(String, String)>> {
    let mut environment = clean_environment(instance_home, runtime)?;
    let instance = instance_home.to_string_lossy().into_owned();
    for (key, value) in &mut environment {
        if key == "HOME" || key == "USERPROFILE" { *value = instance.clone(); }
        if matches!(key.as_str(), "LOCALAPPDATA" | "APPDATA" | "TEMP" | "TMP") {
            let actual = std::env::var(key.as_str()).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("login {key} unavailable: {error}")))?;
            if !Path::new(&actual).is_absolute() || actual.contains('\0') {
                return Err(OrchestrationError::AccessDenied);
            }
            *value = actual;
        }
    }
    Ok(environment)
}

fn owner_login_profile_name(instance_id: &str, home_identity: &RootIdentity) -> String {
    // The registered physical home, rather than a wire path or request ID,
    // determines the isolation domain across login and account/read launches.
    let digest = crate::store::digest::sha256_hex(
        format!("{}\n{instance_id}", home_identity.opaque()).as_bytes());
    format!("Gogoke37.OwnerLogin.{}", &digest[..40])
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
                OwnerLoginSession::PendingFirstStop(_) |
                OwnerLoginSession::PendingAccount(_))) {
                return Err(OrchestrationError::OperationConflict);
            }
            let state = self.owner_login_account_state(&command)?;
            return Ok(owner_login_reply(&command, &state, ""));
        }
        if let Some(session) = &self.owner_login {
            if !same_owner_login(session, &command) {
                if matches!(session, OwnerLoginSession::Active(_) |
                    OwnerLoginSession::PendingFirstStop(_) |
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
                latest_error:None,custody,continuation:None,
            }));
            failure.error
        } else { self.settle_owner_login_preflight_error(command, failure.error) }
    }

    fn begin_owner_device_login(&mut self, command: &OwnerLoginCommand) -> Result<Vec<u8>> {
        if let Some(session) = &self.owner_login {
            return Ok(match session {
                OwnerLoginSession::Active(active) => owner_login_reply(command,
                    if active.halted { "UNKNOWN" } else { "PENDING" },
                    &if active.provider.is_some() { self.provider_display_output(active) }
                        else { active.output.clone() }),
                OwnerLoginSession::PendingFirstStop(pending) => owner_login_reply(command,
                    "UNKNOWN", &pending_first_stop_output(pending)),
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
        let row = self.read_registered_instance(&command.instance_id)?
            .ok_or_else(|| self.settle_owner_login_preflight_error(command, OrchestrationError::AccessDenied))?;
        if row.driver_id != "codex" {
            return self.begin_registered_provider_login(command);
        }
        crate::store::session_transport::credential_launch::quiescent_cleanup(&mut self.connection,
            self.root,&command.instance_id,&command.request_id)
            .map_err(|error|self.settle_owner_login_preflight_error(command,
                OrchestrationError::V37StoreFailure(format!("ordinary login credential aliases: {error}"))))?;
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
            registered_driver:Some(launch.registered_driver.clone()),
            registered_home_identity:Some(launch.registered_home_identity.clone()),
            proof,durable_revision:None,abort_prepared,released:false,frame:None,backend_source:None,
            credential_custody:None,request:None,
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
            stderr_seen: 0,
            halted: false,
            rpc: Some(LoginRpc { phase: LoginRpcPhase::SendInitialize,
                login_id: None, early_completion: None, response_started: Instant::now(),
                stdout_seen: 0, frames_seen: 0 }),
            provider: None,
            provider_completion_frame: false,
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
            OwnerLoginSession::PendingFirstStop(pending) =>
                return self.progress_pending_first_stop(command, pending),
            OwnerLoginSession::PendingAccount(pending) =>
                return self.progress_pending_account(command, pending),
        };
        if active.halted {
            let output = if active.provider.is_some() { self.provider_display_output(&active) }
                else { active.output.clone() };
            let reply = owner_login_reply(command, "UNKNOWN", &output);
            self.owner_login = Some(OwnerLoginSession::Active(active));
            return Ok(reply);
        }
        if let Err(error) = self.append_owner_login_stderr(&mut active) {
            if !active.output.is_empty() { active.output.push('\n'); }
            active.output.push_str(&format!("owner login stderr: {error:?}"));
            let finished = self.finish_owner_device_login(command, active, false,
                Some(format!("owner login stderr: {error:?}")));
            return match finished {
                Ok(_) => Err(error),
                Err(stop_error) => Err(stop_error),
            };
        }
        if active.rpc.is_some() {
            let progress = self.advance_owner_login_rpc(&mut active);
            return match progress {
                Ok(None) => {
                    let reply = owner_login_reply(command, "PENDING", &active.output);
                    self.owner_login = Some(OwnerLoginSession::Active(active));
                    Ok(reply)
                }
                Ok(Some(failure)) => self.finish_owner_device_login(command, active, false, failure),
                Err(error) => {
                    let finished = self.finish_owner_device_login(command, active, false,
                        Some(error.clone()));
                    match finished {
                        Ok(_) => Err(OrchestrationError::V37StoreFailure(error)),
                        Err(stop_error) => Err(stop_error),
                    }
                }
            };
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
                    let finished = self.finish_owner_device_login(command, active, false,
                        Some("owner login output limit".into()));
                    return match finished {
                        Ok(_) => Err(OrchestrationError::Invalid("owner login output limit")),
                        Err(error) => Err(error),
                    };
                }
                if active.provider.as_ref().is_some_and(|provider| provider.driver_id == "opencode")
                    && provider_runtime::opencode_login_success_frame(output.bytes()) {
                    active.provider_completion_frame = true;
                }
                active.output.push_str(&String::from_utf8_lossy(output.bytes()));
                let visible = if active.provider.is_some() { self.provider_display_output(&active) }
                    else { active.output.clone() };
                let reply = owner_login_reply(command, "PENDING", &visible);
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
                if exited { self.finish_owner_device_login(command, active, false, None) }
                else {
                    let visible = if active.provider.is_some() { self.provider_display_output(&active) }
                        else { active.output.clone() };
                    let reply = owner_login_reply(command, "PENDING", &visible);
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
                if exited { self.finish_owner_device_login(command, active, false, None) }
                else {
                    let finished = self.finish_owner_device_login(command, active, false, None);
                    match finished {
                        Ok(_) => Err(OrchestrationError::Process(error)),
                        Err(stop_error) => Err(OrchestrationError::V37StoreFailure(format!(
                            "owner login stdout closed: {error:?}; stop: {stop_error:?}"))),
                    }
                }
            }
            Err(error) => {
                let finished = self.finish_owner_device_login(command, active, false, None);
                match finished {
                    Ok(_) => Err(OrchestrationError::Process(error)),
                    Err(stop_error) => Err(OrchestrationError::V37StoreFailure(format!(
                        "owner login output: {error:?}; stop: {stop_error:?}"))),
                }
            }
        }
    }

    // Only JSON-RPC frames from the retained first child may advance login.
    // Complete authUrl is the sole URL-bearing line returned to the host.
    fn advance_owner_login_rpc(&self, active: &mut ActiveOwnerLogin)
        -> std::result::Result<Option<Option<String>>, String> {
        let rpc = active.rpc.as_mut().ok_or("login RPC state missing")?;
        let process = self.process_custodian.active(&active.prepared.ticket)
            .ok_or("owned login process absent")?;
        if rpc.phase == LoginRpcPhase::SendInitialize {
            process.write_persistent_frame(INITIALIZE)
                .map_err(|error| format!("login initialize stdin: {error}; STDERR_TAIL: {}", process.stderr_tail()))?;
            rpc.phase = LoginRpcPhase::Initialize;
            rpc.response_started = Instant::now();
        }
        if matches!(rpc.phase, LoginRpcPhase::Initialize | LoginRpcPhase::Start)
            && rpc.response_started.elapsed() >= RPC_DEADLINE {
            return Err("login RPC deadline".into());
        }
        let frame = match self.process_custodian.read_persistent_child_frame(
            &active.prepared.ticket, Duration::from_millis(250)) {
            Ok(frame) => frame,
            Err(error) if protocol_timed_out(&error) => {
                if process.wait(Duration::ZERO).map_err(|error|
                    format!("login child wait: {error}; STDERR_TAIL: {}", process.stderr_tail()))? {
                    return Err(format!("login child exited before matched completion; STDERR_TAIL: {}", process.stderr_tail()));
                }
                return Ok(None);
            }
            Err(error) => return Err(format!("login RPC stdout: {error:?}; STDERR_TAIL: {}", process.stderr_tail())),
        };
        if frame.custody() != &active.prepared { return Err("login RPC custody mismatch".into()); }
        rpc.stdout_seen = rpc.stdout_seen.saturating_add(frame.bytes().len());
        rpc.frames_seen += 1;
        if rpc.stdout_seen.saturating_add(active.stderr_seen) > 65_536 || rpc.frames_seen > MAX_RPC_FRAMES {
            return Err("owner login output limit".into());
        }
        let mut fields = rpc_object(frame.bytes())?;
        let method = rpc_string(&mut fields, "method");
        if method.as_deref() == Some("account/login/completed") {
            if rpc.phase == LoginRpcPhase::Start && rpc.login_id.is_none() {
                if rpc.early_completion.is_some() { return Err("duplicate early login completion".into()); }
                rpc.early_completion = Some(frame.bytes().to_vec());
                return Ok(None);
            }
            if rpc.phase != LoginRpcPhase::Completion { return Err("unexpected login completion".into()); }
            let login_id = rpc.login_id.as_deref().ok_or("loginId missing")?;
            rpc.phase = LoginRpcPhase::Complete;
            return login_completion(frame.bytes(), login_id).map(Some);
        }
        if method.is_some() {
            if fields.contains_key(&JsonString::from_str("id")) {
                return Err("login notification carried RPC id".into());
            }
            // Account update notifications are not evidence of login completion.
            return Ok(None);
        }
        match rpc.phase {
            LoginRpcPhase::Initialize => {
                if rpc_frame_identity(frame.bytes(), "1") != RpcIdentity::Expected {
                    return Err(match login_rpc_vendor_error(frame.bytes()) {
                        Some(error) => format!("login initialize vendor error: {error}"),
                        None => "login initialize response identity".into(),
                    });
                }
                use crate::store::session_transport::codex_rpc::{self, Command, RpcId, Reply};
                let init_id = RpcId::client(1).map_err(|error| format!("login initialize ID: {error:?}"))?;
                let init_command = Command::Initialize { client_version: "0.1.0".into() };
                if !matches!(codex_rpc::decode(frame.bytes(), Some((&init_id, &init_command))),
                    Ok(Reply::Initialized { .. })) {
                    return Err("login initialize response shape".into());
                }
                process.write_persistent_frame(INITIALIZED)
                    .map_err(|error| format!("login initialized stdin: {error}; STDERR_TAIL: {}", process.stderr_tail()))?;
                process.write_persistent_frame(LOGIN_START)
                    .map_err(|error| format!("login start stdin: {error}; STDERR_TAIL: {}", process.stderr_tail()))?;
                rpc.phase = LoginRpcPhase::Start;
                rpc.response_started = Instant::now();
                Ok(None)
            }
            LoginRpcPhase::Start => {
                let (login_id, auth_url) = login_start_result(frame.bytes())?;
                if active.stderr_seen.saturating_add(rpc.stdout_seen) > 65_536 {
                    return Err("owner login output limit".into());
                }
                if !active.output.is_empty() && !active.output.ends_with('\n') { active.output.push('\n'); }
                active.output.push_str(&auth_url);
                active.output.push('\n');
                rpc.login_id = Some(login_id);
                rpc.phase = LoginRpcPhase::Completion;
                match rpc.early_completion.take() {
                    Some(early) => {
                        rpc.phase = LoginRpcPhase::Complete;
                        login_completion(&early, rpc.login_id.as_deref().unwrap()).map(Some)
                    }
                    None => Ok(None),
                }
            }
            _ => Err("unexpected login RPC response".into()),
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
                let output = if active.provider.is_some() { self.provider_display_output(&active) }
                    else { active.output.clone() };
                let reply = owner_login_reply(command, "UNKNOWN", &output);
                self.owner_login = Some(OwnerLoginSession::Active(active));
                Ok(reply)
            }
            OwnerLoginSession::Active(active) =>
                self.finish_owner_device_login(command, active, true, None),
            OwnerLoginSession::PendingFirstStop(pending) =>
                self.progress_pending_first_stop(command, pending),
            OwnerLoginSession::PendingAccount(pending) =>
                self.progress_pending_account(command, pending),
        }
    }

    fn release_pending_account_custody(&mut self, custody: &mut PendingAccountCustody) -> Result<bool> {
        if custody.released { return Ok(true); }
        let Some(prepared) = custody.prepared.as_ref() else { return Ok(false); };
        if custody.abort_prepared {
            self.process_custodian.abort_prepared(prepared)?;
            custody.released = true;
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
        custody.released = true;
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
        if let Some(continuation) = pending.continuation.as_ref() {
            let original = pending.custody.operation_id.as_deref()
                == Some(owner_login_operation_id(command).as_str());
            let provider_matches = match continuation.provider.as_ref() {
                Some(provider) => pending.custody.registered_driver.as_deref()
                    == Some(provider.driver_id.as_str())
                    && pending.custody.registered_home_identity.as_ref()
                        == Some(&provider.home.identity),
                None => pending.custody.registered_driver.as_deref() == Some("codex"),
            };
            if !pending.custody.released || !original || !provider_matches
                || pending.custody.runtime_home.is_none()
                || pending.custody.runtime_identity.is_none() {
                pending.latest_error = Some("confirmed login cleanup custody identity mismatch".into());
                self.owner_login = Some(OwnerLoginSession::PendingAccount(pending));
                return Err(OrchestrationError::AccessDenied);
            }
        }
        if let (Some(runtime), Some(identity)) = (&pending.custody.runtime_home,
            &pending.custody.runtime_identity) {
            let cleanup = if pending.custody.operation_id.as_deref()
                == Some(owner_login_operation_id(command).as_str()) {
                match (pending.custody.registered_driver.as_deref(),
                    pending.custody.registered_home_identity.as_ref()) {
                    (Some("codex"), _) => self.cleanup_confirmed_owner_login_runtime(
                        &pending.instance_id, runtime, identity),
                    (Some(driver), Some(home_identity)) => self.cleanup_confirmed_provider_login_runtime(
                        &pending.instance_id, driver, home_identity, runtime, identity),
                    _ => Err(OrchestrationError::AccessDenied),
                }
            } else { remove_owned_runtime(runtime, identity) };
            if let Err(error) = cleanup {
                pending.latest_error = Some(format!("account/read runtime cleanup: {error:?}"));
                let output = format!("{}\n{}", pending.output,
                    pending.latest_error.as_deref().unwrap());
                let reply = owner_login_reply(command, "UNKNOWN", &output);
                self.owner_login = Some(OwnerLoginSession::PendingAccount(pending));
                return Ok(reply);
            }
        }
        if let Some(previous) = pending.latest_error.take() {
            pending.output.push('\n');
            pending.output.push_str(&previous);
        }
        if let Some(mut continuation) = pending.continuation.take() {
            let state_result = if continuation.login_failure.is_none()
                || continuation.provider.is_some() {
                if let Some(provider) = continuation.provider.take() {
                    self.provider_login_account_state(command, provider,
                        continuation.completion_frame && !continuation.cancelled
                            && continuation.login_failure.is_none())
                } else { self.owner_login_account_state(command) }
            } else { Ok("UNKNOWN".to_owned()) };
            if let Some(OwnerLoginSession::PendingAccount(next)) = &mut self.owner_login {
                if !pending.output.is_empty() {
                    next.output = format!("{}\n{}", pending.output, next.output);
                }
                return Err(state_result.err().expect("pending account custody carries an error"));
            }
            let state = state_result.as_ref().cloned().unwrap_or_else(|_| "UNKNOWN".to_owned());
            if let Err(error) = &state_result {
                pending.output.push_str(&format!("\nautomatic account/read failed: {error:?}"));
            }
            let reply = owner_login_final_reply(command, &state, &pending.output);
            self.owner_login = Some(OwnerLoginSession::Final {
                instance_id: pending.instance_id, request_id: pending.request_id,
                expected_revision: pending.expected_revision, state, output: pending.output,
            });
            if let Some(failure) = continuation.login_failure {
                return Err(OrchestrationError::V37StoreFailure(failure));
            }
            if let Err(error) = state_result { return Err(error); }
            return Ok(reply);
        }
        let state = match (&pending.custody.request, &pending.custody.prepared,
            &pending.custody.frame) {
            (Some(request), Some(prepared), Some(frame)) => {
                match self.record_account_and_credential_backend(request, prepared, frame, pending.custody.backend_source.as_ref())
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

    fn progress_pending_first_stop(&mut self, command: &OwnerLoginCommand,
        mut pending: PendingFirstStop) -> Result<Vec<u8>> {
        if pending.durable_revision.is_none() {
            match authority::mark_process_stopped(&mut self.connection,
                &pending.active.operation_id, &pending.proof) {
                Ok(revision) => pending.durable_revision = Some(revision),
                Err(error) => {
                    pending.latest_error = Some(format!("{error:?}"));
                    let reply = owner_login_reply(command, "UNKNOWN", &pending_first_stop_output(&pending));
                    self.owner_login = Some(OwnerLoginSession::PendingFirstStop(pending));
                    return Ok(reply);
                }
            }
        }
        let revision = pending.durable_revision.expect("retained first-child stop revision");
        if let Err(error) = self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: pending.active.prepared.ticket.clone(),
            custodian_nonce: pending.active.prepared.custodian_nonce.clone(),
            identity: pending.active.prepared.identity.clone(),
            proof_hash: pending.proof.proof_hash(), durable_revision: revision,
        }) {
            pending.latest_error = Some(format!("{error:?}"));
            let reply = owner_login_reply(command, "UNKNOWN", &pending_first_stop_output(&pending));
            self.owner_login = Some(OwnerLoginSession::PendingFirstStop(pending));
            return Ok(reply);
        }
        let mut active = pending.active;
        if !active.output.is_empty() { active.output.push('\n'); }
        active.output.push_str(&pending.primary_error);
        if let Some(latest) = pending.latest_error {
            active.output.push('\n');
            active.output.push_str(&latest);
        }
        let finished = self.finish_confirmed_owner_login(command, active,
            pending.cancelled, pending.login_failure);
        match finished {
            Ok(reply) => Ok(reply),
            Err(error) => match &self.owner_login {
                Some(OwnerLoginSession::Final { state, output, .. }) =>
                    Ok(owner_login_final_reply(command, state, output)),
                _ => Err(error),
            },
        }
    }

    fn finish_owner_device_login(&mut self, command: &OwnerLoginCommand,
        mut active: ActiveOwnerLogin, cancelled: bool, inflight_failure: Option<String>) -> Result<Vec<u8>> {
        let mut inflight_failure = inflight_failure;
        if let Some(rpc) = &active.rpc {
            if cancelled {
                if let Some(login_id) = &rpc.login_id {
                    // Cancellation is advisory. The Job stop and later LPAC
                    // account/read remain authoritative even if OAuth wins.
                    let request = format!(
                        "{{\"id\":5,\"method\":\"account/login/cancel\",\"params\":{{\"loginId\":\"{login_id}\"}}}}\n");
                    let sent = self.process_custodian.active(&active.prepared.ticket)
                        .ok_or_else(|| "owned login process absent".to_owned())
                        .and_then(|process| process.write_persistent_frame(request.as_bytes())
                            .map_err(|error| error.to_string()));
                    if let Err(error) = sent {
                        let detail = format!("login cancel RPC stdin: {error}");
                        if !active.output.is_empty() { active.output.push('\n'); }
                        active.output.push_str(&detail);
                    }
                }
            } else if rpc.phase == LoginRpcPhase::Complete {
                if let Err(error) = self.process_custodian.close_child_input(&active.prepared.ticket) {
                    let detail = format!("login completed stdin close: {error:?}");
                    inflight_failure = Some(match inflight_failure {
                        Some(original) => format!("{original}; {detail}"),
                        None => detail,
                    });
                }
            }
        }
        let stop = self.process_custodian.stop(&active.prepared.ticket,
            StopBudgets::production(), || Ok(()));
        let proof = match stop {
            Ok(proof) => proof,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &active.operation_id, &active.prepared);
                if let Some(protocol) = &inflight_failure {
                    if !active.output.is_empty() { active.output.push('\n'); }
                    active.output.push_str(protocol);
                }
                active.halted = true;
                self.owner_login = Some(OwnerLoginSession::Active(active));
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "owner login stop: {error:?}; protocol: {inflight_failure:?}; unknown record: {unknown:?}")));
            }
        };
        let provider_stdout_failure = if active.provider.is_some()
            && proof.writer_fence_verified && proof.active_job_processes == Some(0) {
            self.append_provider_final_stdout(&mut active).err()
                .map(|error| format!("provider login stdout drain: {error:?}"))
        } else if active.provider.is_some() {
            Some("provider login stdout not final before writer fence".into())
        } else { None };
        let process = self.process_custodian.active(&active.prepared.ticket)
            .ok_or(OrchestrationError::AccessDenied)?;
        let drain_failure = if proof.writer_fence_verified && proof.active_job_processes == Some(0) {
            process.drain_stderr_after_writers_stopped().err()
                .map(|error| format!("owner login stderr drain: {error}"))
        } else { None };
        // The child may have exited before its stderr reader consumed the
        // final pipe bytes. Drain first; only then validate the live bytes.
        let stderr = process.stderr_tail();
        let live_failure = self.append_owner_login_stderr(&mut active).err()
            .map(|error| format!("owner login stderr: {error:?}"));
        let failures: Vec<String> = [inflight_failure, provider_stdout_failure, drain_failure, live_failure]
            .into_iter().flatten().collect();
        let stderr_capture_failure = (!failures.is_empty()).then(|| failures.join("; "));
        if let Some(error) = &stderr_capture_failure {
            if !active.output.is_empty() { active.output.push('\n'); }
            active.output.push_str(error);
        }
        // Exit failure is not an account-state observation. The original
        // stderr tail remains attached until durable confirmation releases it.
        // Cancellation still owns the original CLI diagnostic. It must not
        // discard a vendor error that arrived before the Owner cancelled.
        if cancelled {
            if !active.output.is_empty() { active.output.push('\n'); }
            active.output.push_str(&format!("owner login cancelled: STDERR_TAIL: {stderr}"));
        }
        let login_failure = owner_login_failure(cancelled, proof.exit_code, &stderr,
            stderr_capture_failure);
        let revision = match authority::mark_process_stopped(
            &mut self.connection, &active.operation_id, &proof,
        ) {
            Ok(revision) => revision,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &active.operation_id, &active.prepared);
                let cause = OrchestrationError::V37StoreFailure(format!(
                    "owner login stop record: {error:?}; CLI exit code={:?}; STDERR_TAIL: {stderr}; proof: {proof:?}; unknown record: {unknown:?}; login failure: {login_failure:?}", proof.exit_code));
                self.owner_login = Some(OwnerLoginSession::PendingFirstStop(PendingFirstStop {
                    active, proof, durable_revision: None, cancelled, login_failure,
                    primary_error: format!("{cause:?}"), latest_error: None,
                }));
                return Err(cause);
            }
        };
        if let Err(error) = self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: active.prepared.ticket.clone(),
            custodian_nonce: active.prepared.custodian_nonce.clone(),
            identity: active.prepared.identity.clone(),
            proof_hash: proof.proof_hash(),
            durable_revision: revision,
        }) {
            let cause: OrchestrationError = error.into();
            self.owner_login = Some(OwnerLoginSession::PendingFirstStop(PendingFirstStop {
                active, proof, durable_revision: Some(revision), cancelled, login_failure,
                primary_error: format!("{cause:?}"), latest_error: None,
            }));
            return Err(cause);
        }
        self.finish_confirmed_owner_login(command, active, cancelled, login_failure)
    }

    fn append_owner_login_stderr(&self, active: &mut ActiveOwnerLogin) -> Result<()> {
        let process = self.process_custodian.active(&active.prepared.ticket)
            .ok_or(OrchestrationError::AccessDenied)?;
        let bytes = process.stderr_live_bytes().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("owner login live stderr: {error}")))?;
        if let Some(rpc) = &active.rpc {
            if bytes.len() < active.stderr_seen {
                return Err(OrchestrationError::V37StoreFailure(
                    "owner login stderr capture regressed".into()));
            }
            if rpc.stdout_seen.saturating_add(bytes.len()) > 65_536 {
                return Err(OrchestrationError::V37StoreFailure("owner login output limit".into()));
            }
            if let Some(end) = bytes.iter().rposition(|byte| *byte == b'\n') {
                std::str::from_utf8(&bytes[..=end]).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("owner login stderr UTF-8: {error}")))?;
            }
            // app-server logs are retained in the child, not sent as display
            // text. Only the matched URL and vendor errors reach the host.
            active.stderr_seen = bytes.len();
            return Ok(());
        }
        append_complete_login_stderr(&mut active.output, &mut active.stderr_seen, &bytes)
            .map_err(OrchestrationError::V37StoreFailure)
    }

    fn cleanup_confirmed_owner_login_runtime(&self, instance_id: &str,
        runtime: &Path, identity: &RootIdentity) -> Result<()> {
            let home = instance::resolve_codex_instance_home(&self.connection, self.root, instance_id)
                .map_err(|error| OrchestrationError::V37StoreFailure(format!(
                    "login cleanup registered home: {error:?}")))?;
            if runtime.parent() != Some(home.path.as_path()) {
                return Err(OrchestrationError::AccessDenied);
            }
            checked_directory(runtime)?;
            let current_runtime = inspect_root(runtime).map_err(|error|
                OrchestrationError::V37StoreFailure(format!(
                    "login cleanup runtime identity: {error:?}")))?;
            if &current_runtime.identity != identity {
                return Err(OrchestrationError::AccessDenied);
            }
            // Preparation strictly verified the whole original home before
            // this fixed ordinary login ran. Only this Windows-generated
            // cache entry is unlinked after durable Job/writer stop, before
            // the unchanged strict LPAC account/read preparation.
            login_cache::remove_generated_cache_junction(self.root, &home)?;
            remove_owned_runtime(runtime, identity)
    }

    fn finish_confirmed_owner_login(&mut self, command: &OwnerLoginCommand,
        mut active: ActiveOwnerLogin, cancelled: bool, login_failure: Option<String>) -> Result<Vec<u8>> {
        let cleanup = if let Some(provider) = active.provider.as_ref() {
            self.cleanup_confirmed_provider_login_runtime(&active.instance_id,
                &provider.driver_id, &provider.home.identity,
                &active.runtime_home, &active.runtime_identity)
        } else {
            self.cleanup_confirmed_owner_login_runtime(&active.instance_id,
                &active.runtime_home, &active.runtime_identity)
        };
        if let Err(error) = cleanup {
            let (driver, home_identity) = match active.provider.as_ref() {
                Some(provider) => (provider.driver_id.clone(), Some(provider.home.identity.clone())),
                None => ("codex".to_owned(), None),
            };
            if !active.output.is_empty() { active.output.push('\n'); }
            active.output.push_str(&format!("owner login runtime cleanup: {error:?}"));
            if let Some(failure) = &login_failure {
                active.output.push('\n');
                active.output.push_str(failure);
            }
            self.owner_login = Some(OwnerLoginSession::PendingAccount(PendingAccountRead {
                instance_id: active.instance_id,
                request_id: active.request_id,
                expected_revision: active.expected_revision,
                output: active.output,
                latest_error: None,
                custody: PendingAccountCustody {
                    operation_id: Some(active.operation_id),
                    prepared: Some(active.prepared),
                    runtime_home: Some(active.runtime_home),
                    runtime_identity: Some(active.runtime_identity),
                    registered_driver: Some(driver),
                    registered_home_identity: home_identity,
                    proof: None,
                    durable_revision: None,
                    abort_prepared: false,
                    released: true,
                    frame: None,
                    backend_source: None,
                    credential_custody: None,
                    request: None,
                },
                continuation: Some(ConfirmedLoginContinuation {
                    provider: active.provider,
                    completion_frame: active.provider_completion_frame,
                    cancelled,
                    login_failure,
                }),
            }));
            return Err(error);
        }
        let state_result = if login_failure.is_none() || active.provider.is_some() {
            if let Some(provider) = active.provider.take() {
                self.provider_login_account_state(command, provider,
                    active.provider_completion_frame && !cancelled && login_failure.is_none())
            } else { self.owner_login_account_state(command) }
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
            return Err(OrchestrationError::V37StoreFailure(failure));
        }
        if let Err(error) = state_result { return Err(error); }
        if cancelled { return Ok(reply); }
        Ok(reply)
    }

    fn owner_login_account_state(&mut self, command: &OwnerLoginCommand) -> Result<String> {
        let row = self.read_registered_instance(&command.instance_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if row.driver_id != "codex" {
            let provider = match instance::provider_login::prepare_registered_provider_login(
                &mut self.connection, self.root, &self.owner, &command.instance_id)
                .map_err(|error| OrchestrationError::V37StoreFailure(format!(
                    "registered provider status preparation: {error:?}")))? {
                instance::provider_login::LoginPreparation::Ready(prepared) => prepared,
                instance::provider_login::LoginPreparation::Unsupported { .. } =>
                    return Ok("UNKNOWN".into()),
            };
            return self.provider_login_account_state(command, provider, false);
        }
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
        let receipt = match self.dispatch_owner_login_observation_inner(&observation, false) {
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
                        continuation: None,
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
        if row.driver_id != "codex" || row.version != "0.160.0"
            || row.program_digest != format!("sha256:{}",
                gogoke_lpac_path_compat::OBSERVED_CLI_SHA256)
        {
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
        self.recover_legacy_account_baseline(instance_id, &home, &profile)?;
        let (runtime, runtime_identity) = runtime_home(&home.path)?;
        let scope = (|| -> Result<_> {
            let mut environment = clean_environment(&home.path, &runtime)?;
            let login_environment = ordinary_login_environment(&home.path, &runtime)?;
            let credential_custody = self.grant_registered_account_observer(&profile,
                instance_id,&home,&runtime,&runtime_identity,&program)?;
            let module = CompatModule::prepare(self.root, &home.path, &home.identity,
                &profile, &profile_name).map_err(|source|
                    OrchestrationError::V37StoreFailure(format!("login path compatibility: {source}")))?;
            module.extend_environment(&mut environment);
            Ok((login_environment, environment, module, credential_custody))
        })();
        let (login_environment, environment, module, credential_custody) = match scope {
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
        login.arguments = vec![
            "-c".into(), "features.memories=false".into(),
            "-c".into(), "memories.generate_memories=false".into(),
            "-c".into(), "memories.use_memories=false".into(),
            "-c".into(), format!("sqlite_home={}",Json::String(JsonString::from_str(
                &runtime.to_string_lossy())).canonical()),
            "-c".into(), format!("log_dir={}",Json::String(JsonString::from_str(
                &runtime.to_string_lossy())).canonical()),
            "app-server".into(),
        ];
        login.current_directory = Some(runtime.clone());
        login.environment = Some(login_environment);
        // The official app-server returns its complete OAuth URL in the
        // account/login/start response, without opening a browser itself.
        // The host remains the only opener and sends no credential bytes.
        login.protocol_stdio = true;
        login.persistent_protocol_stdio = true;
        // The login app-server remains an ordinary same-user child. Only
        // account/read below enters LPAC.
        let mut account_read = ProcessLaunch::new(program);
        account_read.arguments = vec![
            "-c".into(), "features.memories=false".into(),
            "-c".into(), "memories.generate_memories=false".into(),
            "-c".into(), "memories.use_memories=false".into(),
            "-c".into(), format!("sqlite_home={}",Json::String(JsonString::from_str(
                &runtime.to_string_lossy())).canonical()),
            "-c".into(), format!("log_dir={}",Json::String(JsonString::from_str(
                &runtime.to_string_lossy())).canonical()),
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
            credential_custody,
            login: PrepareRequest { launch: login, binding: binding.clone() },
            account_read: PrepareRequest { launch: account_read, binding },
            runtime_home: runtime,
            runtime_identity,
            registered_driver: "codex".into(),
            registered_home_identity: home.identity,
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
        self.dispatch_owner_login_observation_selected(request, false)
    }

    /// Native-only binding of a new observer to the original instance's File
    /// configuration. The wire has no selector, and other modes are not converted.
    pub(super) fn dispatch_owner_file_backend_observation(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        let configured = instance::read_configured_credential_backend(&self.connection, &request.target_id)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("credential configured source: {error:?}")))?;
        if configured.backend != instance::CredentialBackend::File {
            return Err(OrchestrationError::Invalid("original instance is not configured for File credentials"));
        }
        self.dispatch_owner_login_observation_selected(request, true)
    }

    /// Host composition only: an existing signed-in instance is observed by
    /// the same fixed binary before selecting File for a new process. No login
    /// or credential parsing is performed, and non-File modes are not changed.
    pub(super) fn ensure_native_credential_backend(&mut self,instance_id:&str,
        original_request:&V37Request)->Result<()> {
        let registered=self.read_registered_instance(instance_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if registered.driver_id!="codex" || registered.login_state!="LOGGED_IN" {return Ok(());}
        match instance::read_usable_credential_backend(&self.connection,instance_id) {
            Ok(_)=>return Ok(()),
            Err(instance::CredentialRegistryError::Unusable)=>(),
            Err(error)=>return Err(OrchestrationError::V37StoreFailure(format!("credential startup source: {error:?}"))),
        }
        for file_bound in [false,true] {
            if !file_bound {
                match instance::read_configured_credential_backend(&self.connection,instance_id) {
                    Ok(configured) if configured.backend==instance::CredentialBackend::File=>continue,
                    Ok(_)=>return Err(OrchestrationError::Invalid("registered instance credential backend is not File")),
                    Err(instance::CredentialRegistryError::Unusable)=>(),
                    Err(error)=>return Err(OrchestrationError::V37StoreFailure(format!("credential configuration source: {error:?}"))),
                }
            }
            let revision=self.user_instance_revision(instance_id)?;
            let selector=if file_bound {"file-startup"} else {"original-config"};
            let id=format!("credential-observe-{}",crate::store::digest::sha256_hex(
                format!("{}\n{instance_id}\n{selector}\n{revision}",
                    crate::store::digest::sha256_hex(&original_request.raw_bytes)).as_bytes()));
            let text=|value:&str|Json::String(JsonString::from_str(value));
            let fields=BTreeMap::from([
                (JsonString::from_str("schema"),text("gogoke.37.operations.v1")),
                (JsonString::from_str("family"),text("K-INSTANCE")),
                (JsonString::from_str("operation"),text("login-state")),
                (JsonString::from_str("requestId"),text(&id)),
                (JsonString::from_str("targetId"),text(instance_id)),
                (JsonString::from_str("domainId"),text("global")),
                (JsonString::from_str("expectedRevision"),text(&revision.to_string())),
                (JsonString::from_str("payload"),Json::Object(BTreeMap::new())),
            ]);
            let raw=Json::Object(fields).canonical().into_bytes();
            let request=decode_request(&raw).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native credential observation request: {error:?}")))?;
            let receipt=if file_bound {self.dispatch_owner_file_backend_observation(&request)?}
                else {self.dispatch_owner_login_observation(&request)?};
            if owner_login_state_from_receipt(&receipt)?!="LOGGED_IN" {
                return Err(OrchestrationError::Invalid("native credential observation did not confirm account presence"));
            }
        }
        instance::read_usable_credential_backend(&self.connection,instance_id)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("native File startup qualification: {error:?}")))?;
        Ok(())
    }

    fn dispatch_owner_login_observation_selected(&mut self, request: &V37Request, file_bound: bool) -> Result<Vec<u8>> {
        if matches!(self.owner_login, Some(OwnerLoginSession::Active(_) |
            OwnerLoginSession::PendingFirstStop(_) | OwnerLoginSession::PendingAccount(_))) {
            return Err(OrchestrationError::OperationConflict);
        }
        match self.dispatch_owner_login_observation_inner(request, file_bound) {
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
                        continuation: None,
                    }));
                }
                Err(failure.error)
            }
        }
    }

    fn dispatch_owner_login_observation_inner(&mut self, request: &V37Request, file_bound: bool)
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
        let observed_version = self.read_registered_instance(&request.target_id)?
            .ok_or(OrchestrationError::AccessDenied)?.version;
        let mut prepared_login = self.prepare_owner_codex_login(&request.target_id)?;
        if file_bound {
            let args = &mut prepared_login.account_read.launch.arguments;
            if args.last().map(String::as_str) != Some("app-server") {
                let cleanup = remove_owned_runtime(&prepared_login.runtime_home,
                    &prepared_login.runtime_identity);
                return Err(retain_primary_cleanup_error(OrchestrationError::Invalid(
                    "credential observer is not the original app-server launch"), cleanup,
                    "credential observer selection").into());
            }
            let position = args.len() - 1;
            args.splice(position..position, ["-c".to_owned(), "cli_auth_credentials_store=\"file\"".to_owned()]);
        }
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
                registered_driver: None, registered_home_identity: None,
                proof, durable_revision, abort_prepared, released: false, frame: None, backend_source: None,
                credential_custody:prepared_login.credential_custody.clone(),
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
            let cleanup = remove_owned_runtime(&prepared_login.runtime_home,
                &prepared_login.runtime_identity);
            return Err(retain_primary_cleanup_error(error, cleanup,
                "login prepare record").into());
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
        let mut configured_backend = instance::CredentialBackend::Unknown;
        let execution = self.observe_account_with_credential_configuration(&prepared,
            &prepared_login.runtime_home, &mut configured_backend, file_bound);
        let backend_source = if execution.as_ref().is_ok_and(|frame|
            parse_account_read(frame.bytes()) == NativeAccountState::CredentialPresent) {
            Some(instance::BackendSource {
                request_id: request.request_id.clone(), instance_id: request.target_id.clone(),
                home_identity: prepared_login.registered_home_identity.clone(),
                program_digest: prepared.binding.binary_digest_sha256.clone(), version: observed_version,
                backend: configured_backend,
                startup_selector: if file_bound { instance::CredentialStartupSelector::FileBound }
                    else { instance::CredentialStartupSelector::Unknown },
                operation_id: operation_id.clone(), ticket: prepared.ticket.opaque().into(),
                nonce: prepared.custodian_nonce.clone(), generation: prepared.binding.generation.clone(),
            })
        } else { None };
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
                custody.backend_source = backend_source;
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
                custody.backend_source = backend_source;
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
            let cause = match execution.as_ref().err() {
                Some(primary) => OrchestrationError::V37StoreFailure(format!(
                    "account/read execution: {primary:?}; stop confirmation: {error:?}")),
                None => error.into(),
            };
            let mut custody = pending(Some(proof), Some(revision), false);
            custody.frame = execution.ok();
            custody.backend_source = backend_source;
            return Err(AccountObservationFailure { error: cause, pending: Some(custody) });
        }
        // Keep the original frame/source if settling either durable receipt
        // fails. Reconciliation uses this same stopped process, not a new read.
        let retained_frame = execution.as_ref().ok().cloned();
        match self.finish_confirmed_account_observation(request, &prepared,
            &prepared_login.runtime_home, &prepared_login.runtime_identity, execution, backend_source.clone()) {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                let mut custody = pending(Some(proof), Some(revision), false);
                custody.released = true;
                custody.frame = retained_frame;
                custody.backend_source = backend_source;
                // Cleanup is reconciled only while the original directory still
                // exists; a completed cleanup must not be attempted a second time.
                if !prepared_login.runtime_home.try_exists().map_err(|source|
                    OrchestrationError::V37StoreFailure(format!("account runtime existence: {source}")))? {
                    custody.runtime_home = None;
                    custody.runtime_identity = None;
                }
                Err(AccountObservationFailure { error, pending: Some(custody) })
            }
        }
    }

    fn finish_confirmed_account_observation(&mut self, request: &V37Request,
        prepared: &PreparedCustody, runtime: &Path, identity: &RootIdentity,
        execution: Result<OriginBoundFrame>, backend_source: Option<instance::BackendSource>) -> Result<Vec<u8>> {
        let cleanup = remove_owned_runtime(runtime, identity);
        let frame = match execution {
            Ok(frame) => { cleanup?; frame }
            Err(error) => return Err(retain_primary_cleanup_error(error, cleanup,
                "account/read execution")),
        };
        self.record_account_and_credential_backend(request, prepared, &frame, backend_source.as_ref())
    }

    fn record_account_and_credential_backend(&mut self, request: &V37Request,
        prepared: &PreparedCustody, frame: &OriginBoundFrame,
        backend_source: Option<&instance::BackendSource>) -> Result<Vec<u8>> {
        let receipt = match self.prior_login_state_request(request)? {
            Some(receipt) => receipt,
            None => self.record_trusted_account_read(request, prepared, frame)?,
        };
        if let Some(source) = backend_source.filter(|_| owner_login_state_from_receipt(&receipt)
            .is_ok_and(|state| state == "LOGGED_IN")) {
            if frame.custody() != prepared || source.request_id != request.request_id
                || source.instance_id != request.target_id || source.ticket != prepared.ticket.opaque()
                || source.nonce != prepared.custodian_nonce
                || parse_account_read(frame.bytes()) != NativeAccountState::CredentialPresent {
                return Err(OrchestrationError::AccessDenied);
            }
            instance::record_credential_backend(&mut self.connection, source)
                .map_err(|error| OrchestrationError::V37StoreFailure(format!("credential backend source: {error:?}")))?;
        }
        Ok(receipt)
    }

    fn observe_account_via_active_cli(&self, prepared: &PreparedCustody, cwd: &Path)
        -> Result<OriginBoundFrame> {
        let mut ignored = instance::CredentialBackend::Unknown;
        self.observe_account_with_credential_configuration(prepared, cwd, &mut ignored, false)
    }

    fn observe_account_with_credential_configuration(&self, prepared: &PreparedCustody, cwd: &Path,
        configured_backend: &mut instance::CredentialBackend, file_bound: bool) -> Result<OriginBoundFrame> {
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
        *configured_backend = configured_credential_backend(config.bytes())?;
        if file_bound && *configured_backend != instance::CredentialBackend::File {
            return Err(OrchestrationError::V37StoreFailure(format!(
                "File-bound observer effective credential configuration: {configured_backend:?}")));
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

/// Read only the non-secret selector in the already custody/codec-checked id3
/// frame. A missing key is not the fixed version's explicit default null.
fn configured_credential_backend(frame: &[u8]) -> Result<instance::CredentialBackend> {
    configured_credential_backend_for_id(frame,"3")
}

pub(super) fn configured_credential_backend_for_id(frame:&[u8],expected_id:&str)
    ->Result<instance::CredentialBackend> {
    let text = std::str::from_utf8(frame).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("credential configuration UTF8: {error}")))?;
    let parsed = Parser::parse(text.trim_end()).map_err(OrchestrationError::Atomic)?;
    let mut envelope = object(parsed).ok_or(OrchestrationError::Invalid("credential config envelope"))?;
    if !matches!(envelope.get(&JsonString::from_str("id")), Some(Json::Number(id)) if id == expected_id)
        || envelope.contains_key(&JsonString::from_str("error")) {
        return Err(OrchestrationError::Invalid("credential config original RPC identity"));
    }
    let mut result = envelope.remove(&JsonString::from_str("result")).and_then(object)
        .ok_or(OrchestrationError::Invalid("credential config result"))?;
    let mut config = result.remove(&JsonString::from_str("config")).and_then(object)
        .ok_or(OrchestrationError::Invalid("credential config object"))?;
    Ok(match config.remove(&JsonString::from_str("cli_auth_credentials_store")) {
        Some(Json::Null) => instance::CredentialBackend::File,
        Some(Json::String(value)) => match value.to_well_formed_string().as_deref() {
            Some("file") => instance::CredentialBackend::File,
            Some("keyring" | "auto" | "ephemeral") => instance::CredentialBackend::Other,
            _ => instance::CredentialBackend::Unknown,
        },
        _ => instance::CredentialBackend::Unknown,
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
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_rpc_waiting(phase: LoginRpcPhase) -> LoginRpc {
        LoginRpc { phase, login_id: None, early_completion: None,
            response_started: Instant::now(), stdout_seen: 0, frames_seen: 0 }
    }

    fn environment_value<'a>(environment: &'a [(String, String)], key: &str) -> Option<&'a str> {
        environment.iter().find(|(name, _)| name == key).map(|(_, value)| value.as_str())
    }

    fn callback_port_from_authorization_line(line: &str) -> Option<u16> {
        let query = line.strip_prefix("https://auth.openai.com/oauth/authorize?")?;
        let value = query.split('&').find_map(|field|
            field.split_once('=').and_then(|(key, value)| (key == "redirect_uri").then_some(value)))?;
        let bytes = value.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                let hex = bytes.get(index + 1..index + 3)?;
                let hex = std::str::from_utf8(hex).ok()?;
                decoded.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
        let redirect = String::from_utf8(decoded).ok()?;
        [1455u16, 1457u16].into_iter().find(|port|
            ["localhost", "127.0.0.1"].into_iter().any(|host|
                redirect == format!("http://{host}:{port}/auth/callback")))
    }

    fn cli_os_error_code(output: &str) -> Option<String> {
        output.lines().find_map(|line| {
            let value = line.split_once("os error ")?.1;
            let digits: String = value.chars().take_while(|character| character.is_ascii_digit()).take(6).collect();
            (!digits.is_empty()).then(|| format!("os error {digits}"))
        })
    }

    #[test]
    fn ordinary_login_stderr_waits_for_full_original_url_line_across_chunks() {
        let mut output = "existing stdout".to_owned();
        let mut seen = 0;
        let prefix = b"Open browser: https://auth.openai.com/oauth/authorize?client_id=ci&state=synthetic";
        let mut bytes = prefix.to_vec();
        bytes.extend_from_slice(&[0xE2]);
        append_complete_login_stderr(&mut output, &mut seen, &bytes).unwrap();
        assert_eq!(output, "existing stdout");
        assert_eq!(seen, 0);
        bytes.extend_from_slice(&[0x82, 0xAC, b'\r', b'\n']);
        append_complete_login_stderr(&mut output, &mut seen, &bytes).unwrap();
        assert_eq!(output, format!("existing stdout\n{}€\r\n", String::from_utf8_lossy(prefix)));
        assert_eq!(seen, bytes.len());
        append_complete_login_stderr(&mut output, &mut seen, &bytes).unwrap();
        assert_eq!(output.matches("oauth/authorize").count(), 1);
        bytes.extend_from_slice(b"vendor error before cancellation\r\n");
        append_complete_login_stderr(&mut output, &mut seen, &bytes).unwrap();
        assert!(output.ends_with("vendor error before cancellation\r\n"));
        assert_eq!(append_complete_login_stderr(&mut output, &mut seen, b"short"),
            Err("owner login stderr capture regressed".into()));
    }

    #[test]
    fn stderr_capture_failure_is_login_failure_even_after_zero_exit() {
        let mut output = String::new();
        let mut seen = 0;
        let capture = append_complete_login_stderr(&mut output, &mut seen, b"abc\xff\n").unwrap_err();
        let failure = owner_login_failure(false, Some(0), "", Some(capture));
        assert!(failure.as_deref().unwrap().contains("invalid utf-8 sequence"));
        // finish_confirmed_owner_login runs account/read only when the
        // login_failure argument is None. This failure must remain Some.
        assert!(failure.is_some());
        assert!(owner_login_failure(true, Some(0), "", None).is_none());
    }

    #[test]
    fn zero_exit_tail_stderr_settles_failure_without_cancellation_or_account_read() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-invalid-live-stderr-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity, .. } =
            product.prepare_owner_codex_login("instanceA").unwrap();
        // A signed system runtime is a controlled native pipe fixture only;
        // the production login command and registered auth home stay intact.
        let powershell = Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        login.binding.binary_digest_sha256 = format!("sha256:{}",
            crate::store::digest::sha256_hex(&fs::read(&powershell).unwrap()));
        login.launch.application = powershell;
        login.launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            "[Console]::Error.Write('x' * 5000); [Console]::Error.Flush(); $stderr=[Console]::OpenStandardError(); $stderr.WriteByte(255); $stderr.WriteByte(10); $stderr.Flush(); exit 0".into()];
        login.launch.app_container_profile = None;
        login.launch.app_container_internet_client = false;
        login.launch.app_container_cli_identity_services = false;
        login.launch.environment = None;
        login.launch.path_compat = None;
        let prepared = product.process_custodian.prepare(&login).unwrap();
        drop(login);
        drop(account_read);
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"invalidStderrA","expectedRevision":1}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        let child = product.process_custodian.active(&prepared.ticket).unwrap();
        assert!(child.wait(Duration::from_secs(15)).unwrap(), "controlled child did not exit");
        assert_eq!(child.exit_code().unwrap(), Some(0), "controlled child must exit zero");
        product.owner_login = Some(OwnerLoginSession::Active(ActiveOwnerLogin {
            instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
            expected_revision: command.expected_revision, operation_id, prepared,
            runtime_home, runtime_identity, output: String::new(), stderr_seen: 0, halted: false,
            rpc: Some(test_rpc_waiting(LoginRpcPhase::Initialize)), provider: None,
            provider_completion_frame: false,
        }));
        let deadline = Instant::now() + Duration::from_secs(45);
        let mut settled = None;
        while Instant::now() < deadline {
            if let Ok(reply) = product.status_owner_device_login(&command) {
                let reply = String::from_utf8(reply).unwrap();
                if reply.contains("\"settled\":true") { settled = Some(reply); break; }
            }
        }
        let reply = settled.expect("invalid stderr did not settle original request");
        assert!(reply.contains("\"state\":\"UNKNOWN\""));
        assert!(reply.contains("invalid utf-8 sequence"));
        assert!(!reply.contains("owner login cancelled"));
        assert_eq!(scalar(&product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE operation_id LIKE 'login-observe-%'"), "0",
            "capture failure must not start account/read or report LOGGED_IN");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn mixed_login_output_limit_is_failure_even_when_child_exits_zero() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-mixed-login-limit-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity, .. } =
            product.prepare_owner_codex_login("instanceA").unwrap();
        let powershell = Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        login.binding.binary_digest_sha256 = format!("sha256:{}",
            crate::store::digest::sha256_hex(&fs::read(&powershell).unwrap()));
        login.launch.application = powershell;
        login.launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(),
            r#"[Console]::Error.WriteLine('x' * 64000); [Console]::Error.Flush(); [Console]::Out.WriteLine('{"id":1,"result":{"padding":"' + ('y' * 3000) + '"}}'); exit 0"#.into()];
        login.launch.app_container_profile = None;
        login.launch.app_container_internet_client = false;
        login.launch.app_container_cli_identity_services = false;
        login.launch.environment = None;
        login.launch.path_compat = None;
        let prepared = product.process_custodian.prepare(&login).unwrap();
        drop(login);
        drop(account_read);
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"mixedLimitA","expectedRevision":1}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        let child = product.process_custodian.active(&prepared.ticket).unwrap();
        assert!(child.wait(Duration::from_secs(15)).unwrap());
        assert_eq!(child.exit_code().unwrap(), Some(0));
        let _tail = child.stderr_tail(); // drain the exact child's written stderr before status
        product.owner_login = Some(OwnerLoginSession::Active(ActiveOwnerLogin {
            instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
            expected_revision: command.expected_revision, operation_id, prepared,
            runtime_home, runtime_identity, output: String::new(), stderr_seen: 0, halted: false,
            rpc: Some(test_rpc_waiting(LoginRpcPhase::Initialize)), provider: None,
            provider_completion_frame: false,
        }));
        let _ = product.status_owner_device_login(&command);
        let reply = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(reply.contains("\"settled\":true"));
        assert!(reply.contains("\"state\":\"UNKNOWN\""));
        assert!(reply.contains("owner login output limit"));
        assert!(!reply.contains("owner login cancelled"));
        assert_eq!(scalar(&product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE operation_id LIKE 'login-observe-%'"), "0",
            "mixed stream output limit must not run account/read");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn ordinary_callback_port_comes_from_complete_cli_redirect_uri() {
        // Official 0.160 uses the IPv4 loopback literal; retain the earlier
        // spelling for historical protocol fixtures, with the same endpoint.
        for port in [1455u16, 1457u16] {
            assert_eq!(callback_port_from_authorization_line(&format!(
                "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2F127.0.0.1%3A{port}%2Fauth%2Fcallback")), Some(port));
        }
        for redirect in ["http%3A%2F%2F127.0.0.2%3A1455%2Fauth%2Fcallback",
            "http%3A%2F%2F127.0.0.1%3A1456%2Fauth%2Fcallback",
            "http%3A%2F%2F127.0.0.1%3A1455%2Fother"] {
            assert_eq!(callback_port_from_authorization_line(&format!(
                "https://auth.openai.com/oauth/authorize?redirect_uri={redirect}")), None);
        }
        assert_eq!(callback_port_from_authorization_line(
            "https://auth.openai.com/oauth/authorize?state=synthetic&redirect_uri=http%3A%2F%2Flocalhost%3A1457%2Fauth%2Fcallback"),
            Some(1457));
        assert_eq!(callback_port_from_authorization_line(
            "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"),
            Some(1455));
        assert_eq!(callback_port_from_authorization_line(
            "https://auth.openai.com/oauth/authorize?redirect_uri=http%3A%2F%2Fother%3A1455%2Fauth%2Fcallback"),
            None);
        assert_eq!(cli_os_error_code("Error logging in: Permission denied (os error 10013)"),
            Some("os error 10013".into()));
    }

    #[test]
    fn app_server_login_accepts_only_matched_full_url_and_completion() {
        let started = br#"{"id":4,"result":{"type":"chatgpt","loginId":"01234567-89ab-cdef-0123-456789abcdef","authUrl":"https://auth.openai.com/oauth/authorize?state=synthetic&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"}}"#;
        let (login_id, url) = login_start_result(started).unwrap();
        assert_eq!(callback_port_from_authorization_line(&url), Some(1455));
        assert!(login_start_result(br#"{"id":3,"result":{"type":"chatgpt","loginId":"a","authUrl":"https://auth.openai.com/oauth/authorize?x=y"}}"#).is_err());
        assert!(login_start_result(br#"{"id":4,"result":{"type":"chatgpt","loginId":"a","authUrl":"https://auth.openai.com/oauth/authorize?x=y\nsecond-line"}}"#).is_err());
        let completed = br#"{"method":"account/login/completed","params":{"loginId":"01234567-89ab-cdef-0123-456789abcdef","success":true,"error":null}}"#;
        assert_eq!(login_completion(completed, &login_id), Ok(None));
        assert!(login_completion(completed, "other-id").is_err());
        let failed = br#"{"method":"account/login/completed","params":{"loginId":"01234567-89ab-cdef-0123-456789abcdef","success":false,"error":"vendor denied"}}"#;
        assert_eq!(login_completion(failed, &login_id), Ok(Some("vendor denied".into())));
    }

    #[test]
    fn owned_login_rpc_completion_closes_stdin_and_reads_real_lpac_account() {
        let _guard = route_b_test_guard();
        for early in [false, true] {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!(
                "gogoke-v37-rpc-eof-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            let root = RootLock::acquire(&path).unwrap();
            let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
            let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
            assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
                V37Status::Applied);
            let mut launch = product.prepare_owner_codex_login("instanceA").unwrap();
            let powershell = Path::new(&std::env::var("SystemRoot").unwrap())
                .join("System32/WindowsPowerShell/v1.0/powershell.exe");
            launch.login.binding.binary_digest_sha256 = format!("sha256:{}",
                crate::store::digest::sha256_hex(&fs::read(&powershell).unwrap()));
            launch.login.launch.application = powershell;
            let script = format!(r#"
$null = [Console]::In.ReadLine()
[Console]::Out.WriteLine('{{"id":1,"result":{{"userAgent":"controlled"}}}}')
$null = [Console]::In.ReadLine()
$null = [Console]::In.ReadLine()
$started = '{{"id":4,"result":{{"type":"chatgpt","loginId":"01234567-89ab-cdef-0123-456789abcdef","authUrl":"https://auth.openai.com/oauth/authorize?state=synthetic&redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"}}}}'
$completed = '{{"method":"account/login/completed","params":{{"loginId":"01234567-89ab-cdef-0123-456789abcdef","success":true,"error":null}}}}'
if ({early}) {{ [Console]::Out.WriteLine($completed); [Console]::Out.WriteLine($started) }}
else {{ [Console]::Out.WriteLine($started); [Console]::Out.WriteLine($completed) }}
while ($null -ne [Console]::In.ReadLine()) {{}}
exit 0
"#, early = if early { "$true" } else { "$false" });
            launch.login.launch.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(),
                "-Command".into(), script];
            launch.login.launch.environment = None;
            let begin = owner_login_command(format!(
                "{{\"schema\":\"gogoke.37.owner-login.v1\",\"action\":\"begin\",\"instanceId\":\"instanceA\",\"requestId\":\"rpcEof{early}\",\"expectedRevision\":1}}")
                .as_bytes()).unwrap();
            let status = owner_login_command(format!(
                "{{\"schema\":\"gogoke.37.owner-login.v1\",\"action\":\"status\",\"instanceId\":\"instanceA\",\"requestId\":\"rpcEof{early}\",\"expectedRevision\":1}}")
                .as_bytes()).unwrap();
            let begin_reply = product.start_owner_device_login(&begin, launch,
                |custodian, prepared| custodian.activate(prepared)).unwrap();
            assert!(String::from_utf8(begin_reply).unwrap().contains("\"state\":\"PENDING\""));
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut saw_complete_url = false;
            let final_reply = loop {
                assert!(Instant::now() < deadline, "controlled real Job app-server protocol did not settle");
                let reply = String::from_utf8(product.status_owner_device_login(&status).unwrap()).unwrap();
                if reply.contains("auth.openai.com/oauth/authorize?") {
                    saw_complete_url = true;
                }
                if reply.contains("\"settled\":true") { break reply; }
            };
            assert!(saw_complete_url, "complete URL must reach the private reply");
            assert!(final_reply.contains("\"state\":\"LOGGED_OUT\""),
                "synthetic completion cannot substitute for the real LPAC account/read");
            assert_eq!(scalar(&product,
                "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "2",
                "first Job and automatic LPAC account/read must both durably stop");
            product.close_checked().unwrap();
            drop(root);
            fs::remove_dir_all(path).unwrap();
        }
    }

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
    fn credential_configuration_preserves_missing_default_and_non_file_sources() {
        for value in ["null", "\"file\""] {
            let frame = format!("{{\"id\":3,\"result\":{{\"config\":{{\"cli_auth_credentials_store\":{value}}}}}}}");
            assert_eq!(configured_credential_backend(frame.as_bytes()).unwrap(), instance::CredentialBackend::File);
        }
        for value in ["\"keyring\"", "\"auto\"", "\"ephemeral\""] {
            let frame = format!("{{\"id\":3,\"result\":{{\"config\":{{\"cli_auth_credentials_store\":{value}}}}}}}");
            assert_eq!(configured_credential_backend(frame.as_bytes()).unwrap(), instance::CredentialBackend::Other);
        }
        assert_eq!(configured_credential_backend(br#"{"id":3,"result":{"config":{}}}"#).unwrap(),
            instance::CredentialBackend::Unknown);
        assert!(configured_credential_backend(br#"{"id":"3","result":{"config":{"cli_auth_credentials_store":"file"}}}"#).is_err());
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
    fn legacy_original_home_capture_refuses_same_boot_preserves_unknown_and_reopens_exact_fence() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-legacy-fence-product-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let registration = request("register", "registerLegacyA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&registration).unwrap()).unwrap().status, V37Status::Applied);
        let home = instance::resolve_codex_instance_home(&product.connection, &root, "instanceA").unwrap();
        let source = home.path.join("auth.json");
        fs::write(&source, b"synthetic opaque credential fixture, not an account").unwrap();
        let profile = AppContainerProfile::derive_for_revocation(&owner_login_profile_name("instanceA", &home.identity)).unwrap();
        profile.grant_bound_tree(&home.path, &home.identity, true).unwrap();
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('oldLoginA','oldTicketA','oldNonceA','111','222','synthetic.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','global','1','UNKNOWN',NULL)").unwrap();
        let before_source = crate::process::CredentialBinding::observe_source_metadata(&root, &source, &home.identity).unwrap();
        let error = product.recover_legacy_account_baseline("instanceA", &home, &profile).unwrap_err();
        assert!(format!("{error:?}").contains("Windows system restart required"));
        let original = instance::read_legacy_fence(&product.connection, "instanceA").unwrap().unwrap();
        assert_eq!(original.custody.len(), 1);
        assert_eq!(original.custody[0].state, "UNKNOWN");
        assert!(original.custody[0].stop_proof_hash.is_none());
        assert!(instance::read_legacy_step(&product.connection, "instanceA", instance::LegacyAclStep::Home).unwrap().is_none());
        assert!(profile.has_legacy_owner_login_grant(&home.path, &home.identity).unwrap(), "same boot must not write ACLs");
        assert_eq!(crate::process::CredentialBinding::observe_source_metadata(&root, &source, &home.identity).unwrap(), before_source);
        assert_eq!(fs::read(&source).unwrap(), b"synthetic opaque credential fixture, not an account");
        assert!(!home.path.join("gogoke-login-runtime").exists(), "capture precedes temporary runtime creation");
        product.close_checked().unwrap();
        let mut reopened = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        assert!(format!("{:?}", reopened.recover_legacy_account_baseline("instanceA", &home, &profile).unwrap_err()).contains("Windows system restart required"));
        let replay = instance::read_legacy_fence(&reopened.connection, "instanceA").unwrap().unwrap();
        assert_eq!(replay.original_boot, original.original_boot);
        assert_eq!(replay.native_snapshot_digest, original.native_snapshot_digest);
        assert_eq!(replay.custody, original.custody);
        assert_eq!(replay.revision, original.revision);
        assert_eq!(scalar(&reopened, "SELECT state FROM main.gogoke_coordination_process_custody WHERE operation_id='oldLoginA'"), "UNKNOWN");
        reopened.close_checked().unwrap(); drop(root);
        fs::remove_dir_all(path).unwrap();
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
        assert_eq!(registered, br#"{"instances":[{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceA","loginState":"UNKNOWN","revision":"1","version":"0.160.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceB","loginState":"UNKNOWN","revision":"1","version":"0.160.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceC","loginState":"UNKNOWN","revision":"1","version":"0.160.0"}],"schema":"gogoke.37.instance-list.v1"}"#,
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
        assert_eq!(observed, br#"{"instances":[{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceA","loginState":"LOGGED_OUT","revision":"2","version":"0.160.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceB","loginState":"LOGGED_IN","revision":"2","version":"0.160.0"},{"driverId":"codex","installState":"INSTALLED","instanceId":"instanceC","loginState":"UNKNOWN","revision":"1","version":"0.160.0"}],"schema":"gogoke.37.instance-list.v1"}"#);
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
    fn pinned_codex_owner_login_stop_failure_reconciles_same_proof_and_stderr() {
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
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity, .. } =
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
            output: String::new(), stderr_seen: 0, halted: false, rpc: None, provider: None,
            provider_completion_frame: false,
        }));
        let active = String::from_utf8(product.begin_owner_device_login(&command).unwrap()).unwrap();
        assert!(active.contains("\"state\":\"PENDING\""));
        assert!(active.contains("\"settled\":false"), "active login must retain its request");
        product.connection.execute("CREATE TRIGGER fail_first_stop BEFORE UPDATE OF state ON gogoke_coordination_process_custody WHEN NEW.state='STOPPED' AND NEW.operation_id LIKE 'owner-login-%' BEGIN SELECT RAISE(ABORT,'controlled first CLI stop record failure'); END").unwrap();
        let error = product.status_owner_device_login(&command).unwrap_err();
        let diagnostic = format!("{error:?}");
        assert!(diagnostic.contains("controlled first CLI stop record failure"));
        assert!(diagnostic.contains("code=Some(2)"), "actual exit code was not preserved");
        assert!(diagnostic.contains("STDERR_TAIL:"));
        assert!(diagnostic.contains("unexpected argument"), "actual CLI stderr was not preserved");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "1",
            "a failed login must not start a second CLI for account/read");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "0");
        let held = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(held.contains("\"state\":\"UNKNOWN\""));
        assert!(held.contains("\"settled\":false"));
        assert!(held.contains("controlled first CLI stop record failure"));
        assert!(held.contains("unexpected argument"));
        let new_begin = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"newWhileFirstStopHeld","expectedRevision":1}"#;
        assert!(matches!(product.dispatch_owner_login_frame(new_begin),
            Err(OrchestrationError::OperationConflict)));
        product.connection.execute("DROP TRIGGER fail_first_stop").unwrap();
        let replay = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(replay.contains("\"state\":\"UNKNOWN\""));
        assert!(replay.contains("\"settled\":true"));
        assert!(replay.contains("controlled first CLI stop record failure"));
        assert!(replay.contains("unexpected argument"));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1");
        assert!(!runtime_home.exists(), "owned login runtime must be cleaned after confirmation");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody"), "1");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn cancelled_pinned_cli_preserves_stderr_before_confirmed_release() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-cancel-stderr-{}-{nonce}",
            std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity, .. } =
            product.prepare_owner_codex_login("instanceA").unwrap();
        login.launch.arguments = vec!["login".into(), "--gogoke-invalid-login-control".into()];
        let prepared = product.process_custodian.prepare(&login).unwrap();
        drop(login);
        drop(account_read);
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"cancel","instanceId":"instanceA","requestId":"ownerCancelStderrA","expectedRevision":1}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        assert!(product.process_custodian.active(&prepared.ticket).unwrap()
            .wait(Duration::from_secs(15)).unwrap());
        product.owner_login = Some(OwnerLoginSession::Active(ActiveOwnerLogin {
            instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
            expected_revision: command.expected_revision, operation_id, prepared,
            runtime_home: runtime_home.clone(), runtime_identity, output: String::new(), stderr_seen: 0, halted: false, rpc: None, provider: None,
            provider_completion_frame: false,
        }));
        let reply = match product.cancel_owner_device_login(&command) {
            Ok(reply) => reply,
            Err(error) => {
                assert!(matches!(product.owner_login, Some(OwnerLoginSession::Final { .. })),
                    "cancellation retained unexpected custody: {error:?}");
                product.status_owner_device_login(&command).unwrap()
            }
        };
        let reply = String::from_utf8(reply).unwrap();
        assert!(reply.contains("\"settled\":true"), "original cancellation did not settle: {reply}");
        assert!(reply.contains("unexpected argument"), "actual fixed CLI stderr was discarded: {reply}");
        assert!(reply.contains("owner login cancelled: STDERR_TAIL:"));
        assert!(!runtime_home.exists());
        assert_eq!(scalar(&product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state!='STOPPED'"), "0");
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_account_cleanup_identity_denial_retains_original_failure() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-account-cleanup-{}-{nonce}", std::process::id()));
        let runtime = path.join("runtime");
        let unrelated = path.join("unrelated");
        fs::create_dir_all(&runtime).unwrap();
        fs::create_dir(&unrelated).unwrap();
        let runtime_identity = inspect_root(&runtime).unwrap().identity;
        let unrelated_identity = inspect_root(&unrelated).unwrap().identity;
        assert_ne!(runtime_identity, unrelated_identity);
        let cleanup = remove_owned_runtime(&runtime, &unrelated_identity);
        assert!(matches!(&cleanup, Err(OrchestrationError::AccessDenied)));
        assert!(runtime.is_dir(), "mismatched identity must not remove the runtime");
        let combined = retain_primary_cleanup_error(
            OrchestrationError::V37StoreFailure("original account/read RPC failure".into()),
            cleanup, "account/read execution");
        let diagnostic = format!("{combined:?}");
        assert!(diagnostic.contains("original account/read RPC failure"));
        assert!(diagnostic.contains("AccessDenied"));
        remove_owned_runtime(&runtime, &runtime_identity).unwrap();
        fs::remove_dir(&unrelated).unwrap();
        fs::remove_dir(&path).unwrap();
    }

    #[test]
    fn owner_account_execution_and_cleanup_failures_preserve_both_original_causes() {
        use std::os::windows::fs::OpenOptionsExt;

        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-account-cleanup-cli-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let register = request("register", "registerA", 0, r#"{"driverId":"codex"}"#);
        assert_eq!(decode_receipt(&product.register_user_instance(&register).unwrap()).unwrap().status,
            V37Status::Applied);
        let mut launch = product.prepare_owner_codex_login("instanceA").unwrap();
        // The unchanged pinned CLI itself rejects this argument before it can
        // return an account/read frame; this is a real child protocol failure.
        launch.account_read.launch.arguments = vec![
            "app-server".into(), "--gogoke-invalid-account-control".into()];
        let prepared = product.process_custodian.prepare(&launch.account_read).unwrap();
        let operation_id = "login-observe-cleanup-control";
        authority::record_prepared_process(&mut product.connection, operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, operation_id, &prepared).unwrap();
        assert!(product.process_custodian.active(&prepared.ticket).unwrap()
            .wait(Duration::from_secs(15)).unwrap(), "pinned account/read parser control did not exit");
        let execution = product.observe_account_via_active_cli(&prepared, &launch.runtime_home);
        let native_error = match execution.as_ref() {
            Err(error) => format!("{error:?}"),
            Ok(_) => panic!("actual pinned CLI parser control should fail"),
        };
        assert!(native_error.contains("unexpected argument"), "real CLI stderr must be retained");
        let proof = product.process_custodian.stop(&prepared.ticket,
            StopBudgets::production(), || Ok(())).unwrap();
        let revision = authority::mark_process_stopped(&mut product.connection,
            operation_id, &proof).unwrap();
        product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
            identity: prepared.identity.clone(), proof_hash: proof.proof_hash(),
            durable_revision: revision,
        }).unwrap();
        let sentinel = launch.runtime_home.join("held-cleanup-control.bin");
        fs::write(&sentinel, b"owned test cleanup control").unwrap();
        // FILE_SHARE_READ deliberately excludes FILE_SHARE_DELETE. This real
        // Windows handle prevents remove_owned_runtime from deleting sentinel.
        let held = fs::OpenOptions::new().read(true).share_mode(0x1).open(&sentinel).unwrap();
        let observation = request("login-state", "cleanupControl", 1, "{}");
        let error = product.finish_confirmed_account_observation(&observation,
            &prepared, &launch.runtime_home, &launch.runtime_identity, execution, None).unwrap_err();
        let diagnostic = format!("{error:?}");
        assert!(diagnostic.contains("unexpected argument"), "CLI's original execution error was lost");
        assert!(diagnostic.contains("runtime cleanup"), "actual Windows delete failure was lost");
        assert!(sentinel.is_file());
        assert_eq!(scalar(&product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1");
        drop(held);
        remove_owned_runtime(&launch.runtime_home, &launch.runtime_identity).unwrap();
        drop(launch);
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
        for kind in ["digest","prepared","resume","active","ordinary_active"] {
            product.owner_login = None;
            let revision = product.user_instance_revision("instanceA").unwrap();
            let frame = format!("{{\"schema\":\"gogoke.37.owner-login.v1\",\"action\":\"begin\",\"instanceId\":\"instanceA\",\"requestId\":\"first_{kind}\",\"expectedRevision\":{revision}}}");
            let command = owner_login_command(frame.as_bytes()).unwrap();
            let mut launch = product.prepare_owner_codex_login("instanceA").unwrap();
            if kind != "ordinary_active" { launch.login.launch.arguments = vec!["--version".into()]; }
            let original_home = launch.runtime_home.parent().unwrap().to_path_buf();
            let generated_cache = original_home.join("AppData/Local/Microsoft/Windows/INetCache/Content.IE5");
            if kind == "digest" { launch.login.binding.binary_digest_sha256 = format!("sha256:{}","0".repeat(64)); }
            if kind == "prepared" {
                product.connection.execute("CREATE TRIGGER fail_first_prepare BEFORE INSERT ON gogoke_coordination_process_custody WHEN NEW.operation_id LIKE 'owner-login-%' BEGIN SELECT RAISE(ABORT,'controlled first prepare record failure'); END").unwrap();
            }
            if matches!(kind, "active" | "ordinary_active") {
                product.connection.execute("CREATE TRIGGER fail_first_active BEFORE UPDATE OF state ON gogoke_coordination_process_custody WHEN NEW.state='ACTIVE' AND NEW.operation_id LIKE 'owner-login-%' BEGIN SELECT RAISE(ABORT,'controlled first active record failure'); END").unwrap();
            }
            let error = if kind == "resume" {
                product.start_owner_device_login(&command,launch,|custodian,prepared|custodian.activate_with_failed_resume_for_test(prepared)).unwrap_err()
            } else if kind == "ordinary_active" {
                // Preserve the authentic first-child ACTIVE fault after the
                // fixed app-server returns a complete OAuth URL. This uses
                // the same retained process and pipe as production.
                product.start_owner_device_login(&command, launch, |custodian, prepared| {
                    let activated = custodian.activate(prepared)?;
                    let deadline = Instant::now() + Duration::from_secs(30);
                    custodian.active(&prepared.ticket).unwrap()
                        .write_persistent_frame(INITIALIZE).unwrap();
                    loop {
                        assert!(Instant::now() < deadline, "actual fixed CLI initialize timed out before ACTIVE fault");
                        match custodian.read_persistent_child_frame(&prepared.ticket, Duration::from_millis(250)) {
                            Ok(frame) if rpc_frame_identity(frame.bytes(), "1") == RpcIdentity::Expected => break,
                            Ok(frame) if rpc_frame_identity(frame.bytes(), "1") == RpcIdentity::Notification => (),
                            Err(error) if protocol_timed_out(&error) => (),
                            _ => panic!("actual fixed CLI initialize response identity before ACTIVE fault"),
                        }
                    }
                    custodian.active(&prepared.ticket).unwrap()
                        .write_persistent_frame(INITIALIZED).unwrap();
                    custodian.active(&prepared.ticket).unwrap()
                        .write_persistent_frame(LOGIN_START).unwrap();
                    let ready = loop {
                        assert!(Instant::now() < deadline, "actual fixed CLI login start timed out before ACTIVE fault");
                        match custodian.read_persistent_child_frame(&prepared.ticket, Duration::from_millis(250)) {
                            Ok(frame) if rpc_frame_identity(frame.bytes(), "4") == RpcIdentity::Expected =>
                                break login_start_result(frame.bytes()).is_ok(),
                            Ok(frame) if rpc_frame_identity(frame.bytes(), "4") == RpcIdentity::Notification => (),
                            Err(error) if protocol_timed_out(&error) => (),
                            _ => panic!("actual fixed CLI login start response identity before ACTIVE fault"),
                        }
                    };
                    assert!(ready, "actual fixed CLI must return complete private authUrl before ACTIVE fault");
                    Ok(activated)
                }).unwrap_err()
            } else if kind == "active" {
                // This existing factory-failure fixture uses the real fixed
                // CLI, with a controlled real mount-point at the OS cache
                // path. Ordinary login does not always create this entry;
                // the separate real OAuth test covers that authentic flow.
                product.start_owner_device_login(&command, launch, |custodian, prepared| {
                    let activated = custodian.activate(prepared)?;
                    fs::create_dir_all(generated_cache.parent().unwrap()).unwrap();
                    let target = path.join("owned-active-failure-cache-target");
                    fs::create_dir(&target).unwrap();
                    fs::write(target.join("sentinel"), b"retained controlled cache target").unwrap();
                    let junction = std::process::Command::new("cmd.exe")
                        .args(["/D", "/C", "mklink", "/J"])
                        .arg(&generated_cache).arg(&target).output().unwrap();
                    assert!(junction.status.success(), "controlled ACTIVE cache fixture: {}",
                        String::from_utf8_lossy(&junction.stderr)
                            .replace(path.to_string_lossy().as_ref(), "<test-root>"));
                    let observed = fs::symlink_metadata(&generated_cache)
                        .expect("controlled physical cache entry before ACTIVE failure");
                    assert_eq!(observed.file_attributes() & (REPARSE_POINT | 0x10), REPARSE_POINT | 0x10);
                    Ok(activated)
                }).unwrap_err()
            } else {
                product.start_owner_device_login(&command,launch,|custodian,prepared|custodian.activate(prepared)).unwrap_err()
            };
            let cause = format!("{error:?}");
            let status = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
            assert!(status.contains("\"settled\":true"),"{kind}: a proven released child must settle the original request");
            assert!(status.contains("\"state\":\"UNKNOWN\""));
            let fields = object(Parser::parse(&status).unwrap()).unwrap();
            let Json::String(output) = &fields[&JsonString::from_str("output")] else { panic!("retained output") };
            let output = output.to_well_formed_string().unwrap();
            assert!(output.contains(&cause),"{kind}: original error retained after the first Err");
            if kind == "prepared" { product.connection.execute("DROP TRIGGER fail_first_prepare").unwrap(); }
            if matches!(kind, "active" | "ordinary_active") {
                product.connection.execute("DROP TRIGGER fail_first_active").unwrap();
                assert!(matches!(fs::symlink_metadata(&generated_cache),
                    Err(ref error) if error.kind() == std::io::ErrorKind::NotFound),
                    "original first CLI failure reconciliation must unlink generated cache");
                let observation = product.dispatch_owner_login_observation(
                    &request("login-state", &format!("afterFirst_{kind}"), revision, "{}"))
                    .unwrap_or_else(|error| panic!("post-first-failure LPAC observation: {error:?}; original settlement: {}",
                        output.replace(original_home.to_string_lossy().as_ref(), "<instance-home>")
                            .replace(path.to_string_lossy().as_ref(), "<test-root>")));
                assert!(String::from_utf8(observation).unwrap().contains("\"state\":\"LOGGED_OUT\""),
                    "same original home must remain admissible to LPAC after first CLI failure");
                if kind == "active" {
                    assert_eq!(fs::read(path.join("owned-active-failure-cache-target").join("sentinel")).unwrap(),
                        b"retained controlled cache target");
                }
            }
        }
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn provider_active_record_failure_releases_only_original_registered_runtime() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-provider-stop-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let powershell = Path::new(&std::env::var("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let program = instance::ProgramObservation::observe(&powershell, "1.18.32").unwrap();
        let register = request("register", "registerProvider", 0,
            r#"{"driverId":"opencode"}"#);
        assert_eq!(instance::register_instance(&mut product.connection, &root,
            &instance::Registration { request_id: &register.request_id,
                request_bytes: &register.raw_bytes, instance_id: "instanceA",
                driver_id: "opencode", program: &program }).unwrap(),
            RegistrationDisposition::Applied);
        let home = instance::provider_login::resolve_registered_login_home(
            &mut product.connection, &root, &product.owner, "instanceA", "opencode").unwrap();
        let (runtime, runtime_identity) = runtime_home(&home.path).unwrap();
        let mut child = ProcessLaunch::new(powershell.clone());
        child.arguments = vec!["-NoProfile".into(), "-NonInteractive".into(),
            "-Command".into(), "exit 0".into()];
        child.current_directory = Some(home.path.clone());
        child.protocol_stdio = true;
        child.persistent_protocol_stdio = true;
        let login = PrepareRequest { launch: child,
            binding: NativeBinding { binary_digest_sha256:
                crate::store::digest::content_hash(&fs::read(&powershell).unwrap()),
                profile_id: "instanceA".into(), domain_id: "global".into(),
                generation: "1".into() } };
        let launch = PreparedOwnerLogin { login: login.clone(), account_read: login.clone(),
            credential_custody:None,
            runtime_home: runtime.clone(), runtime_identity: runtime_identity.clone(),
            registered_driver: "opencode".into(),
            registered_home_identity: home.identity.clone() };
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"providerActiveFault","expectedRevision":1}"#).unwrap();
        product.connection.execute("CREATE TRIGGER fail_provider_active BEFORE UPDATE OF state ON gogoke_coordination_process_custody WHEN NEW.state='ACTIVE' AND NEW.operation_id LIKE 'owner-login-%' BEGIN SELECT RAISE(ABORT,'controlled provider active record failure'); END").unwrap();
        let error = product.start_owner_device_login(&command, launch,
            |custodian, prepared| custodian.activate(prepared)).unwrap_err();
        product.connection.execute("DROP TRIGGER fail_provider_active").unwrap();
        assert!(format!("{error:?}").contains("controlled provider active record failure"));
        let sentinel = runtime.join("held-cleanup-control.bin");
        fs::write(&sentinel, b"owned cleanup control").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(0x1).open(&sentinel).unwrap();
        let result = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(result.contains("\"settled\":false"), "failed cleanup retains original request");
        assert!(result.contains("\"state\":\"UNKNOWN\""));
        assert!(result.contains("raw_os_error"), "private result preserves original Windows failure");
        assert!(runtime.exists(), "failed cleanup retains exact host-owned runtime");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1");
        let new_request = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"providerNewRequest","expectedRevision":1}"#;
        assert!(matches!(product.dispatch_owner_login_frame(new_request), Err(OrchestrationError::OperationConflict)),
            "new User intent cannot replace retained cleanup custody");
        drop(held);
        let final_reply = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(final_reply.contains("\"settled\":true"), "same request reconciles cleanup after exact handle release");
        assert!(final_reply.contains("raw_os_error"), "recovered result retains prior failure reason");
        assert!(!runtime.exists(), "confirmed provider runtime must be removed");
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "1");
        // The ordinary confirmed-stop path has the same cleanup obligation:
        // a Windows sharing error must retain the original released custody.
        let (runtime, runtime_identity) = runtime_home(&home.path).unwrap();
        let command = owner_login_command(br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"providerConfirmedCleanup","expectedRevision":1}"#).unwrap();
        let operation_id = owner_login_operation_id(&command);
        let prepared = product.process_custodian.prepare(&login).unwrap();
        authority::record_prepared_process(&mut product.connection, &operation_id, &prepared).unwrap();
        product.process_custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut product.connection, &operation_id, &prepared).unwrap();
        assert!(product.process_custodian.active(&prepared.ticket).unwrap()
            .wait(Duration::from_secs(15)).unwrap());
        let proof = product.process_custodian.stop(&prepared.ticket,
            StopBudgets::production(), || Ok(())).unwrap();
        assert_eq!(proof.exit_code, Some(0));
        let revision = authority::mark_process_stopped(&mut product.connection, &operation_id, &proof).unwrap();
        product.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: prepared.ticket.clone(), custodian_nonce: prepared.custodian_nonce.clone(),
            identity: prepared.identity.clone(), proof_hash: proof.proof_hash(), durable_revision: revision,
        }).unwrap();
        let sentinel = runtime.join("held-confirmed-cleanup-control.bin");
        fs::write(&sentinel, b"owned confirmed cleanup control").unwrap();
        let held = fs::OpenOptions::new().read(true).share_mode(0x1).open(&sentinel).unwrap();
        let active = ActiveOwnerLogin {
            instance_id: command.instance_id.clone(), request_id: command.request_id.clone(),
            expected_revision: command.expected_revision, operation_id, prepared,
            runtime_home: runtime.clone(), runtime_identity,
            output: "original CLI completed".into(), stderr_seen: 0, halted: false, rpc: None,
            provider: Some(instance::provider_login::PreparedProviderLogin {
                instance_id: "instanceA".into(), driver_id: "opencode".into(),
                version: "1.18.32".into(), program_digest: login.binding.binary_digest_sha256.clone(),
                application: powershell.clone(), home: home.clone(), login: login.clone(),
                status: instance::provider_login::StatusObservation::Unknown("test status unused"),
                browser: instance::provider_login::BrowserBehavior::HostOpensPrintedAuthorization,
            }),
            provider_completion_frame: true,
        };
        let error = product.finish_confirmed_owner_login(&command, active, false, None).unwrap_err();
        assert!(format!("{error:?}").contains("raw_os_error"));
        assert!(matches!(&product.owner_login,
            Some(OwnerLoginSession::PendingAccount(pending)) if pending.continuation.as_ref()
                .is_some_and(|continuation| continuation.completion_frame
                    && continuation.provider.as_ref().is_some_and(|provider|
                        provider.driver_id == "opencode"))),
            "original typed OpenCode completion must survive cleanup failure");
        let pending = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(pending.contains("\"settled\":false"));
        assert!(pending.contains("original CLI completed") && pending.contains("raw_os_error"));
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "2");
        let new_request = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"providerAfterConfirmedFault","expectedRevision":1}"#;
        assert!(matches!(product.dispatch_owner_login_frame(new_request), Err(OrchestrationError::OperationConflict)));
        drop(held);
        // This fixture's PowerShell image is not a registered OpenCode CLI.
        // Confirmed cleanup therefore exposes the real catalog failure once,
        // then preserves its final UNKNOWN result for ordinary UI readback.
        let status_error=product.status_owner_device_login(&command).unwrap_err();
        assert!(format!("{status_error:?}").contains("provider login pinned executable"));
        assert!(matches!(&product.owner_login,Some(OwnerLoginSession::Final {state,output,..})
            if state=="UNKNOWN" && output.contains("automatic account/read failed")));
        let final_reply = String::from_utf8(product.status_owner_device_login(&command).unwrap()).unwrap();
        assert!(final_reply.contains("\"settled\":true") && final_reply.contains("raw_os_error"));
        assert!(final_reply.contains("\"state\":\"UNKNOWN\"")
            && final_reply.contains("automatic account/read failed"),
            "synthetic PowerShell metadata must not pass the fixed OpenCode catalog recheck");
        assert!(!runtime.exists());
        assert_eq!(scalar(&product, "SELECT count(*) FROM gogoke_coordination_process_custody WHERE state='STOPPED'"), "2");
        let (next, next_identity) = runtime_home(&home.path).unwrap();
        remove_owned_runtime(&next, &next_identity).unwrap();
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
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity, .. } =
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
            prepared, runtime_home, runtime_identity, output: String::new(), stderr_seen: 0, halted: false, rpc: None, provider: None,
            provider_completion_frame: false,
        };
        let error = product.finish_owner_device_login(&command, active, false, None).unwrap_err();
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
            "the retained real account/read frame must produce the native state; actual Owner-private reply: {settled}");
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
        product.connection.execute("UPDATE main.gogoke_v37_instances SET version='0.160.0' WHERE instance_id='instanceA'").unwrap();
        let PreparedOwnerLogin { mut login, account_read, runtime_home, runtime_identity, .. } =
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
            expected_revision:2,operation_id,prepared,runtime_home,runtime_identity,output:String::new(),stderr_seen:0,halted:false,rpc:None,provider:None,provider_completion_frame:false };
        let error = product.finish_owner_device_login(&command, active, false, None).unwrap_err();
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
        let original=instance::read_configured_credential_backend(&product.connection,"instanceA").unwrap();
        assert_eq!(original.backend,instance::CredentialBackend::File);
        assert_eq!(original.startup_selector,instance::CredentialStartupSelector::Unknown,
            "the prior CLI was not started with an explicit File selector");
        assert!(instance::read_usable_credential_backend(&product.connection,"instanceA").is_err());
        let bound=product.dispatch_owner_file_backend_observation(
            &request("login-state","observeSyntheticFileStartup",2,"{}")).unwrap();
        assert_eq!(owner_login_state_from_receipt(&bound).unwrap(),"LOGGED_IN");
        let qualified=instance::read_usable_credential_backend(&product.connection,"instanceA").unwrap();
        assert_eq!(qualified.startup_selector,instance::CredentialStartupSelector::FileBound);
        assert_ne!(original.operation_id,qualified.operation_id,
            "File-bound evidence must come from the subsequent actual CLI, not backstamp the old process");
        assert_ne!(original.ticket,qualified.ticket);
        product.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_ordinary_login_rejects_a_registration_outside_the_authorized_cli_digest() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-login-digest-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite")).unwrap();
        let registered = product.register_user_instance(
            &request("register", "registerA", 0, r#"{"driverId":"codex"}"#)).unwrap();
        assert_eq!(decode_receipt(&registered).unwrap().status, V37Status::Applied);
        let home = instance::resolve_codex_instance_home(&product.connection,
            &root, "instanceA").unwrap();
        // Only the test's recorded admission metadata changes. The fixed
        // vendor program remains untouched and no alternate program runs.
        let update = Statement::prepare(product.connection.as_ptr(),
            "UPDATE main.gogoke_v37_instances SET program_digest=?1 WHERE instance_id=?2").unwrap();
        update.bind_text(1, &format!("sha256:{}", "0".repeat(64))).unwrap();
        update.bind_text(2, "instanceA").unwrap();
        update.step_done().unwrap();
        drop(update);
        assert!(matches!(product.prepare_owner_codex_login("instanceA"),
            Err(OrchestrationError::AccessDenied)));
        assert!(!home.path.join("gogoke-login-runtime").exists());
        assert!(product.owner_login.is_none());
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
            "CI must install the exact 0.160.0 native catalog, not skip the test");
        let home = instance::resolve_codex_instance_home(&product.connection,
            &root, "instanceA").unwrap();
        let runtime = home.path.join("gogoke-login-runtime");
        let scoped = product.prepare_owner_codex_login("instanceA").unwrap();
        assert!(scoped.login.launch.app_container_profile.is_none());
        assert!(scoped.login.launch.path_compat.is_none());
        assert_eq!(scoped.login.launch.arguments, scoped.account_read.launch.arguments);
        assert!(!scoped.login.launch.app_container_internet_client);
        assert!(!scoped.login.launch.app_container_cli_identity_services);
        assert!(scoped.account_read.launch.app_container_profile.is_some());
        assert!(scoped.account_read.launch.app_container_internet_client);
        assert!(scoped.account_read.launch.app_container_cli_identity_services);
        assert!(scoped.account_read.launch.path_compat.is_some());
        assert_eq!(scoped.login.launch.application,
            scoped.account_read.launch.application);
        let login_environment = scoped.login.launch.environment.as_ref().unwrap();
        let account_environment = scoped.account_read.launch.environment.as_ref().unwrap();
        let instance = home.path.to_string_lossy();
        let runtime_text = runtime.to_string_lossy();
        for key in ["HOME", "USERPROFILE", "CODEX_HOME"] {
            assert!(environment_value(login_environment, key) == Some(instance.as_ref()), "login {key} mismatch");
        }
        for key in ["HOME", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "TEMP", "TMP"] {
            assert!(environment_value(account_environment, key) == Some(runtime_text.as_ref()), "account/read {key} mismatch");
        }
        for key in ["LOCALAPPDATA", "APPDATA", "TEMP", "TMP"] {
            let actual = std::env::var(key).unwrap();
            assert!(environment_value(login_environment, key) == Some(actual.as_str()), "login {key} is not host value");
        }
        let mut login_keys: Vec<&str> = login_environment.iter().map(|(key, _)| key.as_str()).collect();
        login_keys.sort_unstable();
        assert_eq!(login_keys, ["APPDATA", "CODEX_HOME", "HOME", "LOCALAPPDATA", "RUST_LOG",
            "SystemRoot", "TEMP", "TMP", "USERPROFILE", "WINDIR"]);
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

    #[test]
    fn pinned_cli_ordinary_oauth_callback_reaches_exact_owned_child() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        let _guard = route_b_test_guard();
        // Ordinary login clears prior auth. This test therefore registers a
        // fresh home and never sends a code, state, or real authorization.
        for port in [1455, 1457] {
            let probe = TcpListener::bind(("127.0.0.1", port))
                .expect("OAuth callback port already occupied; do not interrupt another login");
            drop(probe);
        }
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-ordinary-oauth-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap_or_else(|_| panic!("isolated OAuth test root creation failed"));
        let root = RootLock::acquire(&path).unwrap_or_else(|_| panic!("isolated OAuth test root lock failed"));
        let mut product = ProductDatabase::open(&root, &path.join("state.sqlite"))
            .unwrap_or_else(|_| panic!("isolated OAuth product database open failed"));
        let registered = decode_receipt(&product.register_user_instance(
            &request("register", "registerA", 0, r#"{"driverId":"codex"}"#))
            .unwrap_or_else(|_| panic!("isolated OAuth instance registration failed")))
            .unwrap_or_else(|_| panic!("isolated OAuth registration receipt invalid"));
        assert_eq!(registered.status, V37Status::Applied);
        let home = instance::resolve_codex_instance_home(&product.connection, &root, "instanceA")
            .unwrap_or_else(|_| panic!("isolated OAuth registered home resolution failed"));
        assert!(matches!(fs::symlink_metadata(home.path.join("auth.json")),
            Err(ref error) if error.kind() == std::io::ErrorKind::NotFound),
            "ordinary OAuth test must begin with an empty registered auth home");
        let begin = br#"{"schema":"gogoke.37.owner-login.v1","action":"begin","instanceId":"instanceA","requestId":"ordinaryOAuthA","expectedRevision":1}"#;
        let status = br#"{"schema":"gogoke.37.owner-login.v1","action":"status","instanceId":"instanceA","requestId":"ordinaryOAuthA","expectedRevision":1}"#;
        let cancel = br#"{"schema":"gogoke.37.owner-login.v1","action":"cancel","instanceId":"instanceA","requestId":"ordinaryOAuthA","expectedRevision":1}"#;
        // Use the actual production preparation and custody path once.
        let first_scope = product.prepare_owner_codex_login("instanceA").unwrap();
        let begin_command = owner_login_command(begin).unwrap();
        let safe_error = |error: &OrchestrationError| {
            let raw = format!("{error:?}");
            // Retain the actual error category/stage and OS code. Only paths
            // and the complete private authorization URL line are redacted.
            raw.split("\\n").filter(|line| !line.contains("https://auth.openai.com/oauth/authorize?"))
                .collect::<Vec<_>>().join("\\n")
                .replace(home.path.to_string_lossy().as_ref(), "<instance-home>")
                .replace(path.to_string_lossy().as_ref(), "<test-root>")
        };
        let stages = (|| -> std::result::Result<std::result::Result<(), String>, String> {
            let started = match product.start_owner_device_login(&begin_command, first_scope,
                |custodian, prepared| custodian.activate(prepared)) {
                Ok(reply) => reply,
                Err(error) => {
                    let status_result = product.dispatch_owner_login_frame(status);
                    let cancel_result = product.dispatch_owner_login_frame(cancel);
                    return Err(format!("ordinary-user fixed CLI start: {}; status_error={:?}; cancel_error={:?}",
                        safe_error(&error), status_result.err().as_ref().map(&safe_error),
                        cancel_result.err().as_ref().map(&safe_error)));
                }
            };
            assert!(String::from_utf8(started).map(|text| text.contains("\"state\":\"PENDING\"")).unwrap_or(false));
            let prepared = match product.owner_login.as_ref().unwrap() {
                OwnerLoginSession::Active(active) => active.prepared.clone(),
                _ => panic!("ordinary login did not retain active child custody"),
            };
            let probe_callback = |product: &mut ProductDatabase<'_>, status: &[u8],
                prepared: &PreparedCustody| -> std::result::Result<(), String> {
                let deadline = Instant::now() + Duration::from_secs(30);
                let mut callback_port = None;
                let mut cli_error_code = None;
                while Instant::now() < deadline {
                    let reply = match product.dispatch_owner_login_frame(status) {
                        Ok(reply) => reply,
                        Err(error) => {
                            cli_error_code = cli_os_error_code(&format!("{error:?}"));
                            product.dispatch_owner_login_frame(status)
                                .map_err(|_| format!("owned login status failed after CLI error; {}",
                                    cli_error_code.clone().unwrap_or_else(|| "no safe CLI OS code observed".into())))?
                        }
                    };
                    let text = String::from_utf8(reply)
                        .map_err(|_| "owner-private status encoding failed".to_owned())?;
                    // Keep full private output only in the session. A public CI
                    // failure may report the raw OS code, never URL/query/home.
                    let mut fields = object(Parser::parse(&text)
                        .map_err(|_| "status parse failed".to_owned())?)
                        .ok_or_else(|| "status shape failed".to_owned())?;
                    let Some(Json::String(value)) = fields.remove(&JsonString::from_str("output")) else {
                        return Err("owner-private login output missing".into());
                    };
                    let output = value.to_well_formed_string()
                        .ok_or_else(|| "owner-private output malformed".to_owned())?;
                    if cli_error_code.is_none() { cli_error_code = cli_os_error_code(&output); }
                    let settled = matches!(fields.remove(&JsonString::from_str("settled")), Some(Json::Bool(true)));
                    if settled {
                        return Err(format!("fixed CLI settled before publishing OAuth URL; {}",
                            cli_error_code.unwrap_or_else(|| "no safe CLI OS code observed".into())));
                    }
                    let complete_line = output.lines().find(|line|
                        line.starts_with("https://auth.openai.com/oauth/authorize?")
                        && output.contains(&format!("{line}\n")));
                    if let Some(line) = complete_line {
                        callback_port = callback_port_from_authorization_line(line);
                        if callback_port.is_none() { return Err("CLI URL has no permitted fixed loopback callback".into()); }
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let callback_port = callback_port
                    .ok_or_else(|| format!("fixed CLI did not publish a complete OAuth URL while PENDING; {}",
                        cli_error_code.unwrap_or_else(|| "no safe CLI OS code observed".into())))?;
                let child = product.process_custodian.active(&prepared.ticket)
                    .ok_or_else(|| "owned login child missing".to_owned())?;
                if child.identity().pid != prepared.identity.pid
                    || child.identity().creation_time_100ns != prepared.identity.creation_time_100ns {
                    return Err("retained child identity changed".into());
                }
                let powershell = Path::new(&std::env::var("SystemRoot")
                    .map_err(|_| "SystemRoot missing".to_owned())?)
                    .join("System32/WindowsPowerShell/v1.0/powershell.exe");
                let owner_query = format!("(Get-NetTCPConnection -LocalAddress '127.0.0.1' -LocalPort {callback_port} -State Listen -ErrorAction Stop).OwningProcess");
                let owner = std::process::Command::new(powershell)
                    .args(["-NoProfile", "-NonInteractive", "-Command", &owner_query])
                    .output().map_err(|error| format!("Windows listener owner query failed: {error}; raw_os_error={:?}", error.raw_os_error()))?;
                if !owner.status.success() { return Err("Windows listener owner query failed".into()); }
                let listener_pid: u32 = String::from_utf8(owner.stdout)
                    .map_err(|_| "listener PID encoding failed".to_owned())?.trim().parse()
                    .map_err(|_| "listener PID parse failed".to_owned())?;
                if listener_pid != child.identity().pid { return Err("listener does not belong to retained child".into()); }
                let address = format!("127.0.0.1:{callback_port}");
                let mut socket = TcpStream::connect_timeout(
                    &address.parse().unwrap(), Duration::from_secs(5))
                    .map_err(|error| format!("callback socket connect failed: {error}; raw_os_error={:?}", error.raw_os_error()))?;
                socket.set_read_timeout(Some(Duration::from_secs(5)))
                    .map_err(|error| format!("callback socket timeout setup failed: {error}; raw_os_error={:?}", error.raw_os_error()))?;
                let callback_request = format!("GET /auth/callback HTTP/1.1\r\nHost: 127.0.0.1:{callback_port}\r\nConnection: close\r\n\r\n");
                socket.write_all(callback_request.as_bytes())
                    .map_err(|error| format!("callback socket write failed: {error}; raw_os_error={:?}", error.raw_os_error()))?;
                let mut response = String::new();
                socket.read_to_string(&mut response)
                    .map_err(|error| format!("callback socket read failed: {error}; raw_os_error={:?}", error.raw_os_error()))?;
                let status_line = response.lines().next().unwrap_or("<missing>");
                let http_code = status_line.split_whitespace().nth(1)
                    .filter(|code| code.len() == 3 && code.bytes().all(|byte| byte.is_ascii_digit()))
                    .unwrap_or("<invalid>");
                if http_code != "400" || !response.contains("State mismatch") {
                    return Err(format!("real fixed CLI callback did not reject missing state; HTTP code: {http_code}"));
                }
                Ok(())
            };
            let production_probe = probe_callback(&mut product, status, &prepared);
            let cancelled = product.dispatch_owner_login_frame(cancel);
            let settled = match cancelled {
                Ok(reply) => String::from_utf8(reply).map(|text| text.contains("\"settled\":true")).unwrap_or(false),
                Err(error) => return Err(format!("ordinary OAuth cancellation failed: {}; callback={production_probe:?}", safe_error(&error))),
            };
            if !settled { return Err(format!("original ordinary OAuth cancellation did not settle; callback={production_probe:?}")); }
            assert!(matches!(fs::symlink_metadata(home.path.join("auth.json")),
                Err(ref error) if error.kind() == std::io::ErrorKind::NotFound),
                "no-code/state callback must not write an auth file");
            let revision = product.user_instance_revision("instanceA")
                .map_err(|error| safe_error(&error))?;
            let readback = product.dispatch_owner_login_observation(
                &request("login-state", "ordinaryOAuthAfterCancel", revision, "{}"))
                .map_err(|error| format!("post-cancel LPAC account/read: {}; callback={production_probe:?}",
                    safe_error(&error)))?;
            if !String::from_utf8(readback).map_err(|error| error.to_string())?
                .contains("\"state\":\"LOGGED_OUT\"") {
                return Err("empty original home must remain readable by LPAC account/read".into());
            }
            Ok(production_probe)
        })();
        // Keep the measured callback results even if subsequent teardown fails.
        // This contains only stage/code summaries, never the authorization URL.
        eprintln!("actual fixed CLI callback stages before teardown: {stages:?}");
        product.close_checked().unwrap_or_else(|error|
            panic!("ordinary OAuth custody close failed: {}; stages={stages:?}",
                format!("{error:?}").replace(home.path.to_string_lossy().as_ref(), "<instance-home>")
                    .replace(path.to_string_lossy().as_ref(), "<test-root>")));
        drop(root);
        // Reuse temporary-home cleanup's per-entry/reparse-aware primitives.
        // Report the exact denied relative object rather than masking the two
        // callback results behind remove_dir_all's pathless error. No retry or
        // permission change; the fresh cloud test root is the only target.
        // Read only the failing entry itself. Open-reparse-point handles never
        // read its target; these probes neither delete nor change permissions.
        fn failed_entry_details(base: &Path, entry: &Path) -> String {
            use std::ffi::c_void;
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            extern "system" {
                fn CreateFileW(path: *const u16, access: u32, share: u32,
                    security: *mut c_void, disposition: u32, flags: u32,
                    template: *mut c_void) -> *mut c_void;
                fn GetFileInformationByHandleEx(handle: *mut c_void, class: u32,
                    data: *mut c_void, length: u32) -> i32;
                fn CloseHandle(handle: *mut c_void) -> i32;
                fn LocalFree(memory: *mut c_void) -> *mut c_void;
            }
            #[link(name = "advapi32")]
            extern "system" {
                fn GetSecurityInfo(handle: *mut c_void, kind: u32, info: u32,
                    owner: *mut *mut c_void, group: *mut *mut c_void,
                    dacl: *mut *mut c_void, sacl: *mut *mut c_void,
                    descriptor: *mut *mut c_void) -> u32;
                fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor: *mut c_void, revision: u32, info: u32,
                    text: *mut *mut u16, length: *mut u32) -> i32;
            }
            fn open(path: &Path, access: u32) -> std::result::Result<*mut c_void, std::io::Error> {
                let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
                let handle = unsafe { CreateFileW(wide.as_ptr(), access, 7,
                    std::ptr::null_mut(), 3, 0x02200000, std::ptr::null_mut()) };
                if handle.is_null() || handle as isize == -1 { Err(std::io::Error::last_os_error()) }
                else { Ok(handle) }
            }
            fn access(path: &Path, desired: u32) -> String {
                match open(path, desired) {
                    Ok(handle) => format!("OPEN_OK_CLOSE={}", unsafe { CloseHandle(handle) }),
                    Err(error) => format!("{error}; raw_os_error={:?}", error.raw_os_error()),
                }
            }
            let mut details = vec![format!("entry_DELETE={}", access(entry, 0x10000))];
            if let Some(parent) = entry.parent() {
                details.push(format!("parent_DELETE_CHILD={}", access(parent, 0x40)));
            }
            match open(entry, 0x80) {
                Ok(handle) => {
                    let mut tag = [0u32; 2];
                    let mut id = [0u64; 3];
                    let tag_ok = unsafe { GetFileInformationByHandleEx(handle, 9,
                        tag.as_mut_ptr().cast(), 8) };
                    let tag_error = (tag_ok == 0).then(std::io::Error::last_os_error);
                    let id_ok = unsafe { GetFileInformationByHandleEx(handle, 18,
                        id.as_mut_ptr().cast(), 24) };
                    let id_error = (id_ok == 0).then(std::io::Error::last_os_error);
                    details.push(format!("tag={tag:x?}; tag_result={tag_ok}; tag_error={tag_error:?}; no_follow_id={id:x?}; id_result={id_ok}; id_error={id_error:?}; close={}",
                        unsafe { CloseHandle(handle) }));
                }
                Err(error) => details.push(format!("entry_attributes_open={error}")),
            }
            match open(entry, 0x20000) {
                Ok(handle) => {
                    let mut descriptor = std::ptr::null_mut();
                    let status = unsafe { GetSecurityInfo(handle, 1, 4,
                        std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(),
                        std::ptr::null_mut(), &mut descriptor) };
                    if status == 0 && !descriptor.is_null() {
                        let mut text = std::ptr::null_mut();
                        let mut length = 0;
                        let converted = unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(
                            descriptor, 1, 4, &mut text, &mut length) };
                        if converted != 0 && !text.is_null() && length <= 65536 {
                            let sddl = String::from_utf16_lossy(unsafe {
                                std::slice::from_raw_parts(text, length as usize) });
                            details.push(format!("link_dacl={}", sddl.trim_end_matches('\0')));
                        } else { details.push(format!("link_dacl_conversion={converted}; error={:?}", std::io::Error::last_os_error())); }
                        if !text.is_null() { unsafe { LocalFree(text.cast()); } }
                        unsafe { LocalFree(descriptor); }
                    } else { details.push(format!("link_dacl_status={status}")); }
                    details.push(format!("link_dacl_handle_close={}", unsafe { CloseHandle(handle) }));
                }
                Err(error) => details.push(format!("link_READ_CONTROL={error}")),
            }
            details.push(match fs::read_link(entry) {
                Ok(target) => format!("target_text_has_test_root_prefix={}; target_text_sha256={}",
                    target.starts_with(base), crate::store::digest::sha256_hex(
                        target.to_string_lossy().as_bytes())),
                Err(error) => format!("read_link={error}; raw_os_error={:?}", error.raw_os_error()),
            });
            details.join("; ")
        }
        fn remove_test_entry(base: &Path, entry: &Path) -> std::result::Result<(), String> {
            use std::os::windows::fs::MetadataExt;
            let relative = entry.strip_prefix(base)
                .map_err(|_| "test cleanup target escaped its root".to_owned())?;
            let metadata = fs::symlink_metadata(entry).map_err(|error|
                format!("test metadata {relative:?}: {error}; raw_os_error={:?}", error.raw_os_error()))?;
            let attributes = metadata.file_attributes();
            if metadata.is_dir() && attributes & 0x400 == 0 {
                for child in fs::read_dir(entry).map_err(|error|
                    format!("test directory read {relative:?}: {error}; raw_os_error={:?}", error.raw_os_error()))? {
                    let child = child.map_err(|error|
                        format!("test directory entry {relative:?}: {error}; raw_os_error={:?}", error.raw_os_error()))?;
                    remove_test_entry(base, &child.path())?;
                }
            }
            let deleted = if attributes & 0x10 != 0 { fs::remove_dir(entry) } else { fs::remove_file(entry) };
            deleted.map_err(|error| format!("test deletion {relative:?}: {error}; raw_os_error={:?}; attributes={attributes}; {}",
                error.raw_os_error(), failed_entry_details(base, entry)))
        }
        remove_test_entry(&path, &path).unwrap_or_else(|error|
            panic!("ordinary OAuth test cleanup failed: {error}; stages={stages:?}"));
        match stages {
            Ok(probe) => assert!(probe.is_ok(),
                "production ordinary-user fixed CLI callback stages (custody and root cleanup complete): {probe:?}"),
            Err(error) => panic!("actual fixed CLI callback stages (custody and root cleanup complete): {error}"),
        }
    }
}
