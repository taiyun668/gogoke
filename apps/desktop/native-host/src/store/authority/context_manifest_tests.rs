//! Native ContextManifest authority regressions. Definitions are not execution evidence.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::{delegate_owner_grant, issue_owner_grant, revise_owner_grant, revoke_owner_grant};
use super::context_manifest::{
    commit_context_manifest, initialize_context_manifest_schema, publish_context_assembly_snapshot,
    read_context_manifest, ContextAssemblySnapshot, ContextManifestCommitInput, ContextPartitionGrantBinding,
    ContextManifestReplayIdentity, ManifestExpectedVersion,
};
use super::context_read::{ContextReadRequest, GranteeContextReadRequest};
use super::model::{GrantRef, GrantSpec};
use super::task_context::{commit_task_context_requirements, CommitTaskContextRequirements, MandatoryContextRef};
use crate::root::RootLock;
use crate::store::action::{
    apply_action_schema, record_action_outcome, reserve_action, ActionReservation,
};
use crate::store::atomic::{
    apply_core_schema, commit_domain_record, DomainRecordInput, Statement,
};
use crate::store::context::{
    apply_context_schema, commit_context_version, ContextCommand,
};
use crate::store::digest::content_hash;
use crate::store::same_open::{
    create_new, route_b_test_guard, VerifiedDatabaseConnection,
};
use std::time::{SystemTime, UNIX_EPOCH};

const ACTION: &str = "opr_11111111111111111111111111111111";
const OPERATION: &str = "manifest-operation";
const MANIFEST_ID: &str = "manifest-one";
const PRINCIPAL: &str = "principal-one";
const SEAT: &str = "seat-one";
const WHEN: &str = "2026-09-21T00:00:00Z";

fn digest(ch: char) -> String {
    format!("sha256:{}", ch.to_string().repeat(64))
}

fn root_path() -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("gogoke-context-manifest-{}-{nonce}", std::process::id()))
}

fn fixture(run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let path = root_path();
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    apply_core_schema(&mut db).unwrap();
    apply_action_schema(&mut db).unwrap();
    apply_context_schema(&mut db).unwrap();
    let owner = initialize_profile(&mut db, &root).unwrap();
    initialize_context_manifest_schema(&mut db).unwrap();
    commit_task_context_requirements(&mut db, &CommitTaskContextRequirements {
        operation_id: "task-create".into(), domain_id: "domain-one".into(), task_id: "task-one".into(),
        expected_previous_revision: None, mandatory_refs: vec![], event_id: "task-event".into(),
        receipt_id: "task-receipt".into(), recorded_at: WHEN.into(),
    }).unwrap();
    run(&root, &mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(path) {
        eprintln!("owned fixture retained: {error}");
    }
}

fn grant_spec(principal: &str, seat: &str, depth: u8) -> GrantSpec {
    GrantSpec {
        principal_id: principal.into(),
        seat_id: seat.into(),
        permission: "context.read".into(),
        promotion_kind: "PROJECT_ONLY".into(),
        source_domain_id: "domain-source".into(),
        destination_domain_id: "domain-one".into(),
        destination_scope: "PROJECT".into(),
        delegable_depth: depth,
    }
}

fn grant(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer) -> GrantRef {
    let parent = issue_owner_grant(
        db, owner, "1", "0", grant_spec(owner.principal_id(), owner.seat_id(), 2),
    ).unwrap();
    delegate_owner_grant(db, owner, "1", &parent, grant_spec(PRINCIPAL, SEAT, 1)).unwrap()
}

fn direct_grant(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, spec: GrantSpec) -> GrantRef {
    issue_owner_grant(db, owner, "1", "0", spec).unwrap()
}

fn partition_binding(grant: GrantRef, source_domain_id: &str) -> ContextPartitionGrantBinding {
    ContextPartitionGrantBinding {
        source_domain_id: source_domain_id.into(), destination_scope: "PROJECT".into(),
        promotion_kind: "PROJECT_ONLY".into(), grant,
    }
}

fn source(db: &mut VerifiedDatabaseConnection<'_>, grant: &GrantRef) {
    source_with_grants(db, &[grant.clone()]);
}

fn source_with_grants(db: &mut VerifiedDatabaseConnection<'_>, grants: &[GrantRef]) {
    commit_context_version(db, ContextCommand {
        operation_id: "context-source-operation".into(),
        context_id: "context-one".into(),
        version: "1".into(),
        scope: "PROJECT".into(),
        domain_id: "domain-source".into(),
        kind: "fact".into(),
        content_hash: digest('a'),
        source_ref: "source://one".into(),
        source_hash: digest('b'),
        source_authority_kind: "repository".into(),
        source_authority_ref: "authority://one".into(),
        derived_from: vec![],
        supersedes: vec![],
        access_policy_revision: "1".into(),
        visibility: "DOMAIN_GRANTED".into(),
        read_grant_refs: grants.iter().map(|grant| grant.grant_id.clone()).collect(),
        promotion: None,
    }).unwrap();
}

fn action(db: &mut VerifiedDatabaseConnection<'_>) {
    reserve_action(db, ActionReservation {
        operation_id: ACTION.into(),
        semantic_digest: digest('c'),
        reservation_id: "action-reservation".into(),
        binding_id: "binding-one".into(),
        session_id: "session-one".into(),
        execution_id: "execution-one".into(),
        runtime_instance_id: "runtime-one".into(),
        profile_id: "profile-one".into(),
        auth_revision: "2".into(),
        generation: "7".into(),
        lane: "work".into(),
        action_kind: "queue".into(),
        payload_hex: "7b7d".into(),
        commitment: crate::store::action::test_commitment("session-one","execution-one","7"),
    }).unwrap();
}

fn decision(db: &mut VerifiedDatabaseConnection<'_>) {
    decision_with_task(db, "1");
}

fn decision_with_task(db: &mut VerifiedDatabaseConnection<'_>, task_revision: &str) {
    let bytes = format!(
        "{{\"actionId\":\"{ACTION}\",\"backend\":\"FAKE\",\"calibrationRef\":\"NONE\",\"candidateHash\":\"{}\",\"decisionId\":\"decision-one\",\"family\":\"CONTEXT_SELECTION\",\"mode\":\"fixture_bounded_auto\",\"modelRequested\":\"NONE\",\"modelResolved\":\"fake-v1\",\"nativeConfidence\":null,\"probabilities\":{{}},\"questionVersion\":\"1\",\"sourceRevisions\":{{\"bindingGeneration\":\"7\",\"capabilityRevision\":\"3\",\"policyRevision\":\"1\",\"taskRevision\":\"{task_revision}\"}},\"state\":\"COMMITTED\",\"stateViewHash\":\"{}\"}}",
        digest('d'), digest('e'),
    ).into_bytes();
    commit_domain_record(db, DomainRecordInput {
        domain_id: "domain-one".into(),
        object_type: "DecisionRecord".into(),
        object_id: "decision-one".into(),
        object_version: "1".into(),
        object_bytes: bytes,
        native_identity: None,
        event_id: "decision-event".into(),
        stream_id: "decision-stream".into(),
        expected_previous_counter: None,
        counter: "0".into(),
        event_type: "DecisionApplied".into(),
        occurred_at: WHEN.into(),
        event_bytes: b"{}".to_vec(),
        receipt_id: "decision-receipt".into(),
        operation_id: "decision-operation".into(),
        receipt_type: "DecisionApplied".into(),
        recorded_at: WHEN.into(),
        receipt_bytes: b"{}".to_vec(),
    }).unwrap();
}

fn snapshot(grant: GrantRef) -> ContextAssemblySnapshot {
    ContextAssemblySnapshot {
        operation_id: OPERATION.into(),
        principal_id: PRINCIPAL.into(),
        seat_id: SEAT.into(),
        task_id: "task-one".into(),
        session_id: "session-one".into(),
        domain_id: "domain-one".into(),
        binding_id: "binding-one".into(),
        binding_generation: "7".into(),
        source_epoch: "9".into(),
        runtime_instance_id: "runtime-one".into(),
        task_revision: "1".into(),
        policy_revision: "1".into(),
        auth_revision: "2".into(),
        revocation_head: "0".into(),
        selection_decision_id: "decision-one".into(),
        manifest_id: MANIFEST_ID.into(),
        admission_action_operation_id: ACTION.into(),
        admission_digest: digest('c'),
        max_content_bytes: 4096,
        max_candidates: 8,
        partition_grant_bindings: vec![partition_binding(grant, "domain-source")],
    }
}

fn snapshot_at(grant: GrantRef, task_revision: &str) -> ContextAssemblySnapshot {
    let mut value = snapshot(grant);
    value.task_revision = task_revision.into();
    value
}

fn read_request(grant: GrantRef) -> GranteeContextReadRequest {
    GranteeContextReadRequest {
        principal_id: PRINCIPAL.into(),
        seat_id: SEAT.into(),
        source: ContextReadRequest {
            source_domain_id: "domain-source".into(),
            context_id: "context-one".into(),
            version: "1".into(),
            expected_scope: "PROJECT".into(),
            expected_content_hash: digest('a'),
            expected_access_policy_revision: "1".into(),
            destination_domain_id: "domain-one".into(),
            destination_scope: "PROJECT".into(),
            promotion_kind: "PROJECT_ONLY".into(),
            policy_revision: "1".into(),
            grant,
        },
    }
}

fn expected() -> ManifestExpectedVersion {
    ManifestExpectedVersion {
        source_domain_id: "domain-source".into(),
        context_id: "context-one".into(),
        version: "1".into(),
        content_hash: digest('a'),
        state_revision: "1".into(),
        access_policy_revision: "1".into(),
    }
}

fn request_digest() -> String { digest('f') }

fn manifest_body(extra_included: &str) -> String {
    format!(
        "{{\"bindingGeneration\":\"7\",\"domainId\":\"domain-one\",\"includedVersions\":[{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"context-one\",{}\"reason\":\"AUTHORIZED_RETRIEVAL\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}],\"manifestId\":\"{MANIFEST_ID}\",\"policyRevision\":\"1\",\"redactions\":[],\"requiredConstraints\":[],\"seatId\":\"{SEAT}\",\"selectionDecisionId\":\"decision-one\",\"sourceSnapshot\":{{\"assemblySchema\":\"gogoke.context-assembly.v1\",\"authRevision\":\"2\",\"bindingId\":\"binding-one\",\"excluded\":[],\"mode\":\"FIXED_SOURCE_RULES\",\"operationId\":\"{OPERATION}\",\"partitions\":[{{\"sourceDomainId\":\"domain-source\"}}],\"principalId\":\"{PRINCIPAL}\",\"requestDigest\":\"{}\",\"revocationHead\":\"0\",\"runtimeInstanceId\":\"runtime-one\",\"sessionId\":\"session-one\",\"sourceEpoch\":\"9\",\"taskRevision\":\"1\"}},\"taskId\":\"task-one\"}}",
        digest('a'), extra_included, request_digest()
    )
}

fn manifest(extra_included: &str) -> Vec<u8> {
    let body = manifest_body(extra_included);
    let hash = content_hash(body.as_bytes());
    body.replacen(
        "\"manifestId\"",
        &format!("\"manifestHash\":\"{hash}\",\"manifestId\""),
        1,
    ).into_bytes()
}

fn mandatory_manifest(required: &str) -> Vec<u8> {
    mandatory_manifest_with_reason(required, "MANDATORY_CONSTRAINT")
}

fn mandatory_manifest_with_reason(required: &str, reason: &str) -> Vec<u8> {
    let body = format!(
        "{{\"bindingGeneration\":\"7\",\"domainId\":\"domain-one\",\"includedVersions\":[{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"context-one\",\"reason\":\"{reason}\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}],\"manifestId\":\"{MANIFEST_ID}\",\"policyRevision\":\"1\",\"redactions\":[],\"requiredConstraints\":{required},\"seatId\":\"{SEAT}\",\"selectionDecisionId\":\"decision-one\",\"sourceSnapshot\":{{\"assemblySchema\":\"gogoke.context-assembly.v1\",\"authRevision\":\"2\",\"bindingId\":\"binding-one\",\"excluded\":[],\"mode\":\"FIXED_SOURCE_RULES\",\"operationId\":\"{OPERATION}\",\"partitions\":[{{\"sourceDomainId\":\"domain-source\"}}],\"principalId\":\"{PRINCIPAL}\",\"requestDigest\":\"{}\",\"revocationHead\":\"0\",\"runtimeInstanceId\":\"runtime-one\",\"sessionId\":\"session-one\",\"sourceEpoch\":\"9\",\"taskRevision\":\"2\"}},\"taskId\":\"task-one\"}}",
        digest('a'), request_digest(),
    );
    let hash = content_hash(body.as_bytes());
    body.replacen("\"manifestId\"", &format!("\"manifestHash\":\"{hash}\",\"manifestId\""), 1).into_bytes()
}

fn set_mandatory_task(db: &mut VerifiedDatabaseConnection<'_>, operation: &str, previous: &str) {
    commit_task_context_requirements(db, &CommitTaskContextRequirements {
        operation_id: operation.into(), domain_id: "domain-one".into(), task_id: "task-one".into(),
        expected_previous_revision: Some(previous.into()),
        mandatory_refs: vec![MandatoryContextRef { source_domain_id: "domain-source".into(), context_id: "context-one".into(), version: "1".into() }],
        event_id: format!("event-{operation}"), receipt_id: format!("receipt-{operation}"), recorded_at: WHEN.into(),
    }).unwrap();
}

fn commit_input(grant: GrantRef, canonical_manifest: Vec<u8>) -> ContextManifestCommitInput {
    ContextManifestCommitInput {
        operation_id: OPERATION.into(),
        request_digest: request_digest(),
        event_id: "manifest-event".into(),
        receipt_id: "manifest-receipt".into(),
        recorded_at: WHEN.into(),
        read_requests: vec![read_request(grant)],
        expected_versions: vec![expected()],
        canonical_manifest,
    }
}

fn seed(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer) -> GrantRef {
    let grant = grant(db, owner);
    source(db, &grant);
    action(db);
    decision(db);
    publish_context_assembly_snapshot(db, &snapshot(grant.clone())).unwrap();
    grant
}

fn count_rows(db: &mut VerifiedDatabaseConnection<'_>, table: &str) -> String {
    let query = format!("SELECT count(*) FROM {table}");
    let statement = Statement::prepare(db.as_ptr(), &query).unwrap();
    assert!(statement.step_row().unwrap());
    statement.column_text(0).unwrap()
}

#[test]
fn manifest_commit_and_replay_reauthorize_and_return_same_canonical_bytes() {
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        let input = commit_input(grant, manifest(""));
        let first = commit_context_manifest(db, &input).unwrap();
        assert_eq!(first.disposition, "COMMITTED");
        assert_eq!(first.canonical_manifest, input.canonical_manifest);
        let replay = commit_context_manifest(db, &input).unwrap();
        assert_eq!(replay.disposition, "REPLAYED");
        assert_eq!(replay.manifest_hash, first.manifest_hash);
        let read = read_context_manifest(db, &ContextManifestReplayIdentity {
            operation_id: OPERATION.into(), principal_id: PRINCIPAL.into(), seat_id: SEAT.into(),
            task_id: "task-one".into(), session_id: "session-one".into(), domain_id: "domain-one".into(),
            binding_id: "binding-one".into(), binding_generation: "7".into(), source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        }).unwrap();
        assert_eq!(read.canonical_manifest, first.canonical_manifest);
    });
}

#[test]
fn mandatory_constraints_are_exactly_the_current_task_set_and_task_revision_change_invalidates_replay() {
    fixture(|_, db, owner| {
        set_mandatory_task(db, "task-mandatory", "1");
        let grant = grant(db, owner);
        source(db, &grant);
        action(db);
        decision_with_task(db, "2");
        publish_context_assembly_snapshot(db, &snapshot_at(grant.clone(), "2")).unwrap();

        let exact = format!(
            "[{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"context-one\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}]",
            digest('a'),
        );
        assert!(commit_context_manifest(db, &commit_input(grant.clone(), mandatory_manifest_with_reason("[]", "AUTHORIZED_RETRIEVAL"))).is_err());
        let extra = format!(
            "{},{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"context-extra\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}]",
            exact.trim_end_matches(']'), digest('a'),
        );
        assert!(commit_context_manifest(db, &commit_input(grant.clone(), mandatory_manifest(&extra))).is_err());

        let input = commit_input(grant, mandatory_manifest(&exact));
        assert_eq!(commit_context_manifest(db, &input).unwrap().disposition, "COMMITTED");
        set_mandatory_task(db, "task-revise-after-manifest", "2");
        assert!(commit_context_manifest(db, &input).is_err());
        assert!(read_context_manifest(db, &ContextManifestReplayIdentity {
            operation_id: OPERATION.into(), principal_id: PRINCIPAL.into(), seat_id: SEAT.into(),
            task_id: "task-one".into(), session_id: "session-one".into(), domain_id: "domain-one".into(),
            binding_id: "binding-one".into(), binding_generation: "7".into(), source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        }).is_err());
    });
}

#[test]
fn snapshot_binding_rejects_a_different_current_acl_authorized_grant() {
    fixture(|_, db, owner| {
        let bound = grant(db, owner);
        let substitute = direct_grant(db, owner, grant_spec(PRINCIPAL, SEAT, 0));
        source_with_grants(db, &[bound.clone(), substitute.clone()]);
        action(db);
        decision(db);
        publish_context_assembly_snapshot(db, &snapshot(bound)).unwrap();
        assert!(commit_context_manifest(db, &commit_input(substitute, manifest(""))).is_err());
        assert_eq!(count_rows(db, "gogoke_context_manifest_read_bindings"), "0");
        assert_eq!(count_rows(db, "gogoke_receipts"), "2");
    });
}

#[test]
fn snapshot_publish_requires_the_named_current_task() {
    fixture(|_, db, owner| {
        let grant = grant(db, owner);
        action(db);
        db.execute("DELETE FROM gogoke_task_context_heads WHERE domain_id='domain-one' AND task_id='task-one'").unwrap();
        assert!(publish_context_assembly_snapshot(db, &snapshot(grant)).is_err());
        assert_eq!(count_rows(db, "gogoke_context_assembly_snapshots"), "0");
    });
}

#[test]
fn same_source_domain_may_bind_distinct_grants_but_duplicate_tuple_is_rejected() {
    fixture(|_, db, owner| {
        let first = grant(db, owner);
        let second = direct_grant(db, owner, grant_spec(PRINCIPAL, SEAT, 0));
        action(db);
        let mut valid = snapshot(first.clone());
        valid.partition_grant_bindings.push(partition_binding(second, "domain-source"));
        publish_context_assembly_snapshot(db, &valid).unwrap();

        let mut exact_retry = valid.clone();
        exact_retry.partition_grant_bindings.reverse();
        publish_context_assembly_snapshot(db, &exact_retry).unwrap();

        let changed_retry = snapshot(first.clone());
        assert!(matches!(
            publish_context_assembly_snapshot(db, &changed_retry),
            Err(crate::store::orchestration::OrchestrationError::OperationConflict),
        ));

        let mut duplicate = valid;
        duplicate.partition_grant_bindings.push(partition_binding(first, "domain-source"));
        assert!(publish_context_assembly_snapshot(db, &duplicate).is_err());
        assert_eq!(count_rows(db, "gogoke_context_assembly_partition_grant_bindings"), "2");
    });
}

#[test]
fn a_bound_partition_without_candidates_is_valid() {
    fixture(|_, db, owner| {
        let selected = grant(db, owner);
        let mut empty_spec = grant_spec(PRINCIPAL, SEAT, 0);
        empty_spec.source_domain_id = "domain-empty".into();
        let empty = direct_grant(db, owner, empty_spec);
        source(db, &selected);
        action(db);
        decision(db);
        let mut value = snapshot(selected.clone());
        value.partition_grant_bindings.push(partition_binding(empty, "domain-empty"));
        publish_context_assembly_snapshot(db, &value).unwrap();
        assert_eq!(commit_context_manifest(db, &commit_input(selected, manifest(""))).unwrap().disposition, "COMMITTED");
    });
}

#[test]
fn publish_rejects_every_wrong_current_grant_axis_without_partial_snapshot() {
    fixture(|_, db, owner| {
        action(db);
        let mut specs = Vec::new();
        let mut spec = grant_spec("principal-other", SEAT, 0); specs.push(spec.clone());
        spec = grant_spec(PRINCIPAL, "seat-other", 0); specs.push(spec.clone());
        spec = grant_spec(PRINCIPAL, SEAT, 0); spec.permission = "context.promote.source".into(); specs.push(spec.clone());
        spec = grant_spec(PRINCIPAL, SEAT, 0); spec.source_domain_id = "domain-other".into(); specs.push(spec.clone());
        spec = grant_spec(PRINCIPAL, SEAT, 0); spec.destination_domain_id = "domain-other".into(); specs.push(spec.clone());
        spec = grant_spec(PRINCIPAL, SEAT, 0); spec.destination_scope = "GLOBAL".into(); specs.push(spec.clone());
        spec = grant_spec(PRINCIPAL, SEAT, 0); spec.promotion_kind = "GLOBAL_LESSON".into(); specs.push(spec);
        for wrong in specs {
            let reference = direct_grant(db, owner, wrong);
            assert!(publish_context_assembly_snapshot(db, &snapshot(reference)).is_err());
        }
        assert_eq!(count_rows(db, "gogoke_context_assembly_snapshots"), "0");
        assert_eq!(count_rows(db, "gogoke_context_assembly_partition_grant_bindings"), "0");
    });
}

#[test]
fn grant_revocation_after_snapshot_blocks_first_commit_and_replay_disclosure() {
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        let input = commit_input(grant.clone(), manifest(""));
        let _new_head = revoke_owner_grant(db, owner, "1", &grant).unwrap();
        assert!(commit_context_manifest(db, &input).is_err());
    });
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        let input = commit_input(grant.clone(), manifest(""));
        commit_context_manifest(db, &input).unwrap();
        let _new_head = revoke_owner_grant(db, owner, "1", &grant).unwrap();
        assert!(read_context_manifest(db, &ContextManifestReplayIdentity {
            operation_id: OPERATION.into(), principal_id: PRINCIPAL.into(), seat_id: SEAT.into(),
            task_id: "task-one".into(), session_id: "session-one".into(), domain_id: "domain-one".into(),
            binding_id: "binding-one".into(), binding_generation: "7".into(), source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        }).is_err());
    });
}

#[test]
fn grant_revision_after_snapshot_blocks_first_commit_and_replay_disclosure() {
    fixture(|_, db, owner| {
        let spec = grant_spec(PRINCIPAL, SEAT, 0);
        let reference = direct_grant(db, owner, spec.clone());
        source(db, &reference);
        action(db);
        decision(db);
        publish_context_assembly_snapshot(db, &snapshot(reference.clone())).unwrap();
        revise_owner_grant(db, owner, "1", &reference, spec).unwrap();
        assert!(commit_context_manifest(db, &commit_input(reference, manifest(""))).is_err());
    });
    fixture(|_, db, owner| {
        let spec = grant_spec(PRINCIPAL, SEAT, 0);
        let reference = direct_grant(db, owner, spec.clone());
        source(db, &reference);
        action(db);
        decision(db);
        publish_context_assembly_snapshot(db, &snapshot(reference.clone())).unwrap();
        let input = commit_input(reference.clone(), manifest(""));
        commit_context_manifest(db, &input).unwrap();
        revise_owner_grant(db, owner, "1", &reference, spec).unwrap();
        assert!(read_context_manifest(db, &ContextManifestReplayIdentity {
            operation_id: OPERATION.into(), principal_id: PRINCIPAL.into(), seat_id: SEAT.into(),
            task_id: "task-one".into(), session_id: "session-one".into(), domain_id: "domain-one".into(),
            binding_id: "binding-one".into(), binding_generation: "7".into(), source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        }).is_err());
    });
}

#[test]
fn a_snapshot_without_persisted_partition_rows_fails_closed() {
    fixture(|_, db, owner| {
        let reference = seed(db, owner);
        let input = commit_input(reference, manifest(""));
        commit_context_manifest(db, &input).unwrap();
        db.execute("DELETE FROM gogoke_context_assembly_partition_grant_bindings WHERE operation_id='manifest-operation'").unwrap();
        assert!(commit_context_manifest(db, &input).is_err());
        assert!(read_context_manifest(db, &ContextManifestReplayIdentity {
            operation_id: OPERATION.into(), principal_id: PRINCIPAL.into(), seat_id: SEAT.into(),
            task_id: "task-one".into(), session_id: "session-one".into(), domain_id: "domain-one".into(),
            binding_id: "binding-one".into(), binding_generation: "7".into(), source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        }).is_err());
    });
}

#[test]
fn stale_context_state_revision_or_nonactive_source_blocks_manifest_commit() {
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        let replacement = ContextCommand {
            operation_id: "context-replace".into(), context_id: "context-one".into(), version: "2".into(),
            scope: "PROJECT".into(), domain_id: "domain-source".into(), kind: "fact".into(),
            content_hash: digest('8'), source_ref: "source://two".into(), source_hash: digest('9'),
            source_authority_kind: "repository".into(), source_authority_ref: "authority://one".into(),
            derived_from: vec![], supersedes: vec!["context-one@1".into()], access_policy_revision: "1".into(),
            visibility: "DOMAIN_GRANTED".into(), read_grant_refs: vec![grant.grant_id.clone()], promotion: None,
        };
        commit_context_version(db, replacement.clone()).unwrap();
        assert!(commit_context_manifest(db, &commit_input(grant, manifest(""))).is_err());
    });
}

#[test]
fn first_manifest_commit_requires_the_same_action_to_still_be_reserved() {
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        record_action_outcome(
            db, "action-reservation", ACTION, &digest('c'), "dispatched", "", "native-receipt",
        ).unwrap();
        assert!(commit_context_manifest(db, &commit_input(grant, manifest(""))).is_err());
    });
}

#[test]
fn partial_manifest_authority_schema_is_corruption_not_an_auto_repair() {
    fixture(|_, db, _owner| {
        db.execute("DROP TABLE gogoke_context_manifest_read_bindings").unwrap();
        let value = snapshot(GrantRef { grant_id: "grant-missing".into(), revision: "1".into(), revocation_head: "0".into() });
        assert!(publish_context_assembly_snapshot(db, &value).is_err());
        let statement = Statement::prepare(
            db.as_ptr(),
            "SELECT count(*) FROM sqlite_schema WHERE name='gogoke_context_manifest_read_bindings'",
        ).unwrap();
        assert!(statement.step_row().unwrap());
        assert_eq!(statement.column_text(0).unwrap(), "0");
    });
}

#[test]
fn missing_or_unknown_partition_schema_and_partition_triggers_fail_closed() {
    fixture(|_, db, _owner| {
        db.execute("DROP TABLE gogoke_context_assembly_partition_grant_bindings").unwrap();
        let value = snapshot(GrantRef { grant_id: "grant-missing".into(), revision: "1".into(), revocation_head: "0".into() });
        assert!(publish_context_assembly_snapshot(db, &value).is_err());
        assert_eq!(count_rows(db, "gogoke_context_assembly_snapshots"), "0");
    });
    fixture(|_, db, _owner| {
        db.execute("DROP TABLE gogoke_context_assembly_partition_grant_bindings").unwrap();
        db.execute("CREATE TABLE gogoke_context_assembly_partition_grant_bindings (operation_id TEXT PRIMARY KEY) STRICT").unwrap();
        let value = snapshot(GrantRef { grant_id: "grant-missing".into(), revision: "1".into(), revocation_head: "0".into() });
        assert!(publish_context_assembly_snapshot(db, &value).is_err());
        assert_eq!(count_rows(db, "gogoke_context_assembly_snapshots"), "0");
    });
    fixture(|_, db, owner| {
        let reference = grant(db, owner);
        action(db);
        db.execute("CREATE TRIGGER forged_partition_binding AFTER INSERT ON gogoke_context_assembly_partition_grant_bindings BEGIN SELECT 1; END").unwrap();
        assert!(publish_context_assembly_snapshot(db, &snapshot(reference)).is_err());
        assert_eq!(count_rows(db, "gogoke_context_assembly_snapshots"), "0");
        assert_eq!(count_rows(db, "gogoke_context_assembly_partition_grant_bindings"), "0");
    });
}

#[test]
fn wrong_manifest_hash_and_extra_nested_fields_fail_closed_even_when_rehashed() {
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        let mut wrong = manifest("");
        let last = wrong.len() - 1;
        wrong[last] = b' ';
        assert!(commit_context_manifest(db, &commit_input(grant.clone(), wrong)).is_err());

        let forged = manifest("\"grantRef\":\"grant-forged\",");
        assert!(commit_context_manifest(db, &commit_input(grant, forged)).is_err());
    });
}

#[test]
fn replay_identity_and_original_receipt_coordinates_are_immutable() {
    fixture(|_, db, owner| {
        let grant = seed(db, owner);
        let input = commit_input(grant.clone(), manifest(""));
        commit_context_manifest(db, &input).unwrap();

        let mut changed = commit_input(grant, input.canonical_manifest.clone());
        changed.receipt_id = "manifest-receipt-other".into();
        assert!(commit_context_manifest(db, &changed).is_err());

        let mut identity = ContextManifestReplayIdentity {
            operation_id: OPERATION.into(), principal_id: PRINCIPAL.into(), seat_id: SEAT.into(),
            task_id: "task-one".into(), session_id: "session-one".into(), domain_id: "domain-one".into(),
            binding_id: "binding-one".into(), binding_generation: "7".into(), source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        };
        identity.binding_generation = "8".into();
        assert!(read_context_manifest(db, &identity).is_err());
    });
}
