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
use super::instance::{self, CatalogError, Registration, RegistrationDisposition, RegistryError};
use crate::ipc::{PrivatePipeConnection, UserOriginProof};
use crate::root::RootLock;
use crate::process::ProcessCustodian;
use std::io::{BufRead, Write};
use std::path::Path;

type Result<T> = std::result::Result<T, OrchestrationError>;

fn user_payload_string(request: &V37Request, field: &'static str) -> Result<String> {
    match request.payload.get(&JsonString::from_str(field)) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or(OrchestrationError::Invalid(field)),
        _ => Err(OrchestrationError::Invalid(field)),
    }
}

/// Opaque native service state. No public raw database/issuer accessor, Clone,
/// deserialization, or caller-selected actor. Its RootLock must outlive it.
pub struct ProductDatabase<'root> {
    root: &'root RootLock,
    connection: VerifiedDatabaseConnection<'root>,
    owner: OwnerIssuer,
    process_custodian: ProcessCustodian,
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
        let process_custodian = ProcessCustodian::new()?;
        Ok(Self { root, connection, owner, process_custodian })
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
        let request = decode_request(frame).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("v37 user frame: {error:?}")))?;
        if request.family == "K-INSTANCE" && request.operation == "register" {
            return self.register_user_instance(&request);
        }
        Ok(encode_receipt(&request, V37Status::Unsupported,
            request.expected_revision, request.expected_revision, Default::default()))
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
        let observed = match instance::discover_program(&driver) {
            Ok(observed) => observed,
            Err(error) => {
                let status = match error {
                    CatalogError::UnknownDriver | CatalogError::UnsupportedVersion |
                    CatalogError::PackageIdentity | CatalogError::PackageFormat |
                    CatalogError::IdentityChanged => V37Status::Denied,
                    _ => V37Status::Failed,
                };
                return Ok(receipt(status, current, current,
                    Some(format!("native program observation: {error:?}"))));
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
            Ok(RegistrationDisposition::Applied) => (V37Status::Applied, None),
            Ok(RegistrationDisposition::Replayed) => (V37Status::Replayed, None),
            Err(error @ (RegistryError::RequestConflict | RegistryError::InstanceConflict)) =>
                (V37Status::Conflict, Some(format!("native instance register: {error:?}"))),
            Err(error) => (V37Status::Unknown,
                Some(format!("native instance register: {error:?}"))),
        };
        let next = if matches!(status, V37Status::Applied | V37Status::Replayed) { 1 }
            else { current };
        Ok(receipt(status, current, next, reason))
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
        let Self { root: _, connection, owner: _, process_custodian } = self;
        // Closing the Job first prevents a child from outliving the active
        // coordination database. Unresolved rows stay UNKNOWN on recovery.
        drop(process_custodian);
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
