//! Native composition owns the existing database and its private bootstrap issuer.
//! IPC remains unprivileged: the legacy typed dispatcher never receives OwnerIssuer.
use super::atomic::{DomainRecordReceipt, Json, JsonString, Statement};
use super::authority::{
    self, AppendExecutionRecipe, AppendTaskMaterial, AuthorizedContextReadSet,
    AuthorizedTaskPackageReceipt,
    CommitTaskContextRequirements, ContextAssemblyBasis, ContextAssemblySnapshot,
    ContextAssemblySource, ContextManifestAuthorityReceipt, ContextManifestCommitInput,
    ContextManifestReplayIdentity, ContextReadRequest, ContextReadSet, ContextReadSnapshot,
    DecisionAuthoritySnapshot, DecisionCommitInput, DecisionCommitReceipt, DelegationGrantIdentity,
    DelegationGrantInput, DelegationGrantSnapshot, DurableDecisionReplay, GrantRef, GrantSpec,
    GranteeContextReadRequest, OwnerIssuer, OwnerOutcomeAppend, PrepareAuthorizedTaskPackage,
    AppendDreamProposal, AppendDreamRun, AppendEvaluation, AppendObjectiveOutcome, DreamReceipt,
    EvaluationReceipt, ExecutionRecipeReceipt,
    ExecutionRecipeVersion, ObjectiveOutcomeVersion,
    PromotionRequest, SessionLineageCommand, SessionLineageReceipt, SessionSnapshot,
    TaskContextRequirements, TaskContextRequirementsReceipt, TaskMaterialReceipt,
    TaskMaterialVersion,
};
use super::context::{ContextCommand, ContextReceipt};
use super::orchestration::OrchestrationError;
use super::same_open::{OpenLedger, SameOpenError, VerifiedDatabaseConnection};
use super::session::{dispatch_service_frame, open_product_database, serve_authenticated_pipe,
    serve_lines, serve_pipe, ServiceFrameSession};
use super::session_transport::{decode_request, encode_receipt, V37Request, V37Status};
use super::instance::{self, CatalogError, Registration, RegistrationDisposition,
    RegistrationReplay, RegistryError};
use crate::ipc::{PrivatePipeConnection, UserOriginProof};
use crate::root::RootLock;
use crate::process::ProcessCustodian;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, OrchestrationError>;

mod v37_seat;
mod v37_managed_cli;
mod v37_policy;
mod v37_session;
mod v37_runtime;
mod v37_output;
mod v37_health;
mod v37_host_rule;
mod v37_host_idle;
mod v37_host_recipient;
mod v37_runtime_status;
mod v37_model_tools;
mod v37_qcard;
mod v37_qcard_user;
mod v37_ledger_user;
mod v37_secretary_routine_user;
mod v37_inbox;
mod v37_capability;
mod v37_models;
mod v37_login;
mod v37_holder_disappearance;
mod v37_grok_home_recovery;
#[cfg(all(test, windows))]
mod v37_holder_disappearance_tests;
#[cfg(all(test, windows))]
mod managed_cli_test_setup;
mod v37_side;

/// Constructed only after the dedicated User pipe's live process proof.
/// H compares this borrowed exact frame with its original stdin request.
pub(crate) struct VerifiedDirectUserInput<'a> {
    origin: &'a UserOriginProof,
    frame: &'a [u8],
    observed_at_ms: Option<i64>,
}

impl VerifiedDirectUserInput<'_> {
    pub(crate) fn matches_live_frame(&self, frame: &[u8]) -> std::result::Result<bool, crate::ipc::PrivateIpcError> {
        self.origin.verify_live_origin()?;
        Ok(self.frame == frame)
    }

    pub(crate) fn observed_at_ms(&self) -> Option<i64> { self.observed_at_ms }
}

fn user_payload_string(request: &V37Request, field: &'static str) -> Result<String> {
    match request.payload.get(&JsonString::from_str(field)) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or(OrchestrationError::Invalid(field)),
        _ => Err(OrchestrationError::Invalid(field)),
    }
}

struct RegisteredInstance {
    driver_id: String,
    program_digest: String,
    version: String,
    login_state: String,
    revision: u64,
}

struct RegistrationSource {
    request_id: String,
    request_bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
enum InstallFact {
    Installed,
    Missing,
    Unknown,
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_framed_hex(value: &str) -> Result<Vec<Vec<u8>>> {
    if value.is_empty() || value.len() > 262_144 || value.len() % 2 != 0 {
        return Err(OrchestrationError::V37StoreFailure(
            "instance journal framing is invalid".into(),
        ));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0]).ok_or_else(||
            OrchestrationError::V37StoreFailure("instance journal hex is invalid".into()))?;
        let low = hex_nibble(pair[1]).ok_or_else(||
            OrchestrationError::V37StoreFailure("instance journal hex is invalid".into()))?;
        bytes.push((high << 4) | low);
    }
    let mut fields = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        if bytes.len() - offset < 8 {
            return Err(OrchestrationError::V37StoreFailure(
                "instance journal field length is missing".into(),
            ));
        }
        let length = u64::from_be_bytes(bytes[offset..offset + 8].try_into()
            .map_err(|_| OrchestrationError::V37StoreFailure(
                "instance journal field length is invalid".into(),
            ))?);
        offset += 8;
        let length = usize::try_from(length).map_err(|_|
            OrchestrationError::V37StoreFailure("instance journal field is too large".into()))?;
        if length > 65_536 || length > bytes.len() - offset {
            return Err(OrchestrationError::V37StoreFailure(
                "instance journal field is out of bounds".into(),
            ));
        }
        fields.push(bytes[offset..offset + length].to_vec());
        offset += length;
    }
    if fields.is_empty() {
        return Err(OrchestrationError::V37StoreFailure(
            "instance journal has no fields".into(),
        ));
    }
    Ok(fields)
}

fn catalog_error_is_missing(error: &CatalogError) -> bool {
    match error {
        CatalogError::Io(error) => error.kind() == std::io::ErrorKind::NotFound,
        CatalogError::Program(RegistryError::Io(error)) =>
            error.kind() == std::io::ErrorKind::NotFound,
        CatalogError::UnsupportedVersion => true,
        _ => false,
    }
}

/// Opaque native service state. No public raw database/issuer accessor, Clone,
/// deserialization, or caller-selected actor. Its RootLock must outlive it.
pub struct ProductDatabase<'root> {
    root: &'root RootLock,
    connection: VerifiedDatabaseConnection<'root>,
    owner: OwnerIssuer,
    // Retain the metadata migration result. Missing old relationships do not
    // become execution authority or disappear from the admission cap ledger.
    session_binding_projection: super::session_transport::session_binding::ProjectionReport,
    process_custodian: ProcessCustodian,
    owner_login: Option<v37_login::OwnerLoginSession>,
    native_sessions: BTreeMap<(String, String), v37_runtime::NativeSession>,
    pending_native_launches: BTreeMap<(String, String), super::session_transport::launch::LaunchEvidence>,
    pending_credential_preparations: BTreeMap<(String, String),
        Vec<super::session_transport::credential_launch::CredentialPreparationCustody>>,
    // Keep the original metadata-only source holder after a completed legacy
    // baseline recovery. Model launches reuse this exact verified holder;
    // no cold holder can silently rebuild an active source DACL.
    recovered_credential_holders: BTreeMap<(String, String),
        std::sync::Arc<crate::process::CredentialBinding>>,
    // A different native qualification from the legacy boot fence. Keeping
    // this metadata holder preserves its read-only baseline adoption witness.
    disappeared_credential_holders: BTreeMap<(String, String),
        std::sync::Arc<crate::process::CredentialBinding>>,
}

impl<'root> ProductDatabase<'root> {
    pub fn open(root: &'root RootLock, database: &Path) -> Result<Self> {
        let mut connection = open_product_database(root, database)?;
        // The existing core record family shares this same verified DB.
        super::atomic::initialize_product_core_schema(&mut connection)?;
        super::context_state::initialize_context_state_schema(&mut connection)?;
        authority::initialize_decision_capacity_schema(&mut connection)?;
        authority::initialize_context_manifest_schema(&mut connection)?;
        authority::initialize_task_context_schema(&mut connection)?;
        authority::initialize_authorized_task_package_schema(&mut connection)?;
        authority::initialize_session_lineage_schema(&mut connection)?;
        authority::initialize_execution_recipe_schema(&mut connection)?;
        authority::initialize_process_custody_schema(&mut connection)?;
        // Reopen the already-persisted bootstrap identity, not a second owner or
        // grant store. initialize_profile checks the exact retained database pin.
        let owner = authority::initialize_profile(&mut connection, root)?;
        let session_binding_projection = super::session_transport::session_binding::project_legacy(&mut connection)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("H session relationship projection: {error:?}")))?;
        let process_custodian = ProcessCustodian::new()?;
        super::session_transport::rpc_journal::initialize_schema(&mut connection)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("native RPC schema: {error:?}")))?;
        // F owns the private Grok HOME ACL journal in this same verified DB.
        // Opening the DB initializes records only; it is not holder retirement.
        instance::initialize_grok_home_grant_schema(&mut connection)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("Grok HOME schema: {error}")))?;
        Ok(Self { root, connection, owner, session_binding_projection, process_custodian, owner_login: None,
            native_sessions: BTreeMap::new(), pending_native_launches: BTreeMap::new(),
            pending_credential_preparations: BTreeMap::new(),
            recovered_credential_holders: BTreeMap::new(),
            disappeared_credential_holders: BTreeMap::new() })
    }

    pub fn serve_pipe(&mut self, pipe: &PrivatePipeConnection) -> Result<()> {
        serve_pipe(&mut self.connection, pipe)
    }

    pub fn serve_authenticated_pipe(
        &mut self,
        pipe: &PrivatePipeConnection,
        service_capability: &str,
    ) -> Result<()> {
        serve_authenticated_pipe(&mut self.connection, &self.owner, &mut self.process_custodian, pipe, service_capability)
    }

    /// A main-thread service frame session for the multiplexed native loop.
    /// The returned state is opaque; the service pipe never owns native issuer
    /// or database state even when its I/O runs on another thread.
    pub fn begin_service_frames(&self, service_capability: &str) -> Result<ServiceFrameSession> {
        ServiceFrameSession::new(service_capability)
    }

    /// Product-owned hosts keep running when their Node service disconnects.
    /// In that mode the service capability cannot invoke Shutdown.
    pub fn begin_shared_service_frames(&self, service_capability: &str) -> Result<ServiceFrameSession> {
        ServiceFrameSession::new_shared(service_capability)
    }

    pub fn dispatch_service_frame(&mut self, state: &mut ServiceFrameSession,
        frame: &[u8]) -> Result<(Vec<u8>, bool)> {
        dispatch_service_frame(&mut self.connection, &self.owner,
            &mut self.process_custodian, state, frame)
    }

    /// A complete User frame can enter only through the dedicated pipe's
    /// process-object proof. Each operation is connected individually to its
    /// native store; all other closed-envelope operations stay unsupported.
    pub fn dispatch_user_frame(&mut self, origin: &UserOriginProof, frame: &[u8]) -> Result<Vec<u8>> {
        origin.verify_live_origin().map_err(OrchestrationError::Ipc)?;
        if v37_side::is_owner_side_frame(frame) {
            return self.dispatch_owner_side_frame(frame);
        }
        if v37_login::is_owner_instance_list_frame(frame) {
            return self.dispatch_owner_instance_list_frame(frame);
        }
        if v37_managed_cli::is_user_managed_cli_frame(frame) {
            return self.dispatch_user_managed_cli(frame);
        }
        if v37_login::is_owner_login_frame(frame) {
            return self.dispatch_owner_login_frame(frame);
        }
        if v37_seat::is_user_v37_configuration_frame(frame) {
            return self.configure_user_v37(frame);
        }
        let request = decode_request(frame).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("v37 user frame: {error:?}")))?;
        if request.family == "K-SESSION" && request.operation == "send" {
            // The input remains an authenticated User request when the wall
            // clock cannot supply a positive instant. H may send it; E must
            // omit only this presence witness, never invent a timestamp.
            let observed_at_ms = SystemTime::now().duration_since(UNIX_EPOCH)
                .ok().and_then(|elapsed|i64::try_from(elapsed.as_millis()).ok())
                .filter(|value|*value>0);
            let input = VerifiedDirectUserInput {origin, frame, observed_at_ms};
            return self.dispatch_verified_user_session(&request, &input);
        }
        self.dispatch_user_request(&request)
    }

    fn dispatch_user_request(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        if request.family == "K-SEAT" { return self.dispatch_user_seat(request); }
        if request.family == "K-POLICY" { return Ok(encode_receipt(request, V37Status::Unsupported,
            request.expected_revision, request.expected_revision, Default::default())); }
        if request.family == "K-SESSION" { return self.dispatch_user_session(request); }
        if request.family == "K-QCARD" { return self.dispatch_user_qcard(request); }
        if request.family == "K-LEDGER" { return self.dispatch_user_ledger(request); }
        if request.family == "K-INBOX" { return self.dispatch_native_inbox(request); }
        if request.family == "K-SIDE" { return self.dispatch_user_side(request); }
        if request.family == "K-WORKTREE" { return self.dispatch_user_worktree(request); }
        if request.family == "K-INSTANCE" {
            return match request.operation.as_str() {
                "register" => self.register_user_instance(request),
                "install-state" => self.read_user_instance(request, false),
                "login-state" => self.read_user_instance(request, true),
                "concurrency-input" => self.read_user_instance_capacity(request),
                "repin-after-manual-upgrade" => self.repin_user_instance(request),
                "version-and-new-version" => self.read_user_instance_version(request),
                _ => Ok(encode_receipt(request, V37Status::Unsupported,
                    request.expected_revision, request.expected_revision, Default::default())),
            };
        }
        Ok(encode_receipt(request, V37Status::Unsupported,
            request.expected_revision, request.expected_revision, Default::default()))
    }

    fn read_user_instance_version(&self, request: &V37Request) -> Result<Vec<u8>> {
        let current = self.read_registered_instance(&request.target_id)?;
        let revision = current.as_ref().map(|row| row.revision).unwrap_or(0);
        let mut result = BTreeMap::new();
        let status = if request.domain_id != "global" || !request.payload.is_empty() {
            V37Status::Denied
        } else if current.is_none() { V37Status::Conflict }
        else if request.expected_revision != revision { V37Status::Stale }
        else {
            let row = current.expect("present instance");
            result.insert(JsonString::from_str("version"), Json::String(JsonString::from_str(&row.version)));
            result.insert(JsonString::from_str("programDigest"), Json::String(JsonString::from_str(&row.program_digest)));
            if let Some(version)=instance::known_new_version(&row.driver_id,&row.version) {
                result.insert(JsonString::from_str("newVersion"),Json::String(JsonString::from_str(version)));
            }
            V37Status::Applied
        };
        Ok(encode_receipt(request, status, revision, revision, result))
    }

    /// F.2 manual upgrade: native observation, no caller-supplied program pin,
    /// no auto installer and no persistent home/credential operation.
    fn repin_user_instance(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let revision = self.user_instance_revision(&request.target_id)?;
        let respond = |status, before, after, reason: Option<String>| {
            let mut result = BTreeMap::new();
            if let Some(reason) = reason {
                result.insert(JsonString::from_str("reason"), Json::String(JsonString::from_str(&reason)));
            }
            encode_receipt(request, status, before, after, result)
        };
        if request.domain_id != "global" || !request.payload.is_empty()
            || request.expected_revision == 0 || request.expected_revision >= i64::MAX as u64 {
            return Ok(respond(V37Status::Denied, revision, revision, None));
        }
        let input = instance::ProgramRepin { request_id: &request.request_id,
            request_bytes: &request.raw_bytes, instance_id: &request.target_id,
            expected_revision: request.expected_revision as i64 };
        let outcome = match instance::reconcile_program_repin(&self.connection, self.root, &input) {
            Ok(Some(replay)) => Ok(replay),
            Err(error) => Err(error),
            Ok(None) => {
                if revision != request.expected_revision {
                    return Ok(respond(V37Status::Stale, revision, revision, None));
                }
                if self.owner_login.as_ref().is_some_and(|session|
                    v37_login::pending_login_for_instance(session, &request.target_id))
                    || self.native_sessions.values().any(|session|
                        session.evidence.instance_id() == request.target_id) {
                    return Ok(respond(V37Status::Conflict, revision, revision,
                        Some("current native instance process custody is pending".into())));
                }
                let row = self.read_registered_instance(&request.target_id)?
                    .ok_or(OrchestrationError::Invalid("instance for manual upgrade"))?;
                if row.driver_id != "codex" { return Ok(respond(V37Status::Denied, revision, revision, None)); }
                let managed=Statement::prepare(self.connection.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
                managed.bind_text(1,&row.driver_id)?;
                if managed.step_row()? { return Ok(respond(V37Status::Denied,revision,revision,
                    Some("manual global CLI repin is unavailable after managed lifecycle starts".into()))); }
                let source = self.registration_source(&request.target_id, &row.driver_id)?;
                if !source.as_ref().map(|source|
                    self.registered_home_is_current(source, &request.target_id)).transpose()?.unwrap_or(false) {
                    return Ok(respond(V37Status::Unknown, revision, revision,
                        Some("manual upgrade registered home identity is not confirmed".into())));
                }
                let observed = match instance::discover_program("codex") {
                    Ok(program) => program,
                    Err(error) => return Ok(respond(V37Status::Failed, revision, revision,
                        Some(format!("manual upgrade native program observation: {error:?}")))),
                };
                let digest = format!("sha256:{}", gogoke_lpac_path_compat::OBSERVED_CLI_SHA256);
                if !observed.matches_pin(&digest, "0.160.0") {
                    return Ok(respond(V37Status::Denied, revision, revision,
                        Some("manual upgrade does not match the fixed native CLI identity".into())));
                }
                instance::repin_program(&mut self.connection, self.root, &self.owner, &input, &observed)
            },
        };
        match outcome {
            Ok(receipt) => {
                let mut result = BTreeMap::new();
                result.insert(JsonString::from_str("programDigest"), Json::String(JsonString::from_str(&receipt.program_digest)));
                result.insert(JsonString::from_str("version"), Json::String(JsonString::from_str(&receipt.version)));
                result.insert(JsonString::from_str("loginState"), Json::String(JsonString::from_str("UNKNOWN")));
                Ok(encode_receipt(request, if receipt.disposition == RegistrationDisposition::Replayed {
                    V37Status::Replayed } else { V37Status::Applied }, request.expected_revision,
                    receipt.revision as u64, result))
            },
            Err(error) => Ok(respond(match error {
                RegistryError::RequestConflict | RegistryError::InstanceConflict => V37Status::Conflict,
                RegistryError::Invalid(_) | RegistryError::IdentityChanged | RegistryError::Authority(_) => V37Status::Denied,
                _ => V37Status::Unknown,
            }, revision, revision, Some(format!("native manual program repin: {error:?}")))),
        }
    }

    fn read_registered_instance(&self, instance_id: &str) -> Result<Option<RegisteredInstance>> {
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT driver_id,program_digest,version,login_state,revision \
             FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
        query.bind_text(1, instance_id)?;
        if !query.step_row()? {
            return Ok(None);
        }
        let driver_id = query.column_text(0)?;
        let program_digest = query.column_text(1)?;
        let version = query.column_text(2)?;
        let login_state = query.column_text(3)?;
        let revision = query.column_text(4)?.parse::<u64>().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("instance revision: {error}")))?;
        if revision == 0 || query.step_row()? {
            return Err(OrchestrationError::Invalid("instance row"));
        }
        Ok(Some(RegisteredInstance {
            driver_id,
            program_digest,
            version,
            login_state,
            revision,
        }))
    }

    /// Recover the original registration bytes from F's durable fingerprint.
    /// The operation journal is the only source for the registration identity;
    /// a read request must never supply or reconstruct a path/digest claim.
    fn registration_source(
        &self,
        instance_id: &str,
        driver_id: &str,
    ) -> Result<Option<RegistrationSource>> {
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id,request_hex,phase FROM main.gogoke_v37_instance_operations \
             WHERE target_id=?1 AND phase='APPLIED' ORDER BY request_id")?;
        query.bind_text(1, instance_id)?;
        let mut source = None;
        while query.step_row()? {
            let request_id = query.column_text(0)?;
            let fields = decode_framed_hex(&query.column_text(1)?)?;
            // Observation fingerprints use six fields and are not registration
            // authority. Only the exact five-field registration fingerprint is
            // eligible to establish the persistent home and program pin.
            if fields.len() != 5 {
                continue;
            }
            if fields[1].as_slice() != instance_id.as_bytes() {
                return Err(OrchestrationError::V37StoreFailure(
                    "registration journal target mismatch".into(),
                ));
            }
            let request = decode_request(&fields[0]).map_err(|error|
                OrchestrationError::V37StoreFailure(format!(
                    "registration journal request: {error:?}"
                )))?;
            if request.family != "K-INSTANCE"
                || request.operation != "register"
                || request.domain_id != "global"
                || request.target_id != instance_id
                || request.payload.len() != 1
                || user_payload_string(&request, "driverId")?.as_str() != driver_id
            {
                return Err(OrchestrationError::V37StoreFailure(
                    "registration journal identity mismatch".into(),
                ));
            }
            if source.is_some() {
                return Err(OrchestrationError::V37StoreFailure(
                    "duplicate registration journal".into(),
                ));
            }
            source = Some(RegistrationSource { request_id, request_bytes: fields[0].clone() });
        }
        Ok(source)
    }

    fn registered_home_is_current(
        &self,
        request: &RegistrationSource,
        instance_id: &str,
    ) -> Result<bool> {
        match instance::reconcile_register_replay(
            &self.connection,
            self.root,
            &request.request_id,
            instance_id,
            &request.request_bytes,
        ) {
            Ok(RegistrationReplay::Replayed) => Ok(true),
            Ok(RegistrationReplay::Unseen | RegistrationReplay::Pending) => Ok(false),
            Err(error) => Err(OrchestrationError::V37StoreFailure(format!(
                "instance home observation: {error:?}"
            ))),
        }
    }

    fn current_install_fact(
        &self,
        row: &RegisteredInstance,
        source: &RegistrationSource,
        instance_id: &str,
    ) -> Result<InstallFact> {
        if !self.registered_home_is_current(source, instance_id)? {
            return Ok(InstallFact::Unknown);
        }
        // H's launch resolver owns the same native catalog check. Reusing it
        // keeps this read observational: no registration or revision write is
        // attempted, and true is possible only for the current pinned bytes
        // and version.
        match instance::locate_bound_instance_program(
            &self.connection, instance_id, &row.driver_id,
            &row.program_digest, &row.version,
        ) {
            Ok(_) => Ok(InstallFact::Installed),
            Err(instance::ProgramSourceError::Legacy(error)) if catalog_error_is_missing(&error) => Ok(InstallFact::Missing),
            Err(_) => Ok(InstallFact::Unknown),
        }
    }

    /// A persisted login value is a last trusted local observation only when
    /// the latest revision was produced by one APPLIED native login observation
    /// for this instance. It does not prove present credential validity.
    fn current_login_observation(
        &self,
        instance_id: &str,
        revision: u64,
        state: &str,
    ) -> Result<bool> {
        if revision <= 1 {
            return Ok(false);
        }
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_instance_operations \
             WHERE target_id=?1 AND phase='APPLIED'")?;
        query.bind_text(1, instance_id)?;
        let expected_revision = (revision - 1).to_string();
        let mut current = None;
        while query.step_row()? {
            let fields = decode_framed_hex(&query.column_text(0)?)?;
            // Registration fingerprints have five fields. F.1 observations
            // have six; any other APPLIED shape cannot prove current login.
            if fields.len() != 6 {
                continue;
            }
            if fields[0].as_slice() != b"observe"
                || fields[1].is_empty()
                || fields[2].as_slice() != instance_id.as_bytes()
            {
                return Err(OrchestrationError::V37StoreFailure(
                    "login observation identity is invalid".into(),
                ));
            }
            let observed_revision = std::str::from_utf8(&fields[3])
                .map_err(|_| OrchestrationError::V37StoreFailure(
                    "login observation revision is invalid".into()))?;
            if observed_revision != expected_revision {
                continue;
            }
            let field = fields[4].as_slice();
            let value = fields[5].as_slice();
            if field != b"login" && field != b"install" {
                return Err(OrchestrationError::V37StoreFailure(
                    "login observation field is invalid".into(),
                ));
            }
            if field == b"login"
                && value != b"UNKNOWN" && value != b"LOGGED_IN" && value != b"LOGGED_OUT"
            {
                return Err(OrchestrationError::V37StoreFailure(
                    "login observation value is invalid".into(),
                ));
            }
            if field == b"install"
                && value != b"UNKNOWN" && value != b"INSTALLED" && value != b"MISSING"
            {
                return Err(OrchestrationError::V37StoreFailure(
                    "install observation value is invalid".into(),
                ));
            }
            if current.is_some() {
                return Err(OrchestrationError::V37StoreFailure(
                    "duplicate current instance observation".into(),
                ));
            }
            current = Some(field == b"login" && value == state.as_bytes());
        }
        Ok(current == Some(true))
    }

    fn read_user_instance(
        &mut self,
        request: &V37Request,
        login: bool,
    ) -> Result<Vec<u8>> {
        let row = match self.read_registered_instance(&request.target_id) {
            Ok(Some(row)) => row,
            Ok(None) => return Ok(encode_receipt(request, V37Status::Conflict, 0, 0,
                Default::default())),
            Err(error) => return Ok(encode_receipt(request, V37Status::Unknown,
                0, 0, BTreeMap::from([(JsonString::from_str("reason"),
                    Json::String(JsonString::from_str(&format!("instance row: {error:?}"))))]))),
        };
        let current = row.revision;
        let receipt = |status, result| encode_receipt(request, status, current, current, result);
        if request.domain_id != "global" || !request.payload.is_empty() {
            return Ok(receipt(V37Status::Denied, Default::default()));
        }
        if request.expected_revision != current {
            return Ok(receipt(V37Status::Stale, Default::default()));
        }
        let source = match self.registration_source(&request.target_id, &row.driver_id) {
            Ok(Some(source)) => source,
            Ok(None) => {
                let result = if login {
                    BTreeMap::from([(JsonString::from_str("state"),
                        Json::String(JsonString::from_str("UNKNOWN")))])
                } else {
                    BTreeMap::from([(JsonString::from_str("installed"), Json::Bool(false))])
                };
                return Ok(receipt(V37Status::Unknown, result));
            }
            Err(error) => {
                let result = if login {
                    BTreeMap::from([(JsonString::from_str("state"),
                        Json::String(JsonString::from_str("UNKNOWN")))])
                } else {
                    BTreeMap::from([(JsonString::from_str("installed"), Json::Bool(false))])
                };
                let mut result = result;
                result.insert(JsonString::from_str("reason"),
                    Json::String(JsonString::from_str(&format!("registration source: {error:?}"))));
                return Ok(receipt(V37Status::Unknown, result));
            }
        };
        if login {
            match self.registered_home_is_current(&source, &request.target_id) {
                Ok(true) => {}
                Ok(false) | Err(_) => return Ok(receipt(V37Status::Unknown, BTreeMap::from([
                    (JsonString::from_str("state"),
                        Json::String(JsonString::from_str("UNKNOWN"))),
                ]))),
            }
            let state = row.login_state.as_str();
            if !matches!(state, "UNKNOWN" | "LOGGED_IN" | "LOGGED_OUT") {
                return Ok(receipt(V37Status::Unknown, BTreeMap::from([
                    (JsonString::from_str("state"),
                        Json::String(JsonString::from_str("UNKNOWN"))),
                ])));
            }
            if state != "UNKNOWN"
                && !matches!(self.current_login_observation(
                    &request.target_id,
                    current,
                    state,
                ), Ok(true))
            {
                return Ok(receipt(V37Status::Unknown, BTreeMap::from([
                    (JsonString::from_str("state"),
                        Json::String(JsonString::from_str("UNKNOWN"))),
                ])));
            }
            return Ok(receipt(V37Status::Applied, BTreeMap::from([
                (JsonString::from_str("state"), Json::String(JsonString::from_str(state))),
            ])));
        }
        let fact = match self.current_install_fact(&row, &source, &request.target_id) {
            Ok(fact) => fact,
            Err(error) => {
                return Ok(receipt(V37Status::Unknown, BTreeMap::from([
                    (JsonString::from_str("installed"), Json::Bool(false)),
                    (JsonString::from_str("reason"),
                        Json::String(JsonString::from_str(&format!(
                            "install observation: {error:?}"
                        )))),
                ])));
            }
        };
        let installed = matches!(fact, InstallFact::Installed);
        let status = match fact {
            InstallFact::Installed | InstallFact::Missing => V37Status::Applied,
            InstallFact::Unknown => V37Status::Unknown,
        };
        Ok(receipt(status, BTreeMap::from([
            (JsonString::from_str("installed"), Json::Bool(installed)),
        ])))
    }

    fn register_user_instance(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        let receipt = |status, previous, revision, reason: Option<String>| {
            let mut result = std::collections::BTreeMap::new();
            if let Some(reason) = reason {
                result.insert(JsonString::from_str("reason"),
                    Json::String(JsonString::from_str(&reason)));
            }
            encode_receipt(request, status, previous, revision, result)
        };
        match instance::reconcile_register_replay(&self.connection, self.root,
            &request.request_id, &request.target_id, &request.raw_bytes) {
            Ok(RegistrationReplay::Unseen) => (),
            Ok(RegistrationReplay::Pending) =>
                return Ok(receipt(V37Status::Unknown, 0, 0, None)),
            // A prior F registration may have committed before its managed
            // source bind. Re-enter the exact original request and bind below;
            // never return REPLAYED while H would reject an unbound source.
            Ok(RegistrationReplay::Replayed) => (),
            Err(RegistryError::RequestConflict) =>
                return Ok(receipt(V37Status::Conflict, 0, 0, None)),
            Err(RegistryError::Invalid(_)) =>
                return Ok(receipt(V37Status::Denied, 0, 0, None)),
            Err(error) => return Ok(receipt(V37Status::Unknown, 0, 0,
                Some(format!("instance replay: {error:?}")))),
        }
        if request.domain_id != "global" || request.payload.len() != 1 {
            return Ok(receipt(V37Status::Denied, 0, 0, None));
        }
        let current = match self.user_instance_revision(&request.target_id) {
            Ok(revision) => revision,
            Err(error) => return Ok(receipt(V37Status::Unknown, 0, 0,
                Some(format!("instance revision: {error:?}")))),
        };
        if request.expected_revision != 0 {
            return Ok(receipt(V37Status::Stale, current, current, None));
        }
        let driver = match user_payload_string(request, "driverId") {
            Ok(driver) => driver,
            Err(_) => return Ok(receipt(V37Status::Denied, current, current, None)),
        };
        let pin = match instance::read_fixed_official_cli(&driver) {
            Some(pin) => pin,
            None => return Ok(receipt(V37Status::Unsupported,current,current,
                Some("no qualified managed official CLI".into()))),
        };
        let digest = format!("sha256:{}",pin.image_sha256);
        let managed = instance::locate_ready_managed_program(&self.connection,self.root,
            &driver,&digest,pin.version);
        let observed = match managed.and_then(|path|path.ok_or(instance::ManagedCliError::IdentityChanged))
            .and_then(|path|instance::ProgramObservation::observe(&path,pin.version)
                .map_err(|error|instance::ManagedCliError::Observation(format!("{error:?}")))) {
            Ok(observed) => observed,
            Err(error) => {
                return Ok(receipt(V37Status::Denied,current,current,
                    Some(format!("managed official CLI unavailable: {error:?}"))));
            }
        };
        let disposition = instance::register_instance(&mut self.connection, self.root,
            &Registration {
                request_id: &request.request_id,
                request_bytes: &request.raw_bytes,
                instance_id: &request.target_id,
                driver_id: &driver,
                program: &observed,
            });
        let (status, reason) = match disposition {
            Ok(applied) => {
                let copy=instance::read_managed_cli(&self.connection,self.root,&driver)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("managed register source read: {error:?}")))?
                    .ok_or(OrchestrationError::AccessDenied)?;
                let stage=copy.stage_name.ok_or(OrchestrationError::AccessDenied)?;
                match instance::bind_managed_instance_program(&mut self.connection,self.root,&self.owner,
                    &request.target_id,&stage,&request.request_id) {
                    Ok(()) => (if applied==RegistrationDisposition::Applied {V37Status::Applied}
                        else {V37Status::Replayed},None),
                    Err(error) => (V37Status::Unknown,Some(format!("managed source bind: {error:?}"))),
                }
            },
            Err(error @ (RegistryError::RequestConflict | RegistryError::InstanceConflict)) =>
                (V37Status::Conflict, Some(format!("native instance register: {error:?}"))),
            Err(error) => (V37Status::Unknown,
                Some(format!("native instance register: {error:?}"))),
        };
        let next = if matches!(status, V37Status::Applied | V37Status::Replayed) { 1 }
            else { current };
        let previous = if matches!(status, V37Status::Applied | V37Status::Replayed) { 0 }
            else { current };
        Ok(receipt(status, previous, next, reason))
    }

    fn user_instance_revision(&self, instance_id: &str) -> Result<u64> {
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT revision FROM main.gogoke_v37_instances WHERE instance_id=?1")
            .map_err(OrchestrationError::Atomic)?;
        query.bind_text(1, instance_id).map_err(OrchestrationError::Atomic)?;
        if !query.step_row().map_err(OrchestrationError::Atomic)? { return Ok(0); }
        let revision = query.column_text(0).map_err(OrchestrationError::Atomic)?
            .parse().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("instance revision: {error}")))?;
        if query.step_row().map_err(OrchestrationError::Atomic)? {
            return Err(OrchestrationError::Invalid("duplicate instance"));
        }
        Ok(revision)
    }

    pub fn serve_lines<R: BufRead, W: Write>(&mut self, input: R, output: &mut W) -> Result<()> {
        serve_lines(&mut self.connection, input, output)
    }

    pub fn close_checked(self) -> std::result::Result<OpenLedger, SameOpenError> {
        let Self { root: _, connection, owner: _, session_binding_projection, process_custodian, owner_login, native_sessions,
            pending_native_launches, pending_credential_preparations, recovered_credential_holders,
            disappeared_credential_holders } = self;
        drop(owner_login);
        // Closing the Job first prevents a child from outliving the active
        // coordination database. Unresolved rows stay UNKNOWN on recovery.
        drop(process_custodian);
        drop(native_sessions);
        drop(pending_native_launches);
        drop(pending_credential_preparations);
        drop(recovered_credential_holders);
        drop(disappeared_credential_holders);
        drop(session_binding_projection);
        connection.close_checked()
    }

    // Trusted in-process composition ONLY. These are not IPC operations and do
    // not authenticate a Node/model caller merely because it is the same OS user.
    // A future Owner UI / seat ingress must supply its own established admission.
    pub(crate) fn issue_grant(
        &mut self,
        policy: &str,
        revocation: &str,
        spec: GrantSpec,
    ) -> Result<GrantRef> {
        authority::issue_owner_grant(&mut self.connection, &self.owner, policy, revocation, spec)
    }

    pub(crate) fn revise_grant(
        &mut self,
        policy: &str,
        expected: &GrantRef,
        spec: GrantSpec,
    ) -> Result<GrantRef> {
        authority::revise_owner_grant(&mut self.connection, &self.owner, policy, expected, spec)
    }

    pub(crate) fn revoke_grant(&mut self, policy: &str, expected: &GrantRef) -> Result<String> {
        authority::revoke_owner_grant(&mut self.connection, &self.owner, policy, expected)
    }

    pub(crate) fn delegate_grant(
        &mut self,
        policy: &str,
        parent: &GrantRef,
        spec: GrantSpec,
    ) -> Result<GrantRef> {
        authority::delegate_owner_grant(&mut self.connection, &self.owner, policy, parent, spec)
    }

    pub(crate) fn issue_delegation(
        &mut self,
        input: DelegationGrantInput,
    ) -> Result<DelegationGrantSnapshot> {
        authority::issue_owner_delegation(&mut self.connection, &self.owner, input)
    }

    pub(crate) fn delegate_delegation(
        &mut self,
        parent: &DelegationGrantIdentity,
        input: DelegationGrantInput,
    ) -> Result<DelegationGrantSnapshot> {
        authority::delegate_owner_delegation(&mut self.connection, &self.owner, parent, input)
    }

    pub(crate) fn read_delegation(&mut self, grant_id: &str) -> Result<DelegationGrantSnapshot> {
        authority::read_current_delegation(&mut self.connection, grant_id)
    }

    pub(crate) fn revise_delegation(
        &mut self,
        identity: &DelegationGrantIdentity,
        input: DelegationGrantInput,
    ) -> Result<DelegationGrantSnapshot> {
        authority::revise_owner_delegation(&mut self.connection, &self.owner, identity, input)
    }

    pub(crate) fn revoke_delegation(
        &mut self,
        identity: &DelegationGrantIdentity,
    ) -> Result<String> {
        authority::revoke_owner_delegation(&mut self.connection, &self.owner, identity)
    }

    pub(crate) fn read_context(
        &mut self,
        request: &ContextReadRequest,
    ) -> Result<ContextReadSnapshot> {
        authority::read_owner_context(&mut self.connection, &self.owner, request)
    }

    pub(crate) fn read_context_set(
        &mut self,
        requests: &[ContextReadRequest],
    ) -> Result<ContextReadSet> {
        authority::read_owner_context_set(&mut self.connection, &self.owner, requests)
    }

    /// Grant-subject read for the authenticated main service. Grant lineage,
    /// ACL, Owner-private rules and lifecycle are all rechecked natively.
    pub(crate) fn read_grantee_context_set(
        &mut self,
        requests: &[GranteeContextReadRequest],
    ) -> Result<AuthorizedContextReadSet> {
        authority::read_grantee_context_set(&mut self.connection, requests)
    }

    pub(crate) fn publish_context_assembly_snapshot(
        &mut self,
        snapshot: &ContextAssemblySnapshot,
    ) -> Result<()> {
        authority::publish_context_assembly_snapshot(&mut self.connection, snapshot)
    }

    pub(crate) fn read_context_assembly_basis(
        &mut self,
        identity: &ContextManifestReplayIdentity,
    ) -> Result<ContextAssemblyBasis> {
        authority::read_context_assembly_basis(&mut self.connection, identity)
    }

    pub(crate) fn list_context_assembly_sources(
        &mut self,
        identity: &ContextManifestReplayIdentity,
    ) -> Result<Vec<ContextAssemblySource>> {
        authority::list_context_assembly_sources(&mut self.connection, identity)
    }

    pub(crate) fn commit_context_manifest(
        &mut self,
        input: &ContextManifestCommitInput,
    ) -> Result<ContextManifestAuthorityReceipt> {
        authority::commit_context_manifest(&mut self.connection, input)
    }

    pub(crate) fn read_context_manifest(
        &mut self,
        identity: &ContextManifestReplayIdentity,
    ) -> Result<ContextManifestAuthorityReceipt> {
        authority::read_context_manifest(&mut self.connection, identity)
    }

    pub(crate) fn commit_task_context_requirements(
        &mut self,
        input: &CommitTaskContextRequirements,
    ) -> Result<TaskContextRequirementsReceipt> {
        authority::commit_task_context_requirements(&mut self.connection, input)
    }

    pub(crate) fn read_task_context_requirements(
        &mut self,
        domain_id: &str,
        task_id: &str,
    ) -> Result<TaskContextRequirements> {
        authority::read_task_context_requirements(&mut self.connection, domain_id, task_id)
    }

    /// The caller supplies only material identities and revisions. Product Authority
    /// resolves their current records in the same transaction as the package write.
    /// The receipt remains preparatory and does not establish production ingress.
    pub(crate) fn prepare_authorized_task_package(
        &mut self,
        input: &PrepareAuthorizedTaskPackage,
    ) -> Result<AuthorizedTaskPackageReceipt> {
        authority::prepare_authorized_task_package(&mut self.connection, input)
    }

    pub(crate) fn read_authorized_task_package(
        &mut self,
        domain_id: &str,
        operation_id: &str,
    ) -> Result<AuthorizedTaskPackageReceipt> {
        authority::read_authorized_task_package(&mut self.connection, domain_id, operation_id)
    }

    /// Private preparatory ingress through ProductDatabase's unforgeable Owner
    /// capability. This typed record does not authorize dispatch or material reads by workers.
    pub(crate) fn append_task_material(
        &mut self,
        input: &AppendTaskMaterial,
    ) -> Result<TaskMaterialReceipt> {
        authority::append_trusted_task_material(&mut self.connection, &self.owner, input)
    }

    /// Reads through the same private in-process Owner capability. Production
    /// PolicyAuthorityPort resolution remains a separate integration step.
    pub(crate) fn read_task_material(
        &mut self,
        domain_id: &str,
        material_id: &str,
    ) -> Result<TaskMaterialVersion> {
        authority::read_trusted_task_material(
            &mut self.connection,
            &self.owner,
            domain_id,
            material_id,
        )
    }

    pub(crate) fn promote(
        &mut self,
        request: &PromotionRequest,
        target: ContextCommand,
    ) -> Result<ContextReceipt> {
        authority::commit_owner_promotion(&mut self.connection, &self.owner, request, target)
    }

    /// Owner-authored data only, not an objective score or an IPC privilege.
    /// The existing canonical tables and Decision source must already be present;
    /// absent schema/source fails closed. Product core-schema startup is separate.
    pub(crate) fn append_owner_outcome(
        &mut self,
        request: &OwnerOutcomeAppend,
    ) -> Result<DomainRecordReceipt> {
        authority::append_owner_override_outcome(&mut self.connection, &self.owner, request)
    }

    /// Append an OBJECTIVE only from same-domain durable authority records.
    pub(crate) fn append_objective_outcome(
        &mut self,
        request: &AppendObjectiveOutcome,
    ) -> Result<DomainRecordReceipt> {
        authority::append_objective_outcome(&mut self.connection, request)
    }

    pub(crate) fn read_objective_outcome(
        &mut self,
        domain_id: &str,
        outcome_id: &str,
        revision: &str,
    ) -> Result<ObjectiveOutcomeVersion> {
        authority::read_objective_outcome(&mut self.connection, domain_id, outcome_id, revision)
    }

    pub(crate) fn append_evaluation(
        &mut self,
        request: &AppendEvaluation,
    ) -> Result<DomainRecordReceipt> {
        authority::append_evaluation(&mut self.connection, request)
    }

    pub(crate) fn read_evaluation(
        &mut self,
        domain_id: &str,
        evaluation_id: &str,
        revision: &str,
    ) -> Result<EvaluationReceipt> {
        authority::read_evaluation(&mut self.connection, domain_id, evaluation_id, revision)
    }

    /// Dream records are durable candidate evidence only. This typed path cannot
    /// activate a proposal or write production facts.
    pub(crate) fn append_dream_run(&mut self, request: &AppendDreamRun) -> Result<DomainRecordReceipt> {
        authority::append_dream_run(&mut self.connection, request)
    }

    pub(crate) fn append_dream_proposal(
        &mut self,
        request: &AppendDreamProposal,
    ) -> Result<DomainRecordReceipt> {
        authority::append_dream_proposal(&mut self.connection, request)
    }

    pub(crate) fn read_dream_run(
        &mut self,
        domain_id: &str,
        run_id: &str,
        revision: &str,
    ) -> Result<DreamReceipt> {
        authority::read_dream_run(&mut self.connection, domain_id, run_id, revision)
    }

    pub(crate) fn read_dream_proposal(
        &mut self,
        domain_id: &str,
        proposal_id: &str,
        revision: &str,
    ) -> Result<DreamReceipt> {
        authority::read_dream_proposal(&mut self.connection, domain_id, proposal_id, revision)
    }

    /// Private native-host session lineage mutation. This is durable metadata,
    /// not process custody, Action completion, or permission to replay work.
    pub(crate) fn apply_session_lineage(
        &mut self,
        command: &SessionLineageCommand,
    ) -> Result<SessionLineageReceipt> {
        authority::apply_session_lineage_command(&mut self.connection, command)
    }

    pub(crate) fn read_session_lineage(
        &mut self,
        domain_id: &str,
        session_id: &str,
    ) -> Result<SessionSnapshot> {
        authority::read_session_lineage(&mut self.connection, domain_id, session_id)
    }

    pub(crate) fn read_exposure_receipt(
        &mut self,
        domain_id: &str,
        receipt_id: &str,
    ) -> Result<authority::StoredExposureReceipt> {
        authority::read_exposure_receipt(&mut self.connection, domain_id, receipt_id)
    }

    /// Durable history only. The caller must separately establish current
    /// eligibility/capacity/binding before using a replayed Decision.
    pub(crate) fn read_decision_replay(
        &mut self,
        domain_id: &str,
        operation_id: &str,
    ) -> Result<DurableDecisionReplay> {
        authority::read_durable_decision_replay(&mut self.connection, domain_id, operation_id)
    }

    /// Persist an operation-scoped current candidate/capacity snapshot for the
    /// single main-service scheduler. This does not choose a candidate.
    pub(crate) fn publish_decision_snapshot(
        &mut self,
        snapshot: &DecisionAuthoritySnapshot,
    ) -> Result<()> {
        authority::publish_decision_snapshot(&mut self.connection, snapshot)
    }

    pub(crate) fn commit_decision(
        &mut self,
        input: &DecisionCommitInput,
    ) -> Result<DecisionCommitReceipt> {
        authority::commit_decision(&mut self.connection, input)
    }

    /// Private typed Owner-control ingress for versioned configuration. The
    /// receipt explicitly leaves runtime/model/admission currentness unresolved.
    pub(crate) fn append_execution_recipe(
        &mut self,
        input: &AppendExecutionRecipe,
    ) -> Result<ExecutionRecipeReceipt> {
        authority::append_owner_execution_recipe(&mut self.connection, &self.owner, input)
    }

    pub(crate) fn read_current_execution_recipe(
        &mut self,
        domain_id: &str,
        recipe_id: &str,
    ) -> Result<Option<ExecutionRecipeVersion>> {
        authority::read_current_execution_recipe(
            &mut self.connection,
            &self.owner,
            domain_id,
            recipe_id,
        )
    }

    pub(crate) fn read_execution_recipe_revision(
        &mut self,
        domain_id: &str,
        recipe_id: &str,
        revision: &str,
    ) -> Result<Option<ExecutionRecipeVersion>> {
        authority::read_execution_recipe_revision(
            &mut self.connection,
            &self.owner,
            domain_id,
            recipe_id,
            revision,
        )
    }
}

#[cfg(test)]
#[path = "product_database_execution_recipe_tests.rs"]
mod execution_recipe_tests;
#[cfg(test)]
#[path = "product_database_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
mod secretary_user_input_tests {
    use super::*;
    use crate::ipc::PrivatePipeListener;
    use crate::store::same_open::route_b_test_guard;
    use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, StoreTemplate};
    use crate::store::session_transport::{self as h, StdinRequest, PrepareDisposition};
    use std::fs::OpenOptions;

    fn count(db: &VerifiedDatabaseConnection<'_>, table: &str) -> String {
        let sql = format!("SELECT COUNT(*) FROM main.{table}");
        let row = Statement::prepare(db.as_ptr(), &sql).unwrap();
        assert!(row.step_row().unwrap());
        row.column_text(0).unwrap()
    }

    fn episode(db:&VerifiedDatabaseConnection<'_>, session:&str, process:&str,
        open:&str, home:&str, binding:&str, incarnation:&str) {
        let row=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('global',?1,?2,'1',NULL,'6f70656e',0,1,?3,'instanceA',?4,?5,'secretaryA',?6,'ACTIVE')").unwrap();
        for (index,value) in [open,session,process,home,binding,incarnation].iter().enumerate() {
            row.bind_text((index+1) as i32,value).unwrap();
        }
        row.step_done().unwrap();
        let generation=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('global',?1,'1',?2,?3)").unwrap();
        for (index,value) in [session,open,process].iter().enumerate() {
            generation.bind_text((index+1) as i32,value).unwrap();
        }
        generation.step_done().unwrap();
    }

    #[test]
    fn only_exact_live_user_input_adds_an_atomic_fixed_presence_witness() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!("gogoke-user-presence-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let mut product=ProductDatabase::open(&root,&folder.join("state.sqlite")).unwrap();
        seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),
            StoreTemplate {domain_id:"global",template_id:"secretaryBase",settings_json:br#"{}"#}).unwrap();
        let seat=seat::create(&mut product.connection,NativeOrigin::user(&product.owner),
            CreateSeat {domain_id:"global",seat_id:"secretaryA",template_id:"secretaryBase",
                instance_id:None,kind:Kind::Long,request_id:"createSecretaryA",
                request_bytes:b"create secretary A"}).unwrap().seat;
        seat::designate_secretary(&mut product.connection,&product.owner,&seat.seat_id,
            &seat.incarnation,"designateSecretaryA",b"designate secretary A").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','global','SESSION','sessionA','1','ACTIVE')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','sessionA','instanceA','homeA','bindingA','1','COMMITTED',1,'processA')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','global','1','ACTIVE')").unwrap();
        episode(&product.connection,"sessionA","processA","openA","homeA","bindingA",&seat.incarnation);

        let endpoint=format!("presence-{}-{nonce}",std::process::id());
        let listener=PrivatePipeListener::bind_user(&endpoint,std::process::id()).unwrap();
        let path=listener.path().to_owned();
        let (release,held)=std::sync::mpsc::channel::<()>();
        let client=std::thread::spawn(move|| {
            let _pipe=OpenOptions::new().read(true).write(true).open(path).unwrap();
            held.recv().unwrap();
        });
        let mut pipe=listener.accept_user().unwrap();
        let proof=pipe.take_user_origin_proof().unwrap();
        let raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","domainId":"global","expectedRevision":"1","payload":{"generation":"1","body":"Owner original"}}"#;
        let source=VerifiedDirectUserInput {origin:&proof,frame:raw,observed_at_ms:Some(100)};
        let input=StdinRequest {domain_id:"global",session_id:"sessionA",ticket:"pct1_ticketA",
            generation:"1",request_bytes:raw};
        let changed=raw.windows(5).position(|window|window==b"Owner").unwrap();
        let mut other=raw.to_vec();other[changed..changed+5].copy_from_slice(b"model");
        let wrong=StdinRequest {request_bytes:&other,..input};
        assert!(matches!(h::prepare_codex_request_with_user_input(&mut product.connection,
            &product.owner,&wrong,&source),Err(h::JournalError::Conflict)));
        assert_eq!(count(&product.connection,"gogoke_v37_h_stdin_journal"),"0");
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"0");

        product.connection.execute("UPDATE main.gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='bindingA'").unwrap();
        assert!(matches!(h::prepare_codex_request_with_user_input(&mut product.connection,
            &product.owner,&input,&source),Err(h::JournalError::Denied)));
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"0");
        product.connection.execute("UPDATE main.gogoke_v37_h_owner_binding SET state='ACTIVE' WHERE binding_id='bindingA'").unwrap();

        product.connection.execute("CREATE TRIGGER fail_presence BEFORE INSERT ON gogoke_v37_seat_secretary_presence BEGIN SELECT RAISE(ABORT,'presence fixture failure'); END").unwrap();
        assert!(matches!(h::prepare_codex_request_with_user_input(&mut product.connection,
            &product.owner,&input,&source),Err(h::JournalError::Seat(_))));
        assert_eq!(count(&product.connection,"gogoke_v37_h_stdin_journal"),"0",
            "E write failure rolls back the H intent before any child write");
        product.connection.execute("DROP TRIGGER fail_presence").unwrap();

        assert_eq!(h::prepare_codex_request_with_user_input(&mut product.connection,
            &product.owner,&input,&source).unwrap().disposition,PrepareDisposition::Prepared);
        assert_eq!(count(&product.connection,"gogoke_v37_h_stdin_journal"),"1");
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"1");
        let original=Statement::prepare(product.connection.as_ptr(),"SELECT request_hex FROM main.gogoke_v37_h_stdin_journal WHERE domain_id='global' AND request_id='sendA'").unwrap();
        assert!(original.step_row().unwrap());
        assert_eq!(original.column_text(0).unwrap(),raw.iter().map(|byte|format!("{byte:02x}")).collect::<String>());
        drop(original);
        let row=Statement::prepare(product.connection.as_ptr(),"SELECT source_operation_id,source_epoch,source_cursor,CAST(occurred_at_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_presence").unwrap();
        assert!(row.step_row().unwrap());
        assert_eq!((row.column_text(0).unwrap(),row.column_text(1).unwrap(),row.column_text(2).unwrap(),row.column_text(3).unwrap()),
            ("processA".into(),"nonceA".into(),"sendA".into(),"100".into()));
        drop(row);
        let later=VerifiedDirectUserInput {origin:&proof,frame:raw,observed_at_ms:Some(200)};
        assert_eq!(h::prepare_codex_request_with_user_input(&mut product.connection,
            &product.owner,&input,&later).unwrap().disposition,PrepareDisposition::Replayed);
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"1");
        let time=Statement::prepare(product.connection.as_ptr(),"SELECT CAST(observed_at_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_presence").unwrap();
        assert!(time.step_row().unwrap());assert_eq!(time.column_text(0).unwrap(),"100");drop(time);
        product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingD','instanceA','global','SESSION','sessionD','1','ACTIVE')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','sessionD','instanceA','homeD','bindingD','1','COMMITTED',1,'processD')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processD','pct1_ticketD','nonceD','14','1','fixture','sha256:fixture','profileD','global','1','ACTIVE')").unwrap();
        episode(&product.connection,"sessionD","processD","openD","homeD","bindingD",&seat.incarnation);
        let raw_d=String::from_utf8(raw.to_vec()).unwrap().replace("sendA","sendD").replace("sessionA","sessionD");
        let input_d=StdinRequest {domain_id:"global",session_id:"sessionD",ticket:"pct1_ticketD",
            generation:"1",request_bytes:raw_d.as_bytes()};
        let clock_backwards=VerifiedDirectUserInput {origin:&proof,frame:raw_d.as_bytes(),observed_at_ms:Some(99)};
        assert_eq!(h::prepare_codex_request_with_user_input(&mut product.connection,&product.owner,
            &input_d,&clock_backwards).unwrap().disposition,PrepareDisposition::Prepared);
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"2",
            "a lower real clock value is retained and the H User send remains prepared");
        let lower=Statement::prepare(product.connection.as_ptr(),"SELECT CAST(occurred_at_ms AS TEXT) FROM main.gogoke_v37_seat_secretary_presence WHERE source_cursor='sendD'").unwrap();
        assert!(lower.step_row().unwrap());assert_eq!(lower.column_text(0).unwrap(),"99");drop(lower);

        // A historical H row without this E witness cannot acquire one from
        // a later authenticated replay of its original bytes.
        product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingB','instanceA','global','SESSION','sessionB','1','ACTIVE')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','sessionB','instanceA','homeB','bindingB','1','COMMITTED',1,'processB')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processB','pct1_ticketB','nonceB','12','1','fixture','sha256:fixture','profileB','global','1','ACTIVE')").unwrap();
        episode(&product.connection,"sessionB","processB","openB","homeB","bindingB",&seat.incarnation);
        let raw_b=String::from_utf8(raw.to_vec()).unwrap().replace("sendA","sendB").replace("sessionA","sessionB");
        let input_b=StdinRequest {domain_id:"global",session_id:"sessionB",ticket:"pct1_ticketB",
            generation:"1",request_bytes:raw_b.as_bytes()};
        assert_eq!(h::prepare_codex_request(&mut product.connection,&input_b).unwrap().disposition,
            PrepareDisposition::Prepared);
        let historical=VerifiedDirectUserInput {origin:&proof,frame:raw_b.as_bytes(),observed_at_ms:Some(300)};
        assert_eq!(h::prepare_codex_request_with_user_input(&mut product.connection,&product.owner,
            &input_b,&historical).unwrap().disposition,PrepareDisposition::Replayed);
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"2");

        // An authenticated input with no usable host clock retains the H
        // request and supplies no invented E presence timestamp.
        product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingC','instanceA','global','SESSION','sessionC','1','ACTIVE')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','sessionC','instanceA','homeC','bindingC','1','COMMITTED',1,'processC')").unwrap();
        product.connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processC','pct1_ticketC','nonceC','13','1','fixture','sha256:fixture','profileC','global','1','ACTIVE')").unwrap();
        episode(&product.connection,"sessionC","processC","openC","homeC","bindingC",&seat.incarnation);
        let raw_c=String::from_utf8(raw.to_vec()).unwrap().replace("sendA","sendC").replace("sessionA","sessionC");
        let input_c=StdinRequest {domain_id:"global",session_id:"sessionC",ticket:"pct1_ticketC",
            generation:"1",request_bytes:raw_c.as_bytes()};
        let no_clock=VerifiedDirectUserInput {origin:&proof,frame:raw_c.as_bytes(),observed_at_ms:None};
        assert_eq!(h::prepare_codex_request_with_user_input(&mut product.connection,&product.owner,
            &input_c,&no_clock).unwrap().disposition,PrepareDisposition::Prepared);
        assert_eq!(count(&product.connection,"gogoke_v37_h_stdin_journal"),"4");
        assert_eq!(count(&product.connection,"gogoke_v37_seat_secretary_presence"),"2");
        release.send(()).unwrap();client.join().unwrap();
        drop(proof);drop(pipe);
        product.close_checked().unwrap();drop(root);
        std::fs::remove_dir_all(folder).unwrap();
    }
}
