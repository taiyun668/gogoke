//! Native Product Authority, in the product's existing Route-B SQLite domain.
//! No model/adapter/Node caller can construct an OwnerIssuer or issue grants.
//! Grant validation is transaction-scoped, not a second Context-side authority.
mod bootstrap;
mod catalog;
mod delegation;
mod model;
mod promotion;
mod promotion_commit;
mod transaction;
mod process_custody;

pub(crate) use process_custody::{
    initialize as initialize_process_custody_schema,
    mark_active as mark_process_active,
    mark_stopped as mark_process_stopped,
    mark_unknown as mark_process_unknown,
    record_prepared as record_prepared_process,
};

pub(crate) use bootstrap::{
    admit_owner_controller_caller, initialize_profile, read_product_identity,
    ProductIdentitySnapshot,
};

#[cfg(test)]
mod promotion_commit_tests;
#[cfg(test)]
mod root_identity_tests;
#[cfg(test)]
mod tests;

mod context_read;
#[cfg(test)]
mod context_read_tests;

#[cfg(test)]
mod context_grantee_tests;
mod context_read_set;
#[cfg(test)]
mod context_read_set_tests;

mod context_manifest;
pub(crate) use context_manifest::{
    commit_context_manifest, initialize_context_manifest_schema, publish_context_assembly_snapshot,
    read_context_manifest, ContextAssemblySnapshot, ContextManifestAuthorityReceipt,
    ContextManifestCommitInput, ContextManifestReplayIdentity, ContextPartitionGrantBinding,
    ManifestExpectedVersion,
};
#[cfg(test)]
mod context_manifest_tests;

mod context_assembly;
pub(crate) use context_assembly::{
    list_context_assembly_sources, read_context_assembly_basis, ContextAssemblyBasis,
    ContextAssemblySource,
};
#[cfg(test)]
mod context_assembly_tests;

mod task_context;
pub(crate) use task_context::{
    commit_task_context_requirements, initialize_task_context_schema,
    read_task_context_requirements, CommitTaskContextRequirements, MandatoryContextRef,
    TaskContextRequirements, TaskContextRequirementsReceipt,
};
#[cfg(test)]
mod task_context_tests;

mod material;
pub(crate) use material::{
    append_trusted_task_material, read_trusted_task_material, AppendTaskMaterial,
    TaskMaterialReceipt, TaskMaterialVersion,
};

mod task_package;
pub(crate) use task_package::{
    initialize_authorized_task_package_schema, prepare_authorized_task_package,
    read_authorized_task_package, AuthorizedTaskPackage, AuthorizedTaskPackageDraft,
    AuthorizedTaskPackageReceipt, PrepareAuthorizedTaskPackage, SelectedMaterial,
    TaskMaterialReference, TaskPackageBinding, TaskPackagePrincipal,
};
#[cfg(test)]
mod task_package_tests;

mod session_lineage;
pub(crate) use session_lineage::{
    apply_session_lineage_command, initialize_session_lineage_schema, read_exposure_receipt,
    read_session_lineage, ExposureAssessment, ExposureReceipt, InheritedExposureSummary,
    MaterialHandoff, NativeSessionIdentity, NativeSourceCoverage, PendingActionRef, SessionLineage,
    SessionLineageCommand, SessionLineageOperation, SessionLineageReceipt, SessionSnapshot,
    SourceObservation, StoredExposureReceipt,
};
#[cfg(test)]
mod session_lineage_tests;

mod execution_recipe;
pub(crate) use execution_recipe::{
    append_owner_execution_recipe, initialize_execution_recipe_schema,
    read_current_execution_recipe, read_execution_recipe_revision, AppendExecutionRecipe,
    ExecutionRecipe, ExecutionRecipeReceipt, ExecutionRecipeVersion, RecipeJsonObject,
    RecipeJsonString, RecipeJsonValue, CURRENTNESS_STATUS,
};

pub(crate) use bootstrap::OwnerIssuer;
pub(crate) use catalog::{
    delegate_owner_grant, issue_owner_grant, revise_owner_grant, revoke_owner_grant,
};
pub(crate) use context_read::{
    read_grantee_context, read_owner_context, AuthorizedContextReadSnapshot, ContextReadRequest,
    ContextReadSnapshot, GranteeContextReadRequest,
};
pub(crate) use context_read_set::{
    read_grantee_context_set, read_owner_context_set, AuthorizedContextReadSet, ContextReadSet,
};
pub(crate) use delegation::{
    delegate_owner_delegation, issue_owner_delegation, issue_r2_test_owner_delegation_once,
    r2_test_grant_id, read_current_delegation,
    revise_owner_delegation, revoke_owner_delegation, AuthorityCeiling, DelegationBinding,
    DelegationGrantIdentity, DelegationGrantInput, DelegationGrantSnapshot, DelegationPrincipal,
};
pub(crate) use model::{GrantRef, GrantSpec};
pub(crate) use promotion::PromotionRequest;
pub(crate) use promotion_commit::commit_owner_promotion;

#[cfg(test)]
mod action_transaction_tests;

#[cfg(test)]
mod delegation_tests;

#[cfg(test)]
mod record_transaction_tests;

mod outcome;
pub(crate) use outcome::{append_owner_override_outcome, OutcomeVersionRef, OwnerOutcomeAppend};
mod objective_outcome;
pub(crate) use objective_outcome::{
    append_objective_outcome, read_objective_outcome, AppendObjectiveOutcome,
    ObjectiveEvidenceRef, ObjectiveObservationWindow, ObjectiveOutcomeVersion,
    ObjectiveVersionRef,
};
#[cfg(test)]
#[path = "objective_outcome_tests.rs"]
mod objective_outcome_tests;
mod evaluation;
pub(crate) use evaluation::{
    append_evaluation, read_evaluation, AppendEvaluation, EvaluationEvidenceRef,
    EvaluationOutcomeRef, EvaluationReceipt, EvaluationVersionRef,
};
mod dream;
pub(crate) use dream::{
    append_dream_proposal, append_dream_run, read_dream_proposal, read_dream_run,
    AppendDreamProposal, AppendDreamRun, DreamAllowedChange, DreamBudgetLease,
    DreamEvaluationRef, DreamObjectRef, DreamReceipt, DreamVersionRef,
};
#[cfg(test)]
#[path = "dream_tests.rs"]
mod dream_tests;
#[cfg(test)]
#[path = "evaluation_tests.rs"]
mod evaluation_tests;
#[cfg(test)]
mod outcome_source_version_tests;
#[cfg(test)]
mod outcome_tests;

mod decision_replay;
pub(crate) use decision_replay::{
    read_durable_decision_replay, DurableDecisionRecord, DurableDecisionReplay,
};
#[cfg(test)]
mod decision_replay_tests;

mod decision_capacity;
pub(crate) use decision_capacity::{
    initialize_decision_capacity_schema, publish_decision_snapshot, DecisionAuthoritySnapshot,
    DecisionCapacityRequest,
};
#[cfg(test)]
mod decision_capacity_tests;

mod decision_commit;
pub(crate) use decision_commit::{
    commit_decision, durable_record_json, DecisionCommitDisposition, DecisionCommitInput,
    DecisionCommitReceipt,
};
#[cfg(test)]
mod decision_commit_tests;

mod action_authority;
pub(crate) use action_authority::{
    begin_committed_action, derive_native_action_current_facts, prepare_action_authority,
    read_native_action_fixture_selection, record_trusted_native_action_receipt,
    complete_action_from_native_receipt, ActionCompletionDisposition, BeginCommittedAction,
    BeginCommittedDisposition, NativeActionCurrentFactsRefs, NativeActionFixtureSelection,
    TrustedActionCompletionEvidence,
    PrepareActionAuthority, PreparedActionAuthority,
};
