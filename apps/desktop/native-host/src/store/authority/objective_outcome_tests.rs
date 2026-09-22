//! Focused fail-closed checks for typed Objective Outcome ingress.
use super::bootstrap::OwnerIssuer;
use super::objective_outcome::{
    append_objective_outcome, canonical_refs, read_objective_outcome, AppendObjectiveOutcome,
    ObjectiveEvidenceRef, ObjectiveObservationWindow, ObjectiveVersionRef,
};
use super::{
    AppendExecutionRecipe, AuthorizedTaskPackageDraft, CommitTaskContextRequirements,
    DelegationBinding, DelegationGrantInput, DelegationPrincipal, NativeSessionIdentity,
    PrepareAuthorizedTaskPackage, RecipeJsonValue, SessionLineageCommand, SessionLineageOperation,
    TaskPackageBinding, TaskPackagePrincipal,
};
use crate::root::RootLock;
use crate::store::action::apply_action_schema;
use crate::store::atomic::{apply_core_schema, commit_domain_record, DomainRecordInput, Statement};
use crate::store::context::apply_context_schema;
use crate::store::digest::content_hash;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn ref_for(kind: &str, id: &str) -> ObjectiveEvidenceRef {
    ObjectiveEvidenceRef {
        object_type: kind.into(),
        object_id: id.into(),
        object_version: "1".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)),
    }
}
fn request(domain: &str) -> AppendObjectiveOutcome {
    AppendObjectiveOutcome {
        domain_id: domain.into(),
        outcome_id: "objective-one".into(),
        revision: "1".into(),
        operation_id: "objective-op-one".into(),
        event_id: "objective-event-one".into(),
        receipt_id: "objective-receipt-one".into(),
        recorded_at: "2026-09-22T00:00:00Z".into(),
        manifest_id: "manifest-one".into(),
        manifest_version: "1".into(),
        manifest_hash: format!("sha256:{}", "b".repeat(64)),
        decision_id: "decision-one".into(),
        decision_version: "1".into(),
        decision_hash: format!("sha256:{}", "c".repeat(64)),
        action_operation_id: "action-one".into(),
        action_completion_ref: "completion-receipt-one".into(),
        result_refs: vec![ref_for("ActionCompletion", "action-one")],
        evidence_refs: vec![ref_for("DecisionRecord", "decision-one")],
        observation: ObjectiveObservationWindow {
            starts_at: "2026-09-21T00:00:00Z".into(),
            ends_at: "2026-09-22T00:00:00Z".into(),
            status: "OBSERVED".into(),
        },
        previous: None,
    }
}
fn fixture(run: impl FnOnce(&RootLock, &mut VerifiedDatabaseConnection<'_>, &OwnerIssuer)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-objective-outcome-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let dbpath = path.join("state.sqlite");
    let mut db = create_new(&root, &dbpath).unwrap();
    apply_core_schema(&mut db).unwrap();
    apply_action_schema(&mut db).unwrap();
    apply_context_schema(&mut db).unwrap();
    let owner = super::initialize_profile(&mut db, &root).unwrap();
    super::initialize_decision_capacity_schema(&mut db).unwrap();
    super::initialize_context_manifest_schema(&mut db).unwrap();
    run(&root, &mut db, &owner);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(&dbpath).unwrap();
    for suffix in ["-wal", "-shm"] {
        match std::fs::remove_file(format!("{}{suffix}", dbpath.display())) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => panic!("fixture cleanup: {e}"),
        }
    }
    std::fs::remove_dir(path).unwrap();
}
fn seed_manifest(db: &mut VerifiedDatabaseConnection<'_>, domain: &str) {
    let bytes =
        format!("{{\"domainId\":\"{domain}\",\"manifestId\":\"manifest-one\"}}").into_bytes();
    commit_domain_record(
        db,
        DomainRecordInput {
            domain_id: domain.into(),
            object_type: "ContextManifest".into(),
            object_id: "manifest-one".into(),
            object_version: "1".into(),
            object_bytes: bytes,
            native_identity: None,
            event_id: "manifest-event-one".into(),
            stream_id: "manifest-stream-one".into(),
            expected_previous_counter: None,
            counter: "0".into(),
            event_type: "ContextManifestCommitted".into(),
            occurred_at: "2026-09-22T00:00:00Z".into(),
            event_bytes: b"{}".to_vec(),
            receipt_id: "manifest-receipt-one".into(),
            operation_id: "manifest-op-one".into(),
            receipt_type: "ContextManifestCommitted".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            receipt_bytes: b"{}".to_vec(),
        },
    )
    .unwrap();
}

fn q(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
fn obj(mut fields: Vec<(&str, String)>) -> String {
    fields.sort_by_key(|x| x.0);
    format!(
        "{{{}}}",
        fields
            .into_iter()
            .map(|(k, v)| format!("\"{k}\":{v}"))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn s(k: &'static str, v: &str) -> (&'static str, String) {
    (k, q(v))
}
fn hash(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}

fn seed_real_manifest(db: &mut VerifiedDatabaseConnection<'_>) -> (String, String) {
    super::initialize_context_manifest_schema(db).unwrap();
    let snapshot = obj(vec![
        s("assemblySchema", "gogoke.context-assembly.v1"),
        s("operationId", "manifest-operation"),
        s("requestDigest", &hash('d')),
        s("principalId", "principal-owner"),
        s("sessionId", "session-worker"),
        s("bindingId", "binding-worker"),
        s("sourceEpoch", "9"),
        s("runtimeInstanceId", "runtime-one"),
        s("taskRevision", "1"),
        s("authRevision", "1"),
        s("revocationHead", "0"),
        ("partitions", "[]".into()),
        s("mode", "FIXED_SOURCE_RULES"),
        ("excluded", "[]".into()),
    ]);
    let body = obj(vec![
        s("bindingGeneration", "7"),
        s("domainId", "domain-one"),
        ("includedVersions", "[]".into()),
        s("manifestId", "manifest-one"),
        s("policyRevision", "1"),
        ("redactions", "[]".into()),
        ("requiredConstraints", "[]".into()),
        s("seatId", "seat-worker"),
        s("selectionDecisionId", "decision-one"),
        ("sourceSnapshot", snapshot),
        s("taskId", "task-one"),
    ]);
    let manifest_hash = content_hash(body.as_bytes());
    let mut fields = vec![
        s("manifestHash", &manifest_hash),
        s("bindingGeneration", "7"),
        s("domainId", "domain-one"),
        ("includedVersions", "[]".into()),
        s("manifestId", "manifest-one"),
        s("policyRevision", "1"),
        ("redactions", "[]".into()),
        ("requiredConstraints", "[]".into()),
        s("seatId", "seat-worker"),
        s("selectionDecisionId", "decision-one"),
        (
            "sourceSnapshot",
            obj(vec![
                s("assemblySchema", "gogoke.context-assembly.v1"),
                s("operationId", "manifest-operation"),
                s("requestDigest", &hash('d')),
                s("principalId", "principal-owner"),
                s("sessionId", "session-worker"),
                s("bindingId", "binding-worker"),
                s("sourceEpoch", "9"),
                s("runtimeInstanceId", "runtime-one"),
                s("taskRevision", "1"),
                s("authRevision", "1"),
                s("revocationHead", "0"),
                ("partitions", "[]".into()),
                s("mode", "FIXED_SOURCE_RULES"),
                ("excluded", "[]".into()),
            ]),
        ),
        s("taskId", "task-one"),
    ];
    // Keep UTF-16 canonical key order, matching native ContextManifest objects.
    fields.sort_by_key(|x| x.0);
    let canonical = format!(
        "{{{}}}",
        fields
            .into_iter()
            .map(|(k, v)| format!("\"{k}\":{v}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    crate::store::authority::transaction::run(db,|tx|{
        tx.write("INSERT INTO main.gogoke_context_assembly_snapshots(operation_id,principal_id,seat_id,task_id,session_id,domain_id,binding_id,binding_generation,source_epoch,runtime_instance_id,task_revision,policy_revision,auth_revision,revocation_head,selection_decision_id,manifest_id,admission_action_operation_id,admission_digest,max_content_bytes,max_candidates) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
            &["manifest-operation","principal-owner","seat-worker","task-one","session-worker","domain-one","binding-worker","7","9","runtime-one","1","1","1","0","decision-one","manifest-one","action-one",&hash('e'),"4096","64"])?;Ok(())}).unwrap();
    commit_domain_record(
        db,
        DomainRecordInput {
            domain_id: "domain-one".into(),
            object_type: "ContextManifest".into(),
            object_id: "manifest-one".into(),
            object_version: "1".into(),
            object_bytes: canonical.as_bytes().to_vec(),
            native_identity: None,
            event_id: "manifest-event".into(),
            stream_id: "gogoke.context-manifest.v1/manifest-one".into(),
            expected_previous_counter: None,
            counter: "0".into(),
            event_type: "ContextManifestCommitted".into(),
            occurred_at: "2026-09-22T00:00:00Z".into(),
            event_bytes: obj(vec![
                s("manifestHash", &manifest_hash),
                s("manifestId", "manifest-one"),
                s("operationId", "manifest-operation"),
            ])
            .into_bytes(),
            receipt_id: "manifest-receipt".into(),
            operation_id: "manifest-operation".into(),
            receipt_type: "ContextManifestCommitted".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            receipt_bytes: obj(vec![
                s("manifestHash", &manifest_hash),
                s("requestDigest", &hash('d')),
                s("schema", "gogoke.context-manifest-commit.v1"),
            ])
            .into_bytes(),
        },
    )
    .unwrap();
    (manifest_hash, content_hash(canonical.as_bytes()))
}

fn seed_atp(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer) -> (String, String) {
    super::initialize_task_context_schema(db).unwrap();
    super::initialize_authorized_task_package_schema(db).unwrap();
    super::initialize_session_lineage_schema(db).unwrap();
    super::initialize_execution_recipe_schema(db).unwrap();
    super::commit_task_context_requirements(
        db,
        &CommitTaskContextRequirements {
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
    let ceiling = super::AuthorityCeiling {
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
        + 3600000;
    let grant = super::issue_owner_delegation(
        db,
        owner,
        DelegationGrantInput {
            principal: DelegationPrincipal {
                principal_id: owner.principal_id().into(),
                project_id: "project-one".into(),
                domain_id: "domain-one".into(),
                role: "controller".into(),
                seat_id: owner.seat_id().into(),
            },
            binding: DelegationBinding {
                session_id: "session-source".into(),
                execution_id: "execution-source".into(),
                generation: "1".into(),
            },
            expires_at_epoch_ms: expiry,
            ceiling: ceiling.clone(),
        },
    )
    .unwrap();
    let draft = AuthorizedTaskPackageDraft {
        parent_grant_ref: grant.reference.grant_id.clone(),
        parent_grant_revision: grant.reference.revision.clone(),
        parent_grant_revocation_head: grant.reference.revocation_head.clone(),
        parent_policy_revision: grant.policy_revision.clone(),
        parent_seat_id: grant.principal.seat_id.clone(),
        child_ceiling: ceiling.clone(),
        action: "delegate".into(),
        route: "controller-worker".into(),
        source: TaskPackagePrincipal {
            principal_id: grant.principal.principal_id.clone(),
            project_id: "project-one".into(),
            domain_id: "domain-one".into(),
            role: "controller".into(),
        },
        target: TaskPackagePrincipal {
            principal_id: "principal-worker".into(),
            project_id: "project-one".into(),
            domain_id: "domain-one".into(),
            role: "worker".into(),
        },
        source_binding: TaskPackageBinding {
            session_id: "session-source".into(),
            execution_id: "execution-source".into(),
            generation: "1".into(),
        },
        target_binding: TaskPackageBinding {
            session_id: "session-worker".into(),
            execution_id: "execution-worker".into(),
            generation: "7".into(),
        },
        target_binding_kind: "existing".into(),
        sink: "task-package".into(),
        instruction: "bounded instruction".into(),
    };
    let package = super::prepare_authorized_task_package(
        db,
        &PrepareAuthorizedTaskPackage {
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
    super::apply_session_lineage_command(
        db,
        &SessionLineageCommand {
            operation_id: "lineage-operation".into(),
            domain_id: "domain-one".into(),
            event_id: "lineage-event".into(),
            receipt_id: "lineage-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            operation: SessionLineageOperation::NewClean {
                session_id: "session-worker".into(),
                native: NativeSessionIdentity {
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
    super::append_owner_execution_recipe(
        db,
        owner,
        &AppendExecutionRecipe {
            operation_id: "recipe-operation".into(),
            domain_id: "domain-one".into(),
            expected_previous_revision: None,
            recipe_id: "recipe-one".into(),
            seat_id: "seat-worker".into(),
            runtime_instance_id: "runtime-one".into(),
            model_ref: BTreeMap::new(),
            tool_profile: RecipeJsonValue::Null,
            isolation_profile: RecipeJsonValue::Null,
            context_manifest_id: "manifest-one".into(),
            budget_policy: RecipeJsonValue::Null,
            admission_ref: grant.reference.grant_id.clone(),
            event_id: "recipe-event".into(),
            receipt_id: "recipe-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
        },
    )
    .unwrap();
    (grant.reference.grant_id, package.package_digest)
}

fn seed_decision_action(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    grant_id: &str,
    package_digest: &str,
) -> AppendObjectiveOutcome {
    use super::action_authority::{
        begin_committed_action, complete_action_from_native_receipt,
        record_trusted_native_action_facts, record_trusted_native_action_receipt,
        ActionCompletionDisposition, BeginCommittedDisposition, TrustedActionCompletionEvidence,
        TrustedNativeActionFacts,
    };
    use super::{
        commit_decision, publish_decision_snapshot, BeginCommittedAction,
        DecisionAuthoritySnapshot, DecisionCommitInput, DurableDecisionRecord,
        PrepareActionAuthority,
    };
    let action = "opr_11111111111111111111111111111111";
    let policy = super::transaction::run(db, |tx| {
        Ok(super::catalog::current_profile(tx)?.policy_revision)
    })
    .unwrap();
    let payload = b"bounded instruction";
    let payload_digest = content_hash(payload);
    let selected_recipe = super::read_current_execution_recipe(db, owner, "domain-one", "recipe-one")
        .unwrap()
        .unwrap();
    let values = [
        "domain-one",
        action,
        "package-operation",
        package_digest,
        grant_id,
        "task-one",
        "1",
        "recipe-one",
        &selected_recipe.recipe.revision,
        &selected_recipe.content_hash,
        "session-worker",
        "manifest-one",
        &policy,
        "queue",
        "work",
        &payload_digest,
    ];
    let mut preimage = String::from("gogoke.action-intent.v1|");
    for value in values {
        preimage.push_str(&value.len().to_string());
        preimage.push(':');
        preimage.push_str(value);
    }
    let action_digest = content_hash(preimage.as_bytes());
    let decision = DurableDecisionRecord {
        operation_id: "decision-op".into(),
        scenario_id: "DF02".into(),
        family: "RESOURCE_SELECTION".into(),
        state_view_hash: hash('a'),
        candidate_hash: hash('b'),
        question_version: "1".into(),
        rubric_version: "1".into(),
        model_requested: None,
        model_resolved: Some("fixture-fake".into()),
        task_revision: "1".into(),
        policy_revision: policy.clone(),
        capability_revision: "1".into(),
        binding_generation: "7".into(),
        backend_kind: "FAKE".into(),
        choice: "candidate-one".into(),
        reason: "QUALIFIED_BOUNDED_SELECTION".into(),
        budget_units: 1,
        deadline_epoch_ms: 1000,
    };
    let snapshot = DecisionAuthoritySnapshot {
        operation_id: decision.operation_id.clone(),
        candidate_id: decision.choice.clone(),
        state_view_hash: decision.state_view_hash.clone(),
        candidate_hash: decision.candidate_hash.clone(),
        task_revision: "1".into(),
        policy_revision: policy.clone(),
        capability_revision: "1".into(),
        binding_id: "binding-worker".into(),
        binding_generation: "7".into(),
        auth_revision: policy.clone(),
        resource_ref: "pool-one".into(),
        resource_revision: "1".into(),
        capacity_total: 2,
        action_operation_id: action.into(),
        action_digest: action_digest.clone(),
    };
    publish_decision_snapshot(db, &snapshot).unwrap();
    let committed = commit_decision(
        db,
        &DecisionCommitInput {
            domain_id: "domain-one".into(),
            decision_id: "decision-one".into(),
            event_id: "decision-event".into(),
            receipt_id: "decision-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            record: decision,
            resource_reservation_ref: "capacity-lease-one".into(),
            action_intent_ref: action.into(),
            required_capacity_units: 1,
        },
    )
    .unwrap();
    let action_request = PrepareActionAuthority {
            domain_id: "domain-one".into(),
            parent_grant_ref: grant_id.into(),
            package_operation_id: "package-operation".into(),
            task_id: "task-one".into(),
            recipe_id: "recipe-one".into(),
            session_id: "session-worker".into(),
            context_manifest_id: "manifest-one".into(),
            action_operation_id: action.into(),
            reservation_id: "action-reservation".into(),
            action_kind: "queue".into(),
            lane: "work".into(),
            payload: payload.to_vec(),
        };
    let prepared = super::prepare_action_authority(db, &action_request).unwrap();
    assert_eq!(prepared.semantic_digest, action_digest);
    let manifest_hash = super::action_authority::tests::persist_manifest(
        db,
        owner,
        &action_request,
        &prepared.semantic_digest,
    );
    let recipe = super::read_current_execution_recipe(db, owner, "domain-one", "recipe-one")
        .unwrap()
        .unwrap();
    record_trusted_native_action_facts(
        db,
        &TrustedNativeActionFacts {
            domain_id: "domain-one".into(),
            operation_id: action.into(),
            expected_previous_revision: None,
            session_id: "session-worker".into(),
            binding_id: "binding-worker".into(),
            generation: "7".into(),
            source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
            model_ref_digest: super::execution_recipe::model_ref_digest(&recipe.recipe).unwrap(),
            capability_revision: "1".into(),
            context_manifest_id: "manifest-one".into(),
            context_manifest_hash: manifest_hash.clone(),
            admission_ref: grant_id.into(),
            admission_revision: "1".into(),
            expires_at_epoch_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                + 60000,
        },
    )
    .unwrap();
    let begin = BeginCommittedAction {
        domain_id: "domain-one".into(),
        operation_id: action.into(),
        reservation_id: "action-reservation".into(),
    };
    let BeginCommittedDisposition::Granted {
        attempt_id,
        send_authority,
    } = begin_committed_action(db, &begin).unwrap()
    else {
        panic!("fixture begin must commit")
    };
    record_trusted_native_action_receipt(
        db,
        &TrustedActionCompletionEvidence {
            domain_id: "domain-one".into(),
            operation_id: action.into(),
            reservation_id: "action-reservation".into(),
            semantic_digest: action_digest.clone(),
            attempt_id,
            send_authority,
            binding_id: "binding-worker".into(),
            generation: "7".into(),
            source_epoch: "9".into(),
            runtime_instance_id: "runtime-one".into(),
            native_request_id: "native-request-one".into(),
            native_session_id: "native-worker".into(),
            trusted_receipt_ref: "native-receipt-one".into(),
            evidence_hash: hash('f'),
            disposition: ActionCompletionDisposition::Completed,
        },
    )
    .unwrap();
    let completed = complete_action_from_native_receipt(db, &begin).unwrap();
    assert_eq!(completed.disposition, "completed");
    let completion_hash = {
        let st=Statement::prepare(db.as_ptr(),"SELECT content_hash FROM main.gogoke_objects WHERE domain_id='domain-one' AND object_type='ActionCompletion' AND object_id=? AND object_version='1'").unwrap();
        st.bind_text(1, action).unwrap();
        assert!(st.step_row().unwrap());
        st.column_text(0).unwrap()
    };
    let mut input = request("domain-one");
    input.action_operation_id = action.into();
    input.action_completion_ref = completed.receipt_id;
    input.manifest_hash = manifest_hash;
    input.decision_hash = committed.replay.decision_content_hash;
    input.result_refs = vec![ObjectiveEvidenceRef {
        object_type: "ActionCompletion".into(),
        object_id: action.into(),
        object_version: "1".into(),
        content_hash: completion_hash,
    }];
    input.evidence_refs = vec![ObjectiveEvidenceRef {
        object_type: "DecisionRecord".into(),
        object_id: "decision-one".into(),
        object_version: "1".into(),
        content_hash: input.decision_hash.clone(),
    }];
    input
}

#[test]
fn objective_evidence_rejects_worker_assertions_and_duplicate_refs() {
    assert!(canonical_refs(&[ref_for("WorkerReport", "worker-result")]).is_err());
    let same = ref_for("DecisionRecord", "decision-one");
    assert!(canonical_refs(&[same.clone(), same]).is_err());
}

#[test]
fn objective_outcome_fails_closed_when_manifest_is_missing_or_cross_domain() {
    fixture(|_, db, _| {
        assert!(append_objective_outcome(db, &request("domain-one")).is_err());
        seed_manifest(db, "domain-two");
        assert!(append_objective_outcome(db, &request("domain-one")).is_err());
    });
}

#[test]
fn objective_outcome_rejects_manifest_hash_mismatch() {
    fixture(|_, db, _| {
        seed_manifest(db, "domain-one");
        let mut input = request("domain-one");
        input.manifest_hash = content_hash(b"different");
        assert!(append_objective_outcome(db, &input).is_err());
    });
}

#[test]
fn objective_append_read_replay_correction_and_tamper_rejection() {
    fixture(|_, db, owner| {
        let (grant_id, package_digest) = seed_atp(db, owner);
        let first = seed_decision_action(
            db,
            owner,
            &grant_id,
            &package_digest,
        );
        let manifest_hash = first.manifest_hash.clone();
        super::transaction::run(db, |tx| {
            super::objective_outcome::validate_manifest(
                tx,
                "domain-one",
                "manifest-one",
                "1",
                &manifest_hash,
            )
        })
        .unwrap_or_else(|e| panic!("manifest validation: {e:?}"));
        super::transaction::run(db,|tx|{let rows=tx.query("SELECT operation_id FROM main.gogoke_receipts WHERE domain_id='domain-one' AND object_type='DecisionRecord' AND object_id='decision-one' AND object_version='1' AND receipt_type='DecisionApplied'",&[],1)?;super::decision_replay::read_in_transaction(tx,"domain-one",&rows[0][0]).map(|_|())}).unwrap_or_else(|e|panic!("decision validation: {e:?}"));
        super::transaction::run(db, |tx| {
            super::objective_outcome::completion(
                tx,
                "domain-one",
                "opr_11111111111111111111111111111111",
                &first.action_completion_ref,
            )
            .map(|_| ())
        })
        .unwrap_or_else(|e| panic!("completion direct: {e:?}"));
        super::transaction::run(db, |tx| {
            super::objective_outcome::validate_record_ref(tx, "domain-one", &first.result_refs[0])
        })
        .unwrap_or_else(|e| panic!("completion evidence validation: {e:?}"));
        super::transaction::run(db, |tx| {
            super::objective_outcome::validate_record_ref(tx, "domain-one", &first.evidence_refs[0])
        })
        .unwrap_or_else(|e| panic!("decision evidence validation: {e:?}"));
        super::transaction::run(db, |tx| {
            super::task_package::read_authorized_task_package_in_transaction(
                tx,
                "domain-one",
                "package-operation",
            )?
            .map(|_| ())
            .ok_or(crate::store::orchestration::OrchestrationError::AccessDenied)
        })
        .unwrap_or_else(|e| panic!("ATP validation: {e:?}"));
        super::transaction::run(db, |tx| {
            super::objective_outcome::completion(
                tx,
                "domain-one",
                "opr_11111111111111111111111111111111",
                &first.action_completion_ref,
            )
            .map(|_| ())
        })
        .unwrap_or_else(|e| panic!("completion validation: {e:?}"));
        let committed = append_objective_outcome(db, &first).unwrap();
        assert_eq!(committed.disposition, "COMMITTED");
        let first_read = read_objective_outcome(db, "domain-one", "objective-one", "1").unwrap();
        assert_eq!(first_read.content_hash, committed.object_hash);
        assert_eq!(
            append_objective_outcome(db, &first).unwrap().disposition,
            "RECONCILED"
        );
        let mut correction = first.clone();
        correction.revision = "2".into();
        correction.operation_id = "objective-op-two".into();
        correction.event_id = "objective-event-two".into();
        correction.receipt_id = "objective-receipt-two".into();
        correction.observation.status = "CENSORED".into();
        correction.previous = Some(ObjectiveVersionRef {
            revision: "1".into(),
            content_hash: first_read.content_hash.clone(),
        });
        append_objective_outcome(db, &correction).unwrap();
        assert_eq!(
            read_objective_outcome(db, "domain-one", "objective-one", "1")
                .unwrap()
                .canonical_outcome,
            first_read.canonical_outcome
        );
        assert_eq!(
            read_objective_outcome(db, "domain-one", "objective-one", "2")
                .unwrap()
                .revision,
            "2"
        );
        let mut tampered = first.clone();
        tampered.action_completion_ref = "wrong-completion".into();
        assert!(append_objective_outcome(db, &tampered).is_err());
        let mut forged = first.clone();
        forged.evidence_refs[0].content_hash = hash('9');
        assert!(append_objective_outcome(db, &forged).is_err());
        let mut cross = first.clone();
        cross.domain_id = "domain-two".into();
        assert!(append_objective_outcome(db, &cross).is_err());
    });
}
