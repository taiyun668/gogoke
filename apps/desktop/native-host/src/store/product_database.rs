//! Native composition owns the existing database and its private bootstrap issuer.
//! IPC remains unprivileged: the legacy typed dispatcher never receives OwnerIssuer.
use super::atomic::DomainRecordReceipt;
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
use super::session::{open_product_database, serve_authenticated_pipe, serve_lines, serve_pipe};
use crate::ipc::PrivatePipeConnection;
use crate::root::RootLock;
use crate::process::ProcessCustodian;
use std::io::{BufRead, Write};
use std::path::Path;

type Result<T> = std::result::Result<T, OrchestrationError>;

/// Opaque native service state. No public raw database/issuer accessor, Clone,
/// deserialization, or caller-selected actor. Its RootLock must outlive it.
pub struct ProductDatabase<'root> {
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
        Ok(Self { connection, owner, process_custodian })
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

    pub fn serve_lines<R: BufRead, W: Write>(&mut self, input: R, output: &mut W) -> Result<()> {
        serve_lines(&mut self.connection, input, output)
    }

    pub fn close_checked(self) -> std::result::Result<OpenLedger, SameOpenError> {
        let Self { connection, owner: _, process_custodian } = self;
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
