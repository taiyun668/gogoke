use super::*;
use crate::root::RootLock;
use crate::store::action::apply_action_schema;
use crate::store::atomic::{apply_core_schema, commit_domain_record, DomainRecordInput, Statement};
use crate::store::context::{apply_context_schema, commit_context_version, ContextCommand};
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn fixture(
    run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &super::super::OwnerIssuer),
) {
    let _guard = route_b_test_guard();
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-action-authority-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = create_new(&root, &database).unwrap();
    apply_core_schema(&mut db).unwrap();
    apply_action_schema(&mut db).unwrap();
    apply_context_schema(&mut db).unwrap();
    let owner = super::super::initialize_profile(&mut db, &root).unwrap();
    super::super::initialize_decision_capacity_schema(&mut db).unwrap();
    super::super::context_manifest::initialize_context_manifest_schema(&mut db).unwrap();
    run(&root, &mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    std::fs::remove_dir(path).unwrap();
}

fn count(db: &VerifiedDatabaseConnection<'_>, sql: &str) -> String {
    let s = Statement::prepare(db.as_ptr(), sql).unwrap();
    assert!(s.step_row().unwrap());
    let value = s.column_text(0).unwrap();
    assert!(!s.step_row().unwrap());
    value
}

#[test]
fn exact_native_transport_hash_rejects_changed_or_missing_frames() {
    let transport = TrustedActionTransportEvidence {
        stop_proof_hash: format!("sha256:{}", "a".repeat(64)),
        pid: "123".into(), creation_time_100ns: "456".into(),
        binary_digest_sha256: format!("sha256:{}", "b".repeat(64)),
        frames: vec!["ack\n".into(), "start\n".into(), "message\n".into(),
            "end\n".into(), "settled\n".into()],
    };
    let mut evidence = TrustedActionCompletionEvidence {
        domain_id: "domain-one".into(), operation_id: "operation-one".into(),
        reservation_id: "reservation-one".into(), semantic_digest: format!("sha256:{}", "c".repeat(64)),
        attempt_id: "attempt-one".into(), send_authority: "send-one".into(),
        binding_id: "binding-one".into(), generation: "7".into(), source_epoch: "9".into(),
        runtime_instance_id: "runtime-one".into(), native_request_id: "request-one".into(),
        native_session_id: "session-one".into(), trusted_receipt_ref: String::new(),
        evidence_hash: String::new(), disposition: ActionCompletionDisposition::Completed,
    };
    let material = format!("{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
        evidence.operation_id, evidence.attempt_id, evidence.send_authority,
        evidence.native_request_id, transport.pid, transport.creation_time_100ns,
        transport.binary_digest_sha256, evidence.semantic_digest, transport.stop_proof_hash,
        evidence.native_session_id,
        transport.frames.iter().map(|frame| format!("{}:{frame}", frame.len())).collect::<String>());
    evidence.evidence_hash = content_hash(material.as_bytes());
    evidence.trusted_receipt_ref = format!("native-receipt-{}", &evidence.evidence_hash[7..]);
    assert!(transport_matches_receipt(&evidence, &transport));
    let mut changed = transport.clone();
    changed.frames[2].push('x');
    assert!(!transport_matches_receipt(&evidence, &changed));
    changed = transport.clone();
    changed.frames[2].replace_range(0..1, "X");
    assert!(!transport_matches_receipt(&evidence, &changed),
        "same-length byte changes must not retain the trusted evidence hash");
    changed = transport.clone();
    changed.frames.pop();
    assert!(!transport_matches_receipt(&evidence, &changed));
    changed = transport.clone();
    changed.stop_proof_hash.replace_range(7..8, "f");
    assert!(!transport_matches_receipt(&evidence, &changed));
}

pub(crate) fn persist_manifest(db: &mut VerifiedDatabaseConnection<'_>, owner: &super::super::OwnerIssuer, action: &PrepareActionAuthority, semantic_digest: &str) -> String {
    use super::super::context_manifest::{commit_context_manifest, publish_context_assembly_snapshot, ContextAssemblySnapshot, ContextManifestCommitInput, ContextPartitionGrantBinding, ManifestExpectedVersion};
    use super::super::context_read::{ContextReadRequest, GranteeContextReadRequest};
    use super::super::model::{GrantRef, GrantSpec};
    let hash_a = format!("sha256:{}", "a".repeat(64));
    let parent = super::super::issue_owner_grant(db, owner, "1", "0", GrantSpec {
        principal_id: owner.principal_id().into(), seat_id: owner.seat_id().into(), permission: "context.read".into(),
        promotion_kind: "PROJECT_ONLY".into(), source_domain_id: "domain-source".into(), destination_domain_id: "domain-one".into(),
        destination_scope: "PROJECT".into(), delegable_depth: 2,
    }).unwrap();
    let grant = super::super::delegate_owner_grant(db, owner, "1", &parent, GrantSpec {
        principal_id: "principal-worker".into(), seat_id: "seat-worker".into(), permission: "context.read".into(),
        promotion_kind: "PROJECT_ONLY".into(), source_domain_id: "domain-source".into(), destination_domain_id: "domain-one".into(),
        destination_scope: "PROJECT".into(), delegable_depth: 1,
    }).unwrap();
    commit_context_version(db, ContextCommand {
        operation_id: "action-manifest-source-op".into(), context_id: "action-context".into(), version: "1".into(), scope: "PROJECT".into(),
        domain_id: "domain-source".into(), kind: "fact".into(), content_hash: hash_a.clone(), source_ref: "source://fixture".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(), source_authority_ref: "authority://fixture".into(),
        derived_from: vec![], supersedes: vec![], access_policy_revision: "1".into(), visibility: "DOMAIN_GRANTED".into(),
        read_grant_refs: vec![grant.grant_id.clone()], promotion: None,
    }).unwrap();
    let decision_id = "action-context-decision";
    let decision = format!("{{\"actionId\":\"{}\",\"backend\":\"FAKE\",\"calibrationRef\":\"NONE\",\"candidateHash\":\"{}\",\"decisionId\":\"{}\",\"family\":\"CONTEXT_SELECTION\",\"mode\":\"fixture_bounded_auto\",\"modelRequested\":\"NONE\",\"modelResolved\":\"fake-v1\",\"nativeConfidence\":null,\"probabilities\":{{}},\"questionVersion\":\"1\",\"sourceRevisions\":{{\"bindingGeneration\":\"7\",\"capabilityRevision\":\"1\",\"policyRevision\":\"1\",\"taskRevision\":\"1\"}},\"state\":\"COMMITTED\",\"stateViewHash\":\"{}\"}}", action.action_operation_id, format!("sha256:{}", "c".repeat(64)), decision_id, format!("sha256:{}", "d".repeat(64)));
    commit_domain_record(db, DomainRecordInput { domain_id: "domain-one".into(), object_type: "DecisionRecord".into(), object_id: decision_id.into(), object_version: "1".into(),
        object_bytes: decision.into_bytes(), native_identity: None, event_id: "action-context-decision-event".into(), stream_id: "action-context-decision-stream".into(),
        expected_previous_counter: None, counter: "0".into(), event_type: "DecisionApplied".into(), occurred_at: "2026-09-22T00:00:00Z".into(), event_bytes: b"{}".to_vec(),
        receipt_id: "action-context-decision-receipt".into(), operation_id: "action-context-decision-op".into(), receipt_type: "DecisionApplied".into(), recorded_at: "2026-09-22T00:00:00Z".into(), receipt_bytes: b"{}".to_vec() }).unwrap();
    let grant_ref = GrantRef { grant_id: grant.grant_id.clone(), revision: grant.revision.clone(), revocation_head: grant.revocation_head.clone() };
    publish_context_assembly_snapshot(db, &ContextAssemblySnapshot {
        operation_id: "action-context-assembly".into(), principal_id: "principal-worker".into(), seat_id: "seat-worker".into(), task_id: "task-one".into(),
        session_id: action.session_id.clone(), domain_id: "domain-one".into(), binding_id: "binding-worker".into(), binding_generation: "7".into(), source_epoch: "9".into(),
        runtime_instance_id: "runtime-one".into(), task_revision: "1".into(), policy_revision: "1".into(), auth_revision: "1".into(), revocation_head: "0".into(),
        selection_decision_id: decision_id.into(), manifest_id: action.context_manifest_id.clone(), admission_action_operation_id: action.action_operation_id.clone(),
        admission_digest: semantic_digest.into(), max_content_bytes: 4096, max_candidates: 8,
        partition_grant_bindings: vec![ContextPartitionGrantBinding { source_domain_id: "domain-source".into(), destination_scope: "PROJECT".into(), promotion_kind: "PROJECT_ONLY".into(), grant: grant_ref.clone() }],
    }).unwrap();
    let read = GranteeContextReadRequest { principal_id: "principal-worker".into(), seat_id: "seat-worker".into(), source: ContextReadRequest {
        source_domain_id: "domain-source".into(), context_id: "action-context".into(), version: "1".into(), expected_scope: "PROJECT".into(), expected_content_hash: hash_a.clone(),
        expected_access_policy_revision: "1".into(), destination_domain_id: "domain-one".into(), destination_scope: "PROJECT".into(), promotion_kind: "PROJECT_ONLY".into(), policy_revision: "1".into(), grant: grant_ref,
    }};
    let body = format!("{{\"bindingGeneration\":\"7\",\"domainId\":\"domain-one\",\"includedVersions\":[{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"{}\",\"contextId\":\"action-context\",\"reason\":\"AUTHORIZED_RETRIEVAL\",\"sourceDomainId\":\"domain-source\",\"stateRevision\":\"1\",\"version\":\"1\"}}],\"manifestId\":\"{}\",\"policyRevision\":\"1\",\"redactions\":[],\"requiredConstraints\":[],\"seatId\":\"seat-worker\",\"selectionDecisionId\":\"{}\",\"sourceSnapshot\":{{\"assemblySchema\":\"gogoke.context-assembly.v1\",\"authRevision\":\"1\",\"bindingId\":\"binding-worker\",\"excluded\":[],\"mode\":\"FIXED_SOURCE_RULES\",\"operationId\":\"action-context-assembly\",\"partitions\":[{{\"sourceDomainId\":\"domain-source\"}}],\"principalId\":\"principal-worker\",\"requestDigest\":\"{}\",\"revocationHead\":\"0\",\"runtimeInstanceId\":\"runtime-one\",\"sessionId\":\"{}\",\"sourceEpoch\":\"9\",\"taskRevision\":\"1\"}},\"taskId\":\"task-one\"}}", format!("sha256:{}", "a".repeat(64)), action.context_manifest_id, decision_id, format!("sha256:{}", "e".repeat(64)), action.session_id);
    let manifest_hash = crate::store::digest::content_hash(body.as_bytes());
    let canonical = body.replacen("\"manifestId\"", &format!("\"manifestHash\":\"{manifest_hash}\",\"manifestId\""), 1).into_bytes();
    commit_context_manifest(db, &ContextManifestCommitInput { operation_id: "action-context-assembly".into(), request_digest: format!("sha256:{}", "e".repeat(64)),
        event_id: "action-context-manifest-event".into(), receipt_id: "action-context-manifest-receipt".into(), recorded_at: "2026-09-22T00:00:00Z".into(), read_requests: vec![read],
        expected_versions: vec![ManifestExpectedVersion { source_domain_id: "domain-source".into(), context_id: "action-context".into(), version: "1".into(), content_hash: hash_a, state_revision: "1".into(), access_policy_revision: "1".into() }], canonical_manifest: canonical }).unwrap().manifest_hash
}

#[test]
fn prepare_fails_closed_without_native_current_atp_and_writes_no_action() {
    fixture(|_, db, _| {
        let request = PrepareActionAuthority {
            domain_id: "domain-one".into(),
            parent_grant_ref: "grant-one".into(),
            package_operation_id: "package-operation-one".into(),
            task_id: "task-one".into(),
            recipe_id: "recipe-one".into(),
            session_id: "session-one".into(),
            context_manifest_id: "manifest-one".into(),
            action_operation_id: "opr_11111111111111111111111111111111".into(),
            reservation_id: "reservation-one".into(),
            action_kind: "queue".into(),
            lane: "work".into(),
            payload: b"instruction".to_vec(),
        };
        assert!(prepare_action_authority(db, &request).is_err());
        assert_eq!(
            count(db, "SELECT count(*) FROM main.gogoke_action_reservations"),
            "0"
        );
        assert_eq!(count(db,"SELECT count(*) FROM main.sqlite_schema WHERE name='gogoke_action_authority_intents'"),"0");
    });
}

#[test]
fn current_atp_task_lineage_and_recipe_prepare_one_native_derived_reservation() {
    fixture(|_, db, owner| {
        super::super::initialize_task_context_schema(db).unwrap();
        super::super::initialize_authorized_task_package_schema(db).unwrap();
        super::super::initialize_session_lineage_schema(db).unwrap();
        super::super::initialize_execution_recipe_schema(db).unwrap();
        super::super::commit_task_context_requirements(
            db,
            &super::super::CommitTaskContextRequirements {
                operation_id: "task-create".into(),
                domain_id: "domain-one".into(),
                task_id: "task-one".into(),
                expected_previous_revision: None,
                mandatory_refs: vec![],
                event_id: "task-event".into(),
                receipt_id: "task-receipt".into(),
                recorded_at: "2026-09-22T00:00:00Z".into(),
            },
        )
        .unwrap();
        let ceiling = super::super::AuthorityCeiling {
            allowed_actions: vec!["delegate".into()],
            allowed_target_principal_ids: vec!["principal-worker".into()],
            allowed_target_domain_ids: vec!["domain-one".into()],
            allowed_sinks: vec!["task-package".into()],
            allowed_material_classes: vec![],
            explicit_private_material_ids: vec![],
            allowed_continuation_responses: vec![],
            max_material_items: 0,
            max_material_bytes: 0,
            max_response_bytes: 256,
        };
        let expiry = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 3_600_000;
        let grant = super::super::issue_owner_delegation(
            db,
            owner,
            super::super::DelegationGrantInput {
                principal: super::super::DelegationPrincipal {
                    principal_id: owner.principal_id().into(),
                    project_id: "project-one".into(),
                    domain_id: "domain-one".into(),
                    role: "controller".into(),
                    seat_id: owner.seat_id().into(),
                },
                binding: super::super::DelegationBinding {
                    session_id: "session-source".into(),
                    execution_id: "execution-source".into(),
                    generation: "1".into(),
                },
                expires_at_epoch_ms: expiry,
                ceiling: ceiling.clone(),
            },
        )
        .unwrap();
        let draft = super::super::AuthorizedTaskPackageDraft {
            parent_grant_ref: grant.reference.grant_id.clone(),
            parent_grant_revision: grant.reference.revision.clone(),
            parent_grant_revocation_head: grant.reference.revocation_head.clone(),
            parent_policy_revision: grant.policy_revision.clone(),
            parent_seat_id: grant.principal.seat_id.clone(),
            child_ceiling: ceiling.clone(),
            action: "delegate".into(),
            route: "controller-worker".into(),
            source: super::super::TaskPackagePrincipal {
                principal_id: grant.principal.principal_id.clone(),
                project_id: grant.principal.project_id.clone(),
                domain_id: grant.principal.domain_id.clone(),
                role: grant.principal.role.clone(),
            },
            target: super::super::TaskPackagePrincipal {
                principal_id: "principal-worker".into(),
                project_id: "project-one".into(),
                domain_id: "domain-one".into(),
                role: "worker".into(),
            },
            source_binding: super::super::TaskPackageBinding {
                session_id: grant.binding.session_id.clone(),
                execution_id: grant.binding.execution_id.clone(),
                generation: grant.binding.generation.clone(),
            },
            target_binding: super::super::TaskPackageBinding {
                session_id: "session-worker".into(),
                execution_id: "execution-worker".into(),
                generation: "7".into(),
            },
            target_binding_kind: "existing".into(),
            sink: "task-package".into(),
            instruction: "bounded instruction".into(),
        };
        let package = super::super::prepare_authorized_task_package(
            db,
            &super::super::PrepareAuthorizedTaskPackage {
                operation_id: "package-operation".into(),
                domain_id: "domain-one".into(),
                event_id: "package-event".into(),
                receipt_id: "package-receipt".into(),
                recorded_at: "2026-09-22T00:00:00Z".into(),
                package: draft,
                material_refs: vec![],
            },
        )
        .unwrap();
        super::super::apply_session_lineage_command(
            db,
            &super::super::SessionLineageCommand {
                operation_id: "lineage-operation".into(),
                domain_id: "domain-one".into(),
                event_id: "lineage-event".into(),
                receipt_id: "lineage-receipt".into(),
                recorded_at: "2026-09-22T00:00:00Z".into(),
                operation: super::super::SessionLineageOperation::NewClean {
                    session_id: "session-worker".into(),
                    native: super::super::NativeSessionIdentity {
                        native_session_id: "native-worker".into(),
                        binding_id: "binding-worker".into(),
                        generation: "7".into(),
                        source_epoch: "9".into(),
                        domain_id: "domain-one".into(),
                    },
                },
            },
        )
        .unwrap();
        super::super::append_owner_execution_recipe(
            db,
            owner,
            &super::super::AppendExecutionRecipe {
                operation_id: "recipe-operation".into(),
                domain_id: "domain-one".into(),
                expected_previous_revision: None,
                recipe_id: "recipe-one".into(),
                seat_id: "seat-worker".into(),
                runtime_instance_id: "runtime-one".into(),
                model_ref: BTreeMap::new(),
                tool_profile: super::super::RecipeJsonValue::Null,
                isolation_profile: super::super::RecipeJsonValue::Null,
                context_manifest_id: "manifest-one".into(),
                budget_policy: super::super::RecipeJsonValue::Null,
                admission_ref: grant.reference.grant_id.clone(),
                event_id: "recipe-event".into(),
                receipt_id: "recipe-receipt".into(),
                recorded_at: "2026-09-22T00:00:00Z".into(),
            },
        )
        .unwrap();
        let request = PrepareActionAuthority {
            domain_id: "domain-one".into(),
            parent_grant_ref: grant.reference.grant_id.clone(),
            package_operation_id: "package-operation".into(),
            task_id: "task-one".into(),
            recipe_id: "recipe-one".into(),
            session_id: "session-worker".into(),
            context_manifest_id: "manifest-one".into(),
            action_operation_id: "opr_11111111111111111111111111111111".into(),
            reservation_id: "action-reservation".into(),
            action_kind: "queue".into(),
            lane: "work".into(),
            payload: b"bounded instruction".to_vec(),
        };
        let (action_digest, policy_revision) = super::super::transaction::run(db, |tx| {
            let (resolved, task, _, recipe, profile) = current_selection(tx, &request)?;
            let payload_digest = crate::store::digest::content_hash(&request.payload);
            Ok((
                intent_digest(
                    &request,
                    &resolved.package_digest,
                    &task.task_revision,
                    &recipe.recipe.revision,
                    &recipe.content_hash,
                    &profile.policy_revision,
                    &payload_digest,
                ),
                profile.policy_revision,
            ))
        })
        .unwrap();
        let decision_record = super::super::DurableDecisionRecord {
            operation_id: "decision-operation".into(),
            scenario_id: "DF02".into(),
            family: "RESOURCE_SELECTION".into(),
            state_view_hash: format!("sha256:{}", "a".repeat(64)),
            candidate_hash: format!("sha256:{}", "b".repeat(64)),
            question_version: "1".into(),
            rubric_version: "1".into(),
            model_requested: None,
            model_resolved: Some("fixture-fake".into()),
            task_revision: "1".into(),
            policy_revision: policy_revision.clone(),
            capability_revision: "1".into(),
            binding_generation: "7".into(),
            backend_kind: "FAKE".into(),
            choice: "candidate-one".into(),
            reason: "QUALIFIED_BOUNDED_SELECTION".into(),
            budget_units: 1,
            deadline_epoch_ms: 9_007_199_254_740_000,
        };
        let decision_snapshot = super::super::DecisionAuthoritySnapshot {
            operation_id: decision_record.operation_id.clone(),
            candidate_id: decision_record.choice.clone(),
            state_view_hash: decision_record.state_view_hash.clone(),
            candidate_hash: decision_record.candidate_hash.clone(),
            task_revision: decision_record.task_revision.clone(),
            policy_revision: decision_record.policy_revision.clone(),
            capability_revision: decision_record.capability_revision.clone(),
            binding_id: "binding-worker".into(),
            binding_generation: decision_record.binding_generation.clone(),
            auth_revision: policy_revision,
            resource_ref: "pool-one".into(),
            resource_revision: "1".into(),
            capacity_total: 2,
            action_operation_id: "opr_11111111111111111111111111111111".into(),
            action_digest,
        };
        super::super::publish_decision_snapshot(db, &decision_snapshot).unwrap();
        super::super::commit_decision(
            db,
            &super::super::DecisionCommitInput {
                domain_id: "domain-one".into(),
                decision_id: "decision-one".into(),
                event_id: "decision-event".into(),
                receipt_id: "decision-receipt".into(),
                recorded_at: "2026-09-22T00:00:00Z".into(),
                record: decision_record,
                resource_reservation_ref: "capacity-lease-one".into(),
                action_intent_ref: "opr_11111111111111111111111111111111".into(),
                required_capacity_units: 1,
            },
        )
        .unwrap();
        let prepared = prepare_action_authority(db, &request).unwrap();
        assert_eq!(prepared.disposition, "COMMITTED");
        assert_eq!(prepared.authority_status, ACTION_AUTHORITY_STATUS);
        assert_eq!(count(db,"SELECT state FROM main.gogoke_action_reservations WHERE operation_id='opr_11111111111111111111111111111111'"),"reserved");
        assert_eq!(count(db,"SELECT count(*) FROM main.gogoke_action_authority_intents WHERE domain_id='domain-one'"),"1");
        assert_eq!(package.disposition, "COMMITTED");
        let recipe =
            super::super::read_current_execution_recipe(db, owner, "domain-one", "recipe-one")
                .unwrap()
                .unwrap();
        let manifest_hash = persist_manifest(db, owner, &request, &prepared.semantic_digest);
        let facts = TrustedNativeActionFacts {
            domain_id: "domain-one".into(),
            operation_id: "opr_11111111111111111111111111111111".into(),
            expected_previous_revision: None,
            session_id: "session-worker".into(),
            binding_id: "binding-worker".into(),
            generation: "7".into(),
            source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
            model_ref_digest: super::super::execution_recipe::model_ref_digest(&recipe.recipe)
                .unwrap(),
            capability_revision: "1".into(),
            context_manifest_id: "manifest-one".into(),
            context_manifest_hash: manifest_hash,
            admission_ref: grant.reference.grant_id.clone(),
            admission_revision: grant.reference.revision.clone(),
            expires_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60_000,
        };
        let mut stale_manifest = facts.clone();
        stale_manifest.context_manifest_hash = format!("sha256:{}", "f".repeat(64));
        assert!(record_trusted_native_action_facts(db, &stale_manifest).is_err());
        let mut stale_admission = facts.clone();
        stale_admission.admission_revision = "2".into();
        assert!(record_trusted_native_action_facts(db, &stale_admission).is_err());
        record_trusted_native_action_facts(db, &facts).unwrap();
        let begin = BeginCommittedAction {
            domain_id: "domain-one".into(),
            operation_id: "opr_11111111111111111111111111111111".into(),
            reservation_id: "action-reservation".into(),
        };
        let original_manifest = super::super::transaction::run(db, |tx| {
            let rows = tx.query("SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='ContextManifest' AND object_id=? AND object_version='1'", &["domain-one", "manifest-one"], 1)?;
            if rows.len() != 1 { return Err(crate::store::orchestration::OrchestrationError::AccessDenied); }
            tx.write("UPDATE main.gogoke_objects SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id=? AND object_type='ContextManifest' AND object_id=? AND object_version='1'", &["domain-one", "manifest-one"])?;
            Ok(rows[0][0].clone())
        }).unwrap();
        assert!(begin_committed_action(db, &begin).is_err());
        super::super::transaction::run(db, |tx| {
            tx.write("UPDATE main.gogoke_objects SET canonical_json=CAST(? AS BLOB) WHERE domain_id=? AND object_type='ContextManifest' AND object_id=? AND object_version='1'", &[&original_manifest, "domain-one", "manifest-one"])?;
            Ok(())
        }).unwrap();
        let first = begin_committed_action(db, &begin).unwrap();
        let BeginCommittedDisposition::Granted {
            attempt_id,
            send_authority,
        } = first
        else {
            panic!("first current begin must grant exactly once");
        };
        assert!(attempt_id.starts_with("attempt:") && send_authority.starts_with("send:"));
        let unknown = complete_action_from_native_receipt(db, &begin).unwrap();
        assert_eq!(unknown.disposition, "ACCEPTANCE_UNKNOWN");
        assert_eq!(
            begin_committed_action(db, &begin).unwrap(),
            BeginCommittedDisposition::Replay {
                state: "ACCEPTANCE_UNKNOWN".into()
            }
        );
        record_trusted_native_action_receipt(
            db,
            &TrustedActionCompletionEvidence {
                domain_id: "domain-one".into(),
                operation_id: begin.operation_id.clone(),
                reservation_id: begin.reservation_id.clone(),
                semantic_digest: prepared.semantic_digest.clone(),
                attempt_id,
                send_authority,
                binding_id: "binding-worker".into(),
                generation: "7".into(),
                source_epoch: "9".into(),
                runtime_instance_id: "runtime-one".into(),
                native_request_id: "native-request-one".into(),
                native_session_id: "native-worker".into(),
                trusted_receipt_ref: "native-receipt-one".into(),
                evidence_hash: format!("sha256:{}", "d".repeat(64)),
                disposition: ActionCompletionDisposition::Completed,
            },
        )
        .unwrap();
        let completed = complete_action_from_native_receipt(db, &begin).unwrap();
        assert_eq!(completed.disposition, "completed");
        assert!(!completed.receipt_id.is_empty());
        assert_eq!(
            complete_action_from_native_receipt(db, &begin)
                .unwrap()
                .disposition,
            "REPLAYED"
        );
        assert_eq!(count(db,"SELECT state FROM main.gogoke_action_reservations WHERE operation_id='opr_11111111111111111111111111111111'"),"completed");
        assert_eq!(count(db,"SELECT count(*) FROM main.gogoke_action_completion_receipts WHERE operation_id='opr_11111111111111111111111111111111'"),"1");
        assert!(read_reconciled_action_transport(db, &begin, "bounded instruction", &completed.receipt_id).is_err(),
            "legacy completion without exact transport frames must stay unknown");
        super::super::transaction::run(db, |tx| {
            tx.write("INSERT INTO main.gogoke_action_transport_evidence(domain_id,operation_id,receipt_ref,evidence_hash,stop_proof_hash,pid,creation_time_100ns,binary_digest_sha256,frame0,frame1,frame2,frame3,frame4) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
                &["domain-one", &begin.operation_id, "native-receipt-one", &format!("sha256:{}", "d".repeat(64)),
                  &format!("sha256:{}", "a".repeat(64)), "123", "456", &format!("sha256:{}", "b".repeat(64)),
                  "ack", "start", "message", "end", "settled"])?;
            Ok(())
        }).unwrap();
        assert!(read_reconciled_action_transport(db, &begin, "bounded instruction", &completed.receipt_id).is_err(),
            "fabricated or changed transport frames must not recover the old completion");

        // A typed completion projection cannot treat a corrupted canonical
        // object as valid replay evidence.
        super::super::transaction::run(db, |tx| {
            tx.write(
                "UPDATE main.gogoke_objects SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id=? AND object_type='ActionCompletion' AND object_id=?",
                &["domain-one", &begin.operation_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(complete_action_from_native_receipt(db, &begin).is_err());
        assert_eq!(
            count(db,"SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id='domain-one' AND object_type='ActionCompletion' AND object_id='opr_11111111111111111111111111111111'"),
            "{}"
        );

        // Native receipt replay checks its canonical object bytes too.
        super::super::transaction::run(db, |tx| {
            tx.write(
                "UPDATE main.gogoke_objects SET canonical_json=CAST('{}' AS BLOB) WHERE domain_id=? AND object_type='ActionNativeReceipt' AND object_id=?",
                &["domain-one", &begin.operation_id],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(complete_action_from_native_receipt(db, &begin).is_err());
        assert_eq!(
            count(db,"SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id='domain-one' AND object_type='ActionNativeReceipt' AND object_id='opr_11111111111111111111111111111111'"),
            "{}"
        );
    });
}
