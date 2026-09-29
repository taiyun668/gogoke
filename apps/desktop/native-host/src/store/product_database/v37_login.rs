//! Private F.1 Codex login preparation and native account observation.
//!
//! The Owner enters credentials in the official CLI. This module never reads
//! `auth.json`, a keyring, or any account field, and never starts a model turn.
//! The caller must establish User origin before invoking this private action.

use super::*;
use crate::process::{DurableStopConfirmation, NativeBinding, OriginBoundFrame,
    PrepareRequest, PreparedCustody, ProcessCustodyError, ProcessLaunch, StopBudgets};
use crate::root::{inspect_root, RootIdentity};
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
    Final {
        instance_id: String,
        request_id: String,
        expected_revision: u64,
        state: String,
        output: String,
    },
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

/// A bounded routing hint only. `owner_login_command` still validates every
/// field after the trusted User pipe has established its process origin.
pub(super) fn is_owner_login_frame(frame: &[u8]) -> bool {
    frame.len() <= 4096 && frame.windows(b"gogoke.37.owner-login.v1".len())
        .any(|window| window == b"gogoke.37.owner-login.v1")
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
    Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"), Json::String(JsonString::from_str("gogoke.37.owner-login.v1"))),
        (JsonString::from_str("instanceId"), Json::String(JsonString::from_str(&command.instance_id))),
        (JsonString::from_str("requestId"), Json::String(JsonString::from_str(&command.request_id))),
        (JsonString::from_str("state"), Json::String(JsonString::from_str(state))),
        // Only this Owner-private response carries device-auth stdout. The
        // value is never stored in the instance or coordination journal.
        (JsonString::from_str("output"), Json::String(JsonString::from_str(output))),
    ])).canonical().into_bytes()
}

fn same_owner_login(session: &OwnerLoginSession, command: &OwnerLoginCommand) -> bool {
    let (instance, request, revision) = match session {
        OwnerLoginSession::Active(active) => (&active.instance_id, &active.request_id,
            active.expected_revision),
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
    ])
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
            if matches!(self.owner_login, Some(OwnerLoginSession::Active(_))) {
                return Err(OrchestrationError::OperationConflict);
            }
            let state = self.owner_login_account_state(&command)?;
            return Ok(owner_login_reply(&command, &state, ""));
        }
        if let Some(session) = &self.owner_login {
            if !same_owner_login(session, &command) {
                if matches!(session, OwnerLoginSession::Active(_)) {
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

    fn begin_owner_device_login(&mut self, command: &OwnerLoginCommand) -> Result<Vec<u8>> {
        if let Some(session) = &self.owner_login {
            return Ok(match session {
                OwnerLoginSession::Active(active) => owner_login_reply(command,
                    if active.halted { "UNKNOWN" } else { "PENDING" }, &active.output),
                OwnerLoginSession::Final { state, output, .. } =>
                    owner_login_reply(command, state, output),
            });
        }
        let current = self.user_instance_revision(&command.instance_id)?;
        if current != command.expected_revision {
            return Err(OrchestrationError::OperationConflict);
        }
        let launch = self.prepare_owner_codex_login(&command.instance_id)?;
        let operation_id = owner_login_operation_id(command);
        let prepared = match self.process_custodian.prepare(&launch.login) {
            Ok(prepared) => prepared,
            Err(error) => {
                let cleaned = remove_owned_runtime(&launch.runtime_home,
                    &launch.runtime_identity);
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "owner login process prepare: {error:?}; cleanup: {cleaned:?}")));
            }
        };
        if let Err(error) = authority::record_prepared_process(
            &mut self.connection, &operation_id, &prepared,
        ) {
            let aborted = self.process_custodian.abort_prepared(&prepared);
            let cleaned = remove_owned_runtime(&launch.runtime_home,
                &launch.runtime_identity);
            return Err(OrchestrationError::V37StoreFailure(format!(
                "owner login prepare record: {error:?}; abort: {aborted:?}; cleanup: {cleaned:?}")));
        }
        if let Err(error) = self.process_custodian.activate(&prepared) {
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            return Err(OrchestrationError::V37StoreFailure(format!(
                "owner login activate: {error:?}; unknown record: {unknown:?}")));
        }
        if let Err(error) = authority::mark_process_active(
            &mut self.connection, &operation_id, &prepared,
        ) {
            let stop = self.process_custodian.stop(&prepared.ticket,
                StopBudgets::production(), || Ok(()));
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            return Err(OrchestrationError::V37StoreFailure(format!(
                "owner login active record: {error:?}; stop: {stop:?}; unknown record: {unknown:?}")));
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
                let reply = owner_login_reply(command, &state, &output);
                self.owner_login = Some(OwnerLoginSession::Final {
                    instance_id, request_id, expected_revision, state, output,
                });
                return Ok(reply);
            }
            OwnerLoginSession::Active(active) => active,
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
                let reply = owner_login_reply(command, &state, &output);
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
        }
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
        let state_result = if cleanup.is_ok() {
            self.owner_login_account_state(command)
        } else {
            Ok("UNKNOWN".to_owned())
        };
        let state = state_result.as_ref().cloned().unwrap_or_else(|_| "UNKNOWN".to_owned());
        let reply = owner_login_reply(command, &state, &active.output);
        self.owner_login = Some(OwnerLoginSession::Final {
            instance_id: active.instance_id,
            request_id: active.request_id,
            expected_revision: active.expected_revision,
            state,
            output: active.output,
        });
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
        let receipt = self.dispatch_owner_login_observation(&observation)?;
        let text = std::str::from_utf8(&receipt).map_err(|error|
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
        let (runtime, runtime_identity) = runtime_home(&home.path)?;
        let environment = clean_environment(&home.path, &runtime)?;
        let digest = row.program_digest.strip_prefix("sha256:")
            .filter(|digest| digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or(OrchestrationError::AccessDenied)?;
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
        let prepared_login = self.prepare_owner_codex_login(&request.target_id)?;
        let operation_hash = crate::store::digest::sha256_hex(&request.raw_bytes);
        let operation_id = format!("login-observe-{}", &operation_hash[..40]);
        let prepared = match self.process_custodian.prepare(&prepared_login.account_read) {
            Ok(prepared) => prepared,
            Err(error) => {
                let cleaned = remove_owned_runtime(&prepared_login.runtime_home,
                    &prepared_login.runtime_identity);
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "login observation process prepare: {error:?}; cleanup: {cleaned:?}")));
            }
        };
        if let Err(error) = authority::record_prepared_process(
            &mut self.connection, &operation_id, &prepared,
        ) {
            // The child is still suspended. Abort it if the durable PREPARED
            // row could not be established; preserve both original errors.
            if let Err(abort) = self.process_custodian.abort_prepared(&prepared) {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "login prepare record: {error:?}; abort: {abort:?}")));
            }
            remove_owned_runtime(&prepared_login.runtime_home,
                &prepared_login.runtime_identity)?;
            return Err(error);
        }
        if let Err(error) = self.process_custodian.activate(&prepared) {
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            return match unknown {
                Ok(()) => Err(error.into()),
                Err(record_error) => Err(OrchestrationError::V37StoreFailure(format!(
                    "login activate: {error:?}; unknown record: {record_error:?}"))),
            };
        }
        if let Err(error) = authority::mark_process_active(
            &mut self.connection, &operation_id, &prepared,
        ) {
            let stop = self.process_custodian.stop(&prepared.ticket,
                StopBudgets::production(), || Ok(()));
            let unknown = authority::mark_process_unknown(&mut self.connection,
                &operation_id, &prepared);
            return Err(OrchestrationError::V37StoreFailure(format!(
                "login active record: {error:?}; stop: {stop:?}; unknown record: {unknown:?}")));
        }
        let execution = self.observe_account_via_active_cli(&prepared);
        let close = self.process_custodian.close_child_input(&prepared.ticket)
            .map_err(|error| format!("account/read stdin close: {error}"));
        let stop = self.process_custodian.stop(&prepared.ticket,
            StopBudgets::production(), move || close);
        let proof = match stop {
            Ok(proof) => proof,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &operation_id, &prepared);
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "login stop: {error:?}; protocol: {:?}; unknown record: {unknown:?}",
                    execution.as_ref().err())));
            }
        };
        let revision = match authority::mark_process_stopped(
            &mut self.connection, &operation_id, &proof,
        ) {
            Ok(revision) => revision,
            Err(error) => {
                let unknown = authority::mark_process_unknown(&mut self.connection,
                    &operation_id, &prepared);
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "login stop record: {error:?}; proof: {proof:?}; protocol: {:?}; unknown record: {unknown:?}",
                    execution.as_ref().err())));
            }
        };
        self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: prepared.ticket.clone(),
            custodian_nonce: prepared.custodian_nonce.clone(),
            identity: prepared.identity.clone(),
            proof_hash: proof.proof_hash(),
            durable_revision: revision,
        })?;
        remove_owned_runtime(&prepared_login.runtime_home,
            &prepared_login.runtime_identity)?;
        let frame = execution?;
        self.record_trusted_account_read(request, &prepared, &frame)
    }

    fn observe_account_via_active_cli(&self, prepared: &PreparedCustody)
        -> Result<OriginBoundFrame> {
        let process = self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::Invalid("login process absent"))?;
        process.write_persistent_frame(INITIALIZE)
            .map_err(|error| OrchestrationError::Process(
                self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                    ProcessCustodyError::ProtocolPipe(error))))?;
        self.read_rpc_response(prepared, "1")?;
        let process = self.process_custodian.active(&prepared.ticket)
            .ok_or(OrchestrationError::Invalid("login process absent"))?;
        process.write_persistent_frame(INITIALIZED)
            .map_err(|error| OrchestrationError::Process(
                self.process_custodian.protocol_error_with_stderr(&prepared.ticket,
                    ProcessCustodyError::ProtocolPipe(error))))?;
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
                            std::io::ErrorKind::Other, "account RPC native error"))))),
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
