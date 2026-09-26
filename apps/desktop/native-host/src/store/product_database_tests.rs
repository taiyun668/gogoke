//! Controlled Windows/native tests for opaque service composition, not public actor admission.
use super::*;
use crate::store::action::{
    self, ActionReservation, BeginDisposition, ReserveDisposition,
};
use crate::store::atomic::Statement;
use crate::store::context::{commit_context_version, PromotionEvidence};
use crate::store::digest::content_hash;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::io::Cursor;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn failed_production_prepared_record_does_not_activate_child() {
    use crate::process::{NativeBinding, PrepareRequest, ProcessLaunch};
    fixture(|_, product| {
        let command = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
            .join("System32").join("cmd.exe");
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let marker = std::env::temp_dir().join(format!(
            "gogoke-prepared-record-failure-{}-{nonce}.txt", std::process::id()
        ));
        assert!(!marker.exists(), "fresh controlled marker");
        let mut launch = ProcessLaunch::new(&command);
        launch.arguments = vec![
            "/D".into(), "/C".into(),
            format!("echo started>\"{}\"", marker.display()),
        ];
        let request = PrepareRequest {
            binding: NativeBinding {
                binary_digest_sha256: content_hash(&std::fs::read(&command).unwrap()),
                profile_id: "profile-test".into(),
                domain_id: "domain-r2-02-test".into(),
                generation: "1".into(),
            },
            launch,
        };
        let first = super::session::prepare_recorded_process(
            &mut product.connection, &mut product.process_custodian,
            "same-operation", &request,
        ).expect("first prepared row committed");
        assert!(super::session::prepare_recorded_process(
            &mut product.connection, &mut product.process_custodian,
            "same-operation", &request,
        ).is_err(), "second coordination write must fail on the unique operation");
        assert_eq!(scalar(product,
            "SELECT count(*) FROM gogoke_coordination_process_custody WHERE operation_id='same-operation' AND state='PREPARED'"), "1");
        assert!(product.process_custodian.active(&first.ticket).is_none(),
            "the committed first child remains suspended");
        std::thread::sleep(std::time::Duration::from_secs(1));
        assert!(!marker.exists(), "failed write cannot resume a child or run its marker");
        product.process_custodian.abort_prepared(&first).unwrap();
        authority::mark_process_unknown(&mut product.connection, "same-operation", &first).unwrap();
    });
}

fn fixture(run: impl FnOnce(&RootLock, &mut ProductDatabase<'_>)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-product-compose-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut product = ProductDatabase::open(&root, &database).unwrap();
    run(&root, &mut product);
    product.close_checked().unwrap(); drop(root);
    std::fs::remove_file(database).unwrap();
    std::fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {error}"); }
}
fn scalar(product: &ProductDatabase<'_>, sql: &str) -> String {
    let statement = Statement::prepare(product.connection.as_ptr(), sql).unwrap();
    assert!(statement.step_row().unwrap()); let result = statement.column_text(0).unwrap();
    assert!(!statement.step_row().unwrap()); result
}
fn spec(product: &ProductDatabase<'_>, permission: &str, depth: u8) -> GrantSpec {
    GrantSpec { principal_id: product.owner.principal_id().into(), seat_id: product.owner.seat_id().into(),
        permission: permission.into(), promotion_kind: "GLOBAL_LESSON".into(),
        source_domain_id: "domain-one".into(), destination_domain_id: "domain-one".into(),
        destination_scope: "GLOBAL".into(), delegable_depth: depth }
}
fn context() -> ContextCommand {
    ContextCommand { operation_id: "source-operation".into(), context_id: "source-one".into(),
        version: "1".into(), scope: "PROJECT".into(), domain_id: "domain-one".into(), kind: "fact".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)), source_ref: "source://one".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(),
        source_authority_ref: "authority://one".into(), derived_from: vec![], supersedes: vec![],
        access_policy_revision: "1".into(), visibility: "OWNER_PRIVATE".into(), read_grant_refs: vec![], promotion: None }
}
fn read_request(grant: GrantRef) -> ContextReadRequest {
    ContextReadRequest { source_domain_id: "domain-one".into(), context_id: "source-one".into(), version: "1".into(),
        expected_scope: "PROJECT".into(), expected_content_hash: format!("sha256:{}", "a".repeat(64)),
        expected_access_policy_revision: "1".into(), destination_domain_id: "domain-one".into(),
        destination_scope: "GLOBAL".into(), promotion_kind: "GLOBAL_LESSON".into(), policy_revision: "1".into(), grant }
}

const VERTICAL_ADMISSION_ACTION: &str = "opr_11111111111111111111111111111111";
const VERTICAL_EXECUTION_ACTION: &str = "opr_22222222222222222222222222222222";
const VERTICAL_WHEN: &str = "2026-09-21T12:00:00Z";

fn hash(character: char) -> String {
    format!("sha256:{}", character.to_string().repeat(64))
}

fn vertical_action(operation_id: &str, reservation_id: &str, semantic: char) -> ActionReservation {
    ActionReservation {
        operation_id: operation_id.into(),
        semantic_digest: hash(semantic),
        reservation_id: reservation_id.into(),
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
        commitment: action::test_commitment("session-one", "execution-one", "7"),
    }
}

fn vertical_grant(product: &mut ProductDatabase<'_>) -> GrantRef {
    let parent = product.issue_grant("1", "0", GrantSpec {
        principal_id: product.owner.principal_id().into(),
        seat_id: product.owner.seat_id().into(),
        permission: "context.read".into(),
        promotion_kind: "PROJECT_ONLY".into(),
        source_domain_id: "domain-source".into(),
        destination_domain_id: "domain-destination".into(),
        destination_scope: "PROJECT".into(),
        delegable_depth: 1,
    }).unwrap();
    product.delegate_grant("1", &parent, GrantSpec {
        principal_id: "principal-worker".into(),
        seat_id: "seat-worker".into(),
        permission: "context.read".into(),
        promotion_kind: "PROJECT_ONLY".into(),
        source_domain_id: "domain-source".into(),
        destination_domain_id: "domain-destination".into(),
        destination_scope: "PROJECT".into(),
        delegable_depth: 0,
    }).unwrap()
}

fn vertical_context(grant: &GrantRef) -> ContextCommand {
    ContextCommand {
        operation_id: "vertical-context-operation".into(),
        context_id: "context-one".into(),
        version: "1".into(),
        scope: "PROJECT".into(),
        domain_id: "domain-source".into(),
        kind: "fact".into(),
        content_hash: hash('a'),
        source_ref: "source://ordinary-fixture".into(),
        source_hash: hash('b'),
        source_authority_kind: "repository".into(),
        source_authority_ref: "authority://fixture".into(),
        derived_from: vec![],
        supersedes: vec![],
        access_policy_revision: "1".into(),
        visibility: "DOMAIN_GRANTED".into(),
        read_grant_refs: vec![grant.grant_id.clone()],
        promotion: None,
    }
}

fn vertical_grantee_read(grant: GrantRef) -> GranteeContextReadRequest {
    GranteeContextReadRequest {
        principal_id: "principal-worker".into(),
        seat_id: "seat-worker".into(),
        source: ContextReadRequest {
            source_domain_id: "domain-source".into(),
            context_id: "context-one".into(),
            version: "1".into(),
            expected_scope: "PROJECT".into(),
            expected_content_hash: hash('a'),
            expected_access_policy_revision: "1".into(),
            destination_domain_id: "domain-destination".into(),
            destination_scope: "PROJECT".into(),
            promotion_kind: "PROJECT_ONLY".into(),
            policy_revision: "1".into(),
            grant,
        },
    }
}

fn vertical_decision_record(
    operation_id: &str,
    scenario_id: &str,
    family: &str,
    choice: &str,
) -> authority::DurableDecisionRecord {
    authority::DurableDecisionRecord {
        operation_id: operation_id.into(),
        scenario_id: scenario_id.into(),
        family: family.into(),
        state_view_hash: hash('e'),
        candidate_hash: hash('f'),
        question_version: "1".into(),
        rubric_version: "1".into(),
        model_requested: None,
        model_resolved: Some("fake-v1".into()),
        task_revision: "1".into(),
        policy_revision: "1".into(),
        capability_revision: "3".into(),
        binding_generation: "7".into(),
        backend_kind: "FAKE".into(),
        choice: choice.into(),
        reason: "QUALIFIED_BOUNDED_SELECTION".into(),
        budget_units: 1,
        deadline_epoch_ms: 1000,
    }
}

fn commit_vertical_decision(
    product: &mut ProductDatabase<'_>,
    operation_id: &str,
    decision_id: &str,
    scenario_id: &str,
    family: &str,
    choice: &str,
    action: &ActionReservation,
    resource: &str,
) -> authority::DecisionCommitReceipt {
    let record = vertical_decision_record(operation_id, scenario_id, family, choice);
    product.publish_decision_snapshot(&DecisionAuthoritySnapshot {
        operation_id: record.operation_id.clone(),
        candidate_id: record.choice.clone(),
        state_view_hash: record.state_view_hash.clone(),
        candidate_hash: record.candidate_hash.clone(),
        task_revision: record.task_revision.clone(),
        policy_revision: record.policy_revision.clone(),
        capability_revision: record.capability_revision.clone(),
        binding_id: action.binding_id.clone(),
        binding_generation: action.generation.clone(),
        auth_revision: action.auth_revision.clone(),
        resource_ref: resource.into(),
        resource_revision: "1".into(),
        capacity_total: 2,
        action_operation_id: action.operation_id.clone(),
        action_digest: action.semantic_digest.clone(),
    }).unwrap();
    product.commit_decision(&DecisionCommitInput {
        domain_id: "domain-destination".into(),
        decision_id: decision_id.into(),
        event_id: format!("{decision_id}-event"),
        receipt_id: format!("{decision_id}-receipt"),
        recorded_at: VERTICAL_WHEN.into(),
        record,
        resource_reservation_ref: format!("{resource}-lease"),
        action_intent_ref: action.operation_id.clone(),
        required_capacity_units: 1,
    }).unwrap()
}

fn vertical_assembly_snapshot(grant: GrantRef, action: &ActionReservation) -> ContextAssemblySnapshot {
    ContextAssemblySnapshot {
        operation_id: "assembly-operation".into(),
        principal_id: "principal-worker".into(),
        seat_id: "seat-worker".into(),
        task_id: "task-one".into(),
        session_id: action.session_id.clone(),
        domain_id: "domain-destination".into(),
        binding_id: action.binding_id.clone(),
        binding_generation: action.generation.clone(),
        source_epoch: "9".into(),
        runtime_instance_id: action.runtime_instance_id.clone(),
        task_revision: "1".into(),
        policy_revision: "1".into(),
        auth_revision: action.auth_revision.clone(),
        revocation_head: "0".into(),
        selection_decision_id: "context-decision".into(),
        manifest_id: "manifest-one".into(),
        admission_action_operation_id: action.operation_id.clone(),
        admission_digest: action.semantic_digest.clone(),
        max_content_bytes: 4096,
        max_candidates: 4,
        partition_grant_bindings: vec![authority::ContextPartitionGrantBinding {
            source_domain_id: "domain-source".into(),
            destination_scope: "PROJECT".into(),
            promotion_kind: "PROJECT_ONLY".into(),
            grant,
        }],
    }
}

fn vertical_manifest(request_digest: &str) -> Vec<u8> {
    let body = format!(
        "{{\"bindingGeneration\":\"7\",\"domainId\":\"domain-destination\",\"includedVersions\":[{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"context-one\",\"reason\":\"MANDATORY_CONSTRAINT\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}],\"manifestId\":\"manifest-one\",\"policyRevision\":\"1\",\"redactions\":[],\"requiredConstraints\":[{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"context-one\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}],\"seatId\":\"seat-worker\",\"selectionDecisionId\":\"context-decision\",\"sourceSnapshot\":{{\"assemblySchema\":\"gogoke.context-assembly.v1\",\"authRevision\":\"2\",\"bindingId\":\"binding-one\",\"excluded\":[],\"mode\":\"FIXED_SOURCE_RULES\",\"operationId\":\"assembly-operation\",\"partitions\":[{{\"sourceDomainId\":\"domain-source\"}}],\"principalId\":\"principal-worker\",\"requestDigest\":\"{request_digest}\",\"revocationHead\":\"0\",\"runtimeInstanceId\":\"runtime-one\",\"sessionId\":\"session-one\",\"sourceEpoch\":\"9\",\"taskRevision\":\"1\"}},\"taskId\":\"task-one\"}}",
        hash('a'), hash('a'),
    );
    let manifest_hash = content_hash(body.as_bytes());
    body.replacen(
        "\"manifestId\"",
        &format!("\"manifestHash\":\"{manifest_hash}\",\"manifestId\""),
        1,
    ).into_bytes()
}

fn cleanup_vertical(path: &Path) {
    for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm", ".gogoke-state.sqlite.custody-v1"] {
        match std::fs::remove_file(path.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("owned fixture cleanup failed: {error}"),
        }
    }
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn startup_retains_one_persisted_owner_and_creates_no_automatic_grants() {
    fixture(|_, product| {
        assert!(!product.owner.principal_id().is_empty());
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_authority_profile"), "1");
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_authority_events WHERE event_kind='BOOTSTRAP'"), "1");
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_authority_grants"), "0");
    });
}

#[test]
fn established_root_refuses_lost_database_before_product_authority_bootstrap() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-root-loss-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let database = path.join("state.sqlite");
    let marker = path.join(".gogoke-state.sqlite.custody-v1");
    let root = RootLock::acquire(&path).unwrap();
    let product = ProductDatabase::open(&root, &database).unwrap();
    product.close_checked().unwrap();
    drop(root);

    let original = std::fs::read(&marker).unwrap();
    std::fs::write(&marker, b"tampered\n").unwrap();
    let root = RootLock::acquire(&path).unwrap();
    assert!(matches!(ProductDatabase::open(&root, &database), Err(OrchestrationError::AccessDenied)));
    drop(root);
    assert!(database.exists(), "marker tamper must not mutate the valid database");
    std::fs::write(&marker, original).unwrap();

    for name in ["state.sqlite", "state.sqlite-wal", "state.sqlite-shm"] {
        match std::fs::remove_file(path.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("isolated fixture DB loss failed: {error}"),
        }
    }
    let root = RootLock::acquire(&path).unwrap();
    assert!(matches!(ProductDatabase::open(&root, &database), Err(OrchestrationError::AccessDenied)));
    assert!(matches!(ProductDatabase::open(&root, &path.join("STATE.SQLITE")), Err(OrchestrationError::AccessDenied)));
    assert!(!database.exists(), "rejected reopen must not create a replacement authority journal");
    let blank = create_new(&root, &database).unwrap();
    blank.close_checked().unwrap();
    assert!(matches!(ProductDatabase::open(&root, &database), Err(OrchestrationError::AccessDenied)),
        "an empty replacement DB cannot mint a new Owner under the established root");
    drop(root);
    cleanup_vertical(&path);
}

#[test]
fn existing_valid_database_gains_custody_marker_without_replacing_owner() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-root-migration-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let database = path.join("state.sqlite");
    let marker = path.join(".gogoke-state.sqlite.custody-v1");
    let root = RootLock::acquire(&path).unwrap();
    let mut old = create_new(&root, &database).unwrap();
    let owner = authority::initialize_profile(&mut old, &root).unwrap();
    let original_principal = owner.principal_id().to_owned();
    old.close_checked().unwrap();
    assert!(!marker.exists());
    let product = ProductDatabase::open(&root, &database).unwrap();
    assert_eq!(product.owner.principal_id(), original_principal);
    assert!(marker.is_file());
    product.close_checked().unwrap();
    drop(root);
    cleanup_vertical(&path);
}
#[test]
fn trusted_native_methods_use_the_retained_issuer_and_same_context_database() {
    fixture(|_, product| {
        commit_context_version(&mut product.connection, context()).unwrap();
        let bounds = spec(product, "context.read", 0);
        let grant = product.issue_grant("1", "0", bounds).unwrap();
        let request = read_request(grant);
        assert_eq!(product.read_context(&request).unwrap().context_id, "source-one");
        assert_eq!(product.read_context_set(&[request]).unwrap().sources.len(), 1);
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_authority_grants"), "1");
    });
}

#[test]
fn task_context_methods_share_the_product_database_and_advance_one_cas_head() {
    fixture(|_, product| {
        let create = CommitTaskContextRequirements {
            operation_id: "task-create".into(), domain_id: "domain-one".into(), task_id: "task-one".into(),
            expected_previous_revision: None,
            mandatory_refs: vec![authority::MandatoryContextRef { source_domain_id: "source-one".into(), context_id: "context-one".into(), version: "1".into() }],
            event_id: "task-event".into(), receipt_id: "task-receipt".into(), recorded_at: "2026-09-21T00:00:00Z".into(),
        };
        assert_eq!(product.commit_task_context_requirements(&create).unwrap().current.task_revision, "1");
        assert_eq!(product.read_task_context_requirements("domain-one", "task-one").unwrap().mandatory_refs.len(), 1);
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_task_context_heads"), "1");
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_objects WHERE object_type='Task'"), "1");
    });
}
#[test]
fn revision_and_revocation_are_not_cached_by_the_service_wrapper() {
    fixture(|_, product| {
        commit_context_version(&mut product.connection, context()).unwrap();
        let bounds = spec(product, "context.read", 0);
        let grant = product.issue_grant("1", "0", bounds.clone()).unwrap();
        let mut request = read_request(grant);
        assert!(product.read_context(&request).is_ok());
        let revised = product.revise_grant("1", &request.grant, bounds).unwrap();
        assert!(product.read_context(&request).is_err());
        request.grant = revised;
        assert!(product.read_context(&request).is_ok());
        let head = product.revoke_grant("1", &request.grant).unwrap();
        request.grant.revocation_head = head;
        assert!(product.read_context(&request).is_err());
    });
}
#[test]
fn delegation_uses_current_parent_authority_not_the_retained_owner_name_alone() {
    fixture(|_, product| {
        let parent_spec = spec(product, "context.read", 1);
        let parent = product.issue_grant("1", "0", parent_spec).unwrap();
        let child_spec = spec(product, "context.read", 0);
        product.delegate_grant("1", &parent, child_spec.clone()).unwrap();
        product.revoke_grant("1", &parent).unwrap();
        assert!(product.delegate_grant("1", &parent, child_spec).is_err());
    });
}
#[test]
fn untrusted_line_ingress_cannot_invoke_privileged_owner_methods() {
    fixture(|_, product| {
        let operations = ["IssueOwnerGrant", "ReviseOwnerGrant", "RevokeOwnerGrant", "DelegateOwnerGrant", "ReadOwnerContext", "CommitOwnerPromotion", "AppendOwnerOutcome"];
        let input = operations.iter().map(|op| format!("{{\"operation\":\"{op}\",\"owner\":true}}\n")).collect::<String>();
        let mut output = Vec::new(); product.serve_lines(Cursor::new(input.as_bytes()), &mut output).unwrap();
        let text = String::from_utf8(output).unwrap(); let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), operations.len() + 1); assert_eq!(lines[0], "READY");
        assert!(lines[1..].iter().all(|line| line.starts_with("ERR\t")), "{text}");
        assert!(!text.contains(product.owner.principal_id())); assert!(!text.contains(product.owner.seat_id()));
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_authority_grants"), "0");
    });
}
#[test]
fn an_issuer_swapped_from_another_native_profile_is_rejected() {
    fixture(|root, product| {
        let path = root.canonical_root().canonical_path.join("other.sqlite");
        let mut other = ProductDatabase::open(root, &path).unwrap();
        let bounds = spec(product, "context.read", 0);
        std::mem::swap(&mut product.owner, &mut other.owner);
        assert!(product.issue_grant("1", "0", bounds).is_err());
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_authority_grants"), "0");
        std::mem::swap(&mut product.owner, &mut other.owner);
        other.close_checked().unwrap(); std::fs::remove_file(path).unwrap();
        std::fs::remove_file(root.canonical_root().canonical_path.join(".gogoke-other.sqlite.custody-v1")).unwrap();
    });
}
#[test]
fn trusted_promotion_method_uses_atomic_native_commit_and_reauthorizes_replay() {
    fixture(|_, product| {
        commit_context_version(&mut product.connection, context()).unwrap();
        let source_bounds = spec(product, "context.promote.source", 0);
        let source_grant = product.issue_grant("1", "0", source_bounds).unwrap();
        let target_bounds = spec(product, "context.promote.target", 0);
        let target_grant = product.issue_grant("1", "0", target_bounds).unwrap();
        let request = PromotionRequest { source_context_id: "source-one".into(), source_version: "1".into(),
            source_domain_id: "domain-one".into(), source_content_hash: format!("sha256:{}", "a".repeat(64)),
            source_access_policy_revision: "1".into(), destination_domain_id: "domain-one".into(),
            destination_scope: "GLOBAL".into(), promotion_kind: "GLOBAL_LESSON".into(), policy_revision: "1".into(),
            source_grant, target_grant, provenance_refs: vec!["evidence://review".into()] };
        let mut target = context(); target.operation_id = "promotion-operation".into();
        target.context_id = "global-one".into(); target.scope = "GLOBAL".into();
        target.derived_from = vec!["source-one@1".into()];
        target.promotion = Some(PromotionEvidence { source_version_ref: "source-one@1".into(),
            source_grant_ref: request.source_grant.grant_id.clone(), target_grant_ref: request.target_grant.grant_id.clone(),
            provenance_refs: request.provenance_refs.clone() });
        assert_eq!(product.promote(&request, target.clone()).unwrap().disposition, "COMMITTED");
        assert_eq!(product.promote(&request, target.clone()).unwrap().disposition, "RECONCILED");
        product.revoke_grant("1", &request.source_grant).unwrap();
        assert!(product.promote(&request, target).is_err());
        assert_eq!(scalar(product, "SELECT count(*) FROM gogoke_context_promotion_authorizations"), "1");
    });
}

#[test]
fn controlled_vertical_survives_checked_close_and_reopen_on_one_product_database() {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-product-vertical-{}-{nonce}",
        std::process::id(),
    ));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let manifest_receipt;
    let resource_decision;

    {
        let mut product = ProductDatabase::open(&root, &database).unwrap();
        let task = CommitTaskContextRequirements {
            operation_id: "vertical-task-create".into(),
            domain_id: "domain-destination".into(),
            task_id: "task-one".into(),
            expected_previous_revision: None,
            mandatory_refs: vec![authority::MandatoryContextRef {
                source_domain_id: "domain-source".into(),
                context_id: "context-one".into(),
                version: "1".into(),
            }],
            event_id: "vertical-task-event".into(),
            receipt_id: "vertical-task-receipt".into(),
            recorded_at: VERTICAL_WHEN.into(),
        };
        assert_eq!(
            product.commit_task_context_requirements(&task).unwrap().current.task_revision,
            "1",
        );

        let grant = vertical_grant(&mut product);
        commit_context_version(&mut product.connection, vertical_context(&grant)).unwrap();
        assert_eq!(
            product.read_grantee_context_set(&[vertical_grantee_read(grant.clone())])
                .unwrap().sources.len(),
            1,
        );

        let admission = vertical_action(VERTICAL_ADMISSION_ACTION, "admission-reservation", 'c');
        let execution = vertical_action(VERTICAL_EXECUTION_ACTION, "execution-reservation", 'd');
        assert_eq!(
            action::reserve_action(&mut product.connection, admission.clone()).unwrap(),
            ReserveDisposition::Reserved,
        );
        assert_eq!(
            action::reserve_action(&mut product.connection, execution.clone()).unwrap(),
            ReserveDisposition::Reserved,
        );

        // Current Manifest authority requires its CONTEXT_SELECTION Decision to
        // pre-exist. The downstream RESOURCE_SELECTION Decision remains after
        // the Manifest, matching the service vertical rather than faking this
        // native precondition away.
        commit_vertical_decision(
            &mut product,
            "context-decision-operation",
            "context-decision",
            "DF10",
            "CONTEXT_SELECTION",
            "context-candidate",
            &admission,
            "context-pool",
        );

        let assembly = vertical_assembly_snapshot(grant.clone(), &admission);
        product.publish_context_assembly_snapshot(&assembly).unwrap();
        let identity = ContextManifestReplayIdentity {
            operation_id: assembly.operation_id.clone(),
            principal_id: assembly.principal_id.clone(),
            seat_id: assembly.seat_id.clone(),
            task_id: assembly.task_id.clone(),
            session_id: assembly.session_id.clone(),
            domain_id: assembly.domain_id.clone(),
            binding_id: assembly.binding_id.clone(),
            binding_generation: assembly.binding_generation.clone(),
            source_epoch: assembly.source_epoch.clone(),
            runtime_instance_id: assembly.runtime_instance_id.clone(),
        };
        let basis = product.read_context_assembly_basis(&identity).unwrap();
        assert_eq!(basis.mandatory_refs.len(), 1);
        let sources = product.list_context_assembly_sources(&identity).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].grant_id, grant.grant_id);
        assert_eq!(sources[0].grant_revision, grant.revision);
        assert_eq!(sources[0].grant_revocation_head, grant.revocation_head);

        let request_digest = hash('1');
        let canonical_manifest = vertical_manifest(&request_digest);
        manifest_receipt = product.commit_context_manifest(&ContextManifestCommitInput {
            operation_id: assembly.operation_id.clone(),
            request_digest,
            event_id: "manifest-event".into(),
            receipt_id: "manifest-receipt".into(),
            recorded_at: VERTICAL_WHEN.into(),
            read_requests: vec![vertical_grantee_read(grant)],
            expected_versions: vec![authority::ManifestExpectedVersion {
                source_domain_id: "domain-source".into(),
                context_id: "context-one".into(),
                version: "1".into(),
                content_hash: hash('a'),
                state_revision: "1".into(),
                access_policy_revision: "1".into(),
            }],
            canonical_manifest,
        }).unwrap();
        assert_eq!(manifest_receipt.disposition, "COMMITTED");

        resource_decision = commit_vertical_decision(
            &mut product,
            "resource-decision-operation",
            "resource-decision",
            "DF02",
            "RESOURCE_SELECTION",
            "worker-candidate",
            &execution,
            "worker-pool",
        );
        assert_eq!(resource_decision.replay.action_intent_ref, VERTICAL_EXECUTION_ACTION);
        assert!(matches!(
            action::begin_action_commitment(
                &mut product.connection,
                &execution.reservation_id,
                &execution,
            ).unwrap(),
            BeginDisposition::Granted { .. },
        ));
        action::record_action_outcome(
            &mut product.connection,
            &execution.reservation_id,
            &execution.operation_id,
            &execution.semantic_digest,
            "completed",
            "fake-native-complete",
            "fake-provider-receipt",
        ).unwrap();

        let outcome = format!(
            "{{\"actionId\":\"{VERTICAL_EXECUTION_ACTION}\",\"censorStatus\":\"OBSERVED\",\"cost\":null,\"decisionId\":\"resource-decision\",\"evidenceRefs\":[\"fake-provider-receipt\"],\"labelSource\":\"OWNER_OVERRIDE\",\"latency\":null,\"observationWindow\":null,\"outcomeId\":\"outcome-one\",\"quality\":null,\"revision\":\"1\",\"rework\":null,\"safetyEvents\":[]}}",
        ).into_bytes();
        assert_eq!(
            product.append_owner_outcome(&OwnerOutcomeAppend {
                domain_id: "domain-destination".into(),
                operation_id: "outcome-operation-one".into(),
                event_id: "outcome-event-one".into(),
                receipt_id: "outcome-receipt-one".into(),
                recorded_at: VERTICAL_WHEN.into(),
                policy_revision: "1".into(),
                revocation_head: "0".into(),
                decision_version: resource_decision.replay.object_version.clone(),
                decision_hash: resource_decision.replay.decision_content_hash.clone(),
                action_operation_id: execution.operation_id.clone(),
                action_digest: execution.semantic_digest.clone(),
                previous: None,
                canonical_outcome: outcome,
            }).unwrap().disposition,
            "COMMITTED",
        );
        product.close_checked().unwrap();
    }

    {
        let mut reopened = ProductDatabase::open(&root, &database).unwrap();
        let task = reopened.read_task_context_requirements("domain-destination", "task-one").unwrap();
        assert_eq!(task.task_revision, "1");
        assert_eq!(task.mandatory_refs.len(), 1);
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_context_versions"), "1");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_context_states WHERE state='ACTIVE'"), "1");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_authority_grants"), "2");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_task_context_heads"), "1");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_objects"), "5");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_events"), "5");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_receipts"), "5");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_objects WHERE object_type='ContextManifest'"), "1");
        assert_eq!(scalar(&reopened, "SELECT count(*) FROM gogoke_objects WHERE object_type='OutcomeRecord'"), "1");
        assert_eq!(scalar(&reopened, "SELECT state FROM gogoke_action_reservations WHERE operation_id='opr_22222222222222222222222222222222'"), "completed");
        assert_eq!(
            reopened.read_decision_replay("domain-destination", "resource-decision-operation")
                .unwrap().decision_content_hash,
            resource_decision.replay.decision_content_hash,
        );
        let replay = reopened.read_context_manifest(&ContextManifestReplayIdentity {
            operation_id: "assembly-operation".into(),
            principal_id: "principal-worker".into(),
            seat_id: "seat-worker".into(),
            task_id: "task-one".into(),
            session_id: "session-one".into(),
            domain_id: "domain-destination".into(),
            binding_id: "binding-one".into(),
            binding_generation: "7".into(),
            source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
        }).unwrap();
        assert_eq!(replay.manifest_hash, manifest_receipt.manifest_hash);
        assert_eq!(replay.canonical_manifest, manifest_receipt.canonical_manifest);
        reopened.close_checked().unwrap();
    }
    drop(root);
    cleanup_vertical(&path);
}

#[test]
fn controlled_vertical_midstage_failure_rolls_back_without_an_action_half_row() {
    fixture(|_, product| {
        action::apply_action_schema(&mut product.connection).unwrap();
        let input = vertical_action(
            "opr_33333333333333333333333333333333",
            "rollback-reservation",
            'e',
        );
        product.connection.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(
            action::reserve_action_in_transaction(&mut product.connection, input).unwrap(),
            ReserveDisposition::Reserved,
        );
        product.connection.execute("ROLLBACK").unwrap();
        assert_eq!(
            scalar(
                product,
                "SELECT count(*) FROM gogoke_action_reservations WHERE operation_id='opr_33333333333333333333333333333333'",
            ),
            "0",
        );
    });
}
