use super::{
    append_dream_proposal, append_dream_run, append_evaluation, read_dream_proposal,
    read_dream_run, AppendDreamProposal, AppendDreamRun, DreamAllowedChange, DreamBudgetLease,
    DreamEvaluationRef, DreamObjectRef, DreamVersionRef, EvaluationEvidenceRef,
    EvaluationOutcomeRef, AppendEvaluation, DecisionAuthoritySnapshot, DecisionCommitInput,
    DurableDecisionRecord,
};
use crate::root::RootLock;
use crate::store::action::{apply_action_schema, reserve_action, ActionReservation};
use crate::store::atomic::{apply_core_schema, commit_domain_record, DomainRecordInput};
use crate::store::digest::content_hash;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn digest(c: char) -> String { format!("sha256:{}", c.to_string().repeat(64)) }
fn quote(v: &str) -> String { format!("\"{}\"", v.replace('\\', "\\\\").replace('\"', "\\\"")) }
fn object(mut fields: Vec<(&str, String)>) -> String {
    fields.sort_by_key(|v| v.0);
    format!("{{{}}}", fields.into_iter().map(|(k,v)|format!("\"{k}\":{v}")).collect::<Vec<_>>().join(","))
}
fn s(k: &'static str, v: &str) -> (&'static str, String) { (k, quote(v)) }

fn seed_record(db: &mut VerifiedDatabaseConnection<'_>, domain: &str, kind: &str, id: &str, version: &str, event_type: &str, receipt_type: &str, bytes: Vec<u8>, event_id: &str, receipt_id: &str, operation_id: &str) -> String {
    let object_hash = content_hash(&bytes);
    let event = object(vec![s("contentHash", &object_hash), s("objectId", id), ("previousHash", "null".into()), s("revision", version)]);
    let receipt = object(vec![s("schema", receipt_type), s("sourceIdentity", "PRODUCT_AUTHORITY")]);
    commit_domain_record(db, DomainRecordInput {
        domain_id: domain.into(), object_type: kind.into(), object_id: id.into(), object_version: version.into(), object_bytes: bytes, native_identity: None,
        event_id: event_id.into(), stream_id: format!("test.{kind}/{id}"), expected_previous_counter: None, counter: "0".into(),
        event_type: event_type.into(), occurred_at: "2026-09-22T00:00:00Z".into(), event_bytes: event.into_bytes(),
        receipt_id: receipt_id.into(), operation_id: operation_id.into(), receipt_type: receipt_type.into(), recorded_at: "2026-09-22T00:00:00Z".into(), receipt_bytes: receipt.into_bytes(),
    }).unwrap();
    object_hash
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, DreamObjectRef, DreamObjectRef, DreamEvaluationRef, DreamBudgetLease)) {
    let _guard = route_b_test_guard();
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-dream-positive-{}-{n}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let dbpath = path.join("state.sqlite");
    let mut db = create_new(&root, &dbpath).unwrap();
    apply_core_schema(&mut db).unwrap();
    apply_action_schema(&mut db).unwrap();
    super::initialize_profile(&mut db, &root).unwrap();
    super::initialize_decision_capacity_schema(&mut db).unwrap();
    super::initialize_context_manifest_schema(&mut db).unwrap();
    super::initialize_execution_recipe_schema(&mut db).unwrap();

    let action = "opr_11111111111111111111111111111111";
    let record = DurableDecisionRecord {
        operation_id: "decision-op".into(), scenario_id: "DF02".into(), family: "RESOURCE_SELECTION".into(),
        state_view_hash: digest('a'), candidate_hash: digest('b'), question_version: "1".into(), rubric_version: "1".into(),
        model_requested: None, model_resolved: Some("fixture-model".into()), task_revision: "1".into(), policy_revision: "1".into(),
        capability_revision: "3".into(), binding_generation: "7".into(), backend_kind: "FAKE".into(), choice: "candidate-one".into(),
        reason: "QUALIFIED_BOUNDED_SELECTION".into(), budget_units: 1, deadline_epoch_ms: 1000,
    };
    let snapshot = DecisionAuthoritySnapshot {
        operation_id: record.operation_id.clone(), candidate_id: record.choice.clone(), state_view_hash: record.state_view_hash.clone(), candidate_hash: record.candidate_hash.clone(),
        task_revision: record.task_revision.clone(), policy_revision: record.policy_revision.clone(), capability_revision: record.capability_revision.clone(),
        binding_id: "binding-one".into(), binding_generation: record.binding_generation.clone(), auth_revision: "2".into(), resource_ref: "pool-one".into(), resource_revision: "5".into(),
        capacity_total: 2, action_operation_id: action.into(), action_digest: digest('c'),
    };
    reserve_action(&mut db, ActionReservation {
        operation_id: action.into(), semantic_digest: snapshot.action_digest.clone(), reservation_id: "action-reservation".into(), binding_id: snapshot.binding_id.clone(),
        session_id: "session-one".into(), execution_id: "execution-one".into(), runtime_instance_id: "runtime-one".into(), profile_id: "profile-one".into(),
        auth_revision: "1".into(), generation: "7".into(), lane: "work".into(), action_kind: "queue".into(), payload_hex: "7b7d".into(),
        commitment: crate::store::action::test_commitment("session-one", "execution-one", "7"),
    }).unwrap();
    super::publish_decision_snapshot(&mut db, &snapshot).unwrap();
    let committed = super::commit_decision(&mut db, &DecisionCommitInput {
        domain_id: "domain-one".into(), decision_id: "decision-one".into(), event_id: "decision-event".into(), receipt_id: "decision-receipt".into(),
        recorded_at: "2026-09-22T00:00:00Z".into(), record, resource_reservation_ref: "capacity-lease-one".into(), action_intent_ref: action.into(), required_capacity_units: 1,
    }).unwrap();

    let manifest_body = object(vec![s("domainId", "domain-one"), s("labelSource", "CONTEXT_MANIFEST"), s("manifestId", "manifest-one"), s("revision", "1")]);
    let manifest_hash = seed_record(&mut db, "domain-one", "ContextManifest", "manifest-one", "1", "ContextManifestCommitted", "ContextManifestCommitted", manifest_body.into_bytes(), "manifest-event", "manifest-receipt", "manifest-op");
    super::transaction::run(&mut db, |tx| tx.write(
        "INSERT INTO gogoke_context_assembly_snapshots(operation_id,principal_id,seat_id,task_id,session_id,domain_id,binding_id,binding_generation,source_epoch,runtime_instance_id,task_revision,policy_revision,auth_revision,revocation_head,selection_decision_id,manifest_id,admission_action_operation_id,admission_digest,max_content_bytes,max_candidates) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        &["manifest-op","principal-one","seat-one","task-one","session-one","domain-one","binding-one","7","epoch-one","runtime-one","1","1","1","revocation-one","decision-one","manifest-one",action,&snapshot.action_digest,"10000","32"])).unwrap();
    let manifest_ref = DreamObjectRef { object_type: "ContextManifest".into(), object_id: "manifest-one".into(), revision: "1".into(), content_hash: manifest_hash };

    let outcome_body = object(vec![s("decisionHash", &committed.replay.decision_content_hash), s("decisionId", "decision-one"), s("decisionRevision", "1"), s("domainId", "domain-one"),
        s("labelSource", "OBJECTIVE"), s("manifestHash", &manifest_ref.content_hash), s("manifestId", "manifest-one"), s("manifestVersion", "1"), s("outcomeId", "objective-one"), s("revision", "1")]);
    let outcome_hash = content_hash(outcome_body.as_bytes());
    let outcome_event = object(vec![s("contentHash", &outcome_hash), s("outcomeId", "objective-one"), ("previousHash", "null".into()), s("revision", "1")]);
    commit_domain_record(&mut db, DomainRecordInput {
        domain_id: "domain-one".into(), object_type: "OutcomeRecord".into(), object_id: "objective-one".into(), object_version: "1".into(), object_bytes: outcome_body.into_bytes(), native_identity: None,
        event_id: "objective-event".into(), stream_id: "objective-stream".into(), expected_previous_counter: None, counter: "0".into(), event_type: "ObjectiveOutcomeAppended".into(), occurred_at: "2026-09-22T00:00:00Z".into(), event_bytes: outcome_event.into_bytes(),
        receipt_id: "objective-receipt".into(), operation_id: "objective-operation".into(), receipt_type: "ObjectiveOutcomeAppended".into(), recorded_at: "2026-09-22T00:00:00Z".into(), receipt_bytes: br#"{"schema":"gogoke.objective-outcome.v1"}"#.to_vec(),
    }).unwrap();

    let evaluation = AppendEvaluation {
        domain_id: "domain-one".into(), evaluation_id: "evaluation-one".into(), revision: "1".into(), operation_id: "evaluation-op".into(), event_id: "evaluation-event".into(), receipt_id: "evaluation-receipt".into(), recorded_at: "2026-09-22T00:00:00Z".into(), source_identity: "fixture-scorer".into(),
        outcome_refs: vec![EvaluationOutcomeRef { outcome_id: "objective-one".into(), revision: "1".into(), content_hash: outcome_hash }], decision_family: "RESOURCE_SELECTION".into(), scorer_version: "scorer-v1".into(), rubric_version: "rubric-v1".into(), calibration_key: "fixture-calibration".into(), calibration_version: "1".into(), dataset_namespace: "fixture-data".into(), dataset_split: "heldout".into(),
        evidence_refs: vec![EvaluationEvidenceRef { object_type: "DecisionRecord".into(), object_id: "decision-one".into(), object_version: "1".into(), content_hash: committed.replay.decision_content_hash }], metrics_hash: digest('e'), safety_status: "CLEAR".into(), privacy_status: "REDACTED".into(), previous: None,
    };
    let evaluation_receipt = append_evaluation(&mut db, &evaluation).unwrap();
    let evaluation_ref = DreamEvaluationRef { evaluation_id: "evaluation-one".into(), revision: "1".into(), content_hash: evaluation_receipt.object_hash };

    let recipe_body = object(vec![s("recipeId", "recipe-one"), s("revision", "1"), s("domainId", "domain-one"), s("contextManifestId", "manifest-one"), s("labelSource", "EXECUTION_RECIPE")]);
    let recipe_hash = seed_record(&mut db, "domain-one", "ExecutionRecipe", "recipe-one", "1", "ExecutionRecipeVersionCommitted", "ExecutionRecipeVersionCommitted", recipe_body.into_bytes(), "recipe-event", "recipe-receipt", "recipe-op");
    super::transaction::run(&mut db, |tx| {
        tx.write("INSERT INTO gogoke_execution_recipe_heads(domain_id,recipe_id,object_type,recipe_revision,content_hash,updated_at) VALUES(?,?, 'ExecutionRecipe',?,?,?)", &["domain-one","recipe-one","1",&recipe_hash,"2026-09-22T00:00:00Z"])
    }).unwrap();
    let recipe_ref = DreamObjectRef { object_type: "ExecutionRecipe".into(), object_id: "recipe-one".into(), revision: "1".into(), content_hash: recipe_hash };
    let budget = DreamBudgetLease { lease_ref: "capacity-lease-one".into(), operation_id: "decision-op".into(), resource_ref: "pool-one".into(), resource_revision: "5".into(), units: 1 };

    run(&mut db, manifest_ref, recipe_ref, evaluation_ref, budget);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(&dbpath).unwrap();
    for suffix in ["-wal", "-shm"] { let _ = std::fs::remove_file(format!("{}{suffix}", dbpath.display())); }
    std::fs::remove_dir(path).unwrap();
}

#[test]
fn durable_dream_run_proposal_replay_correction_and_currentness() {
    fixture(|db, manifest, recipe, evaluation, budget| {
        let run = AppendDreamRun {
            domain_id: "domain-one".into(), run_id: "dream-run-one".into(), revision: "1".into(), operation_id: "dream-run-op".into(), event_id: "dream-run-event".into(), receipt_id: "dream-run-receipt".into(), recorded_at: "2026-09-22T00:00:00Z".into(), source_identity: "dream-fixture".into(),
            input_snapshot: manifest.clone(), dataset_namespace: "fixture-data".into(), dataset_split: "heldout".into(), dataset_split_hash: digest('d'), recipe_ref: recipe.clone(), budget_lease: budget.clone(), evaluation_refs: vec![evaluation.clone()], previous: None,
        };
        let committed = append_dream_run(db, &run).unwrap();
        assert_eq!(append_dream_run(db, &run).unwrap().disposition, "RECONCILED");
        let read = read_dream_run(db, "domain-one", "dream-run-one", "1").unwrap();
        assert_eq!(read.content_hash, committed.object_hash);

        let rollback_json = object(vec![s("domainId", "domain-one"), s("labelSource", "ROLLBACK_PLAN"), s("planId", "rollback-one")]);
        let rollback_hash = seed_record(db, "domain-one", "RollbackPlan", "rollback-one", "1", "RollbackPlanRecorded", "RollbackPlanRecorded", rollback_json.into_bytes(), "rollback-event", "rollback-receipt", "rollback-op");
        let proposal = AppendDreamProposal {
            domain_id: "domain-one".into(), proposal_id: "dream-proposal-one".into(), revision: "1".into(), operation_id: "dream-proposal-op".into(), event_id: "dream-proposal-event".into(), receipt_id: "dream-proposal-receipt".into(), recorded_at: "2026-09-22T00:00:00Z".into(), source_identity: "dream-fixture".into(),
            run_ref: DreamObjectRef { object_type: "DreamRun".into(), object_id: "dream-run-one".into(), revision: "1".into(), content_hash: committed.object_hash.clone() }, candidate_kind: "PARAMETER_TUNING".into(), before_hash: digest('f'), after_hash: digest('a'),
            allowed_change_set: vec![DreamAllowedChange { key: "candidate.parameter.temperature".into(), before_hash: digest('f'), after_hash: digest('a') }], heldout_receipt: Some(evaluation.clone()), rollback_ref: DreamObjectRef { object_type: "RollbackPlan".into(), object_id: "rollback-one".into(), revision: "1".into(), content_hash: rollback_hash },
            base_policy_revision: "1".into(), namespace: "test/s1r4".into(), test_only: true, activation_grant: None, previous: None,
        };
        let proposal_receipt = append_dream_proposal(db, &proposal).unwrap();
        assert_eq!(append_dream_proposal(db, &proposal).unwrap().disposition, "RECONCILED");
        assert_eq!(read_dream_proposal(db, "domain-one", "dream-proposal-one", "1").unwrap().content_hash, proposal_receipt.object_hash);

        let mut correction = run.clone();
        correction.revision = "2".into(); correction.operation_id = "dream-run-op-2".into(); correction.event_id = "dream-run-event-2".into(); correction.receipt_id = "dream-run-receipt-2".into();
        correction.previous = Some(DreamVersionRef { revision: "1".into(), content_hash: committed.object_hash.clone() });
        correction.dataset_split_hash = digest('e');
        append_dream_run(db, &correction).unwrap();
        assert_eq!(read_dream_run(db, "domain-one", "dream-run-one", "2").unwrap().revision, "2");

        let mut stale_budget = run.clone(); stale_budget.run_id = "bad-budget-run".into(); stale_budget.operation_id = "bad-budget-op".into(); stale_budget.event_id = "bad-budget-event".into(); stale_budget.receipt_id = "bad-budget-receipt".into(); stale_budget.budget_lease.units = 2;
        assert!(append_dream_run(db, &stale_budget).is_err());
        let mut cross_domain = run.clone(); cross_domain.domain_id = "domain-two".into(); cross_domain.run_id = "cross-run".into(); cross_domain.operation_id = "cross-op".into(); cross_domain.event_id = "cross-event".into(); cross_domain.receipt_id = "cross-receipt".into();
        assert!(append_dream_run(db, &cross_domain).is_err());
        let mut stale_recipe = run.clone(); stale_recipe.run_id = "stale-recipe".into(); stale_recipe.operation_id = "stale-recipe-op".into(); stale_recipe.event_id = "stale-recipe-event".into(); stale_recipe.receipt_id = "stale-recipe-receipt".into(); stale_recipe.recipe_ref.content_hash = digest('9');
        assert!(append_dream_run(db, &stale_recipe).is_err());
        let mut stale_policy = proposal.clone(); stale_policy.proposal_id = "stale-policy".into(); stale_policy.operation_id = "stale-policy-op".into(); stale_policy.event_id = "stale-policy-event".into(); stale_policy.receipt_id = "stale-policy-receipt".into(); stale_policy.base_policy_revision = "2".into();
        assert!(append_dream_proposal(db, &stale_policy).is_err());
        let mut activation = proposal.clone(); activation.proposal_id = "activation-attempt".into(); activation.operation_id = "activation-op".into(); activation.event_id = "activation-event".into(); activation.receipt_id = "activation-receipt".into(); activation.activation_grant = Some("grant".into());
        assert!(append_dream_proposal(db, &activation).is_err());
    });
}

#[test]
fn dream_rejects_budget_lease_from_unrelated_foreign_domain_decision() {
    fixture(|db, manifest, recipe, evaluation, _budget| {
        let action = "opr_22222222222222222222222222222222";
        let record = DurableDecisionRecord {
            operation_id: "foreign-decision-op".into(), scenario_id: "DF02".into(), family: "RESOURCE_SELECTION".into(),
            state_view_hash: digest('1'), candidate_hash: digest('2'), question_version: "1".into(), rubric_version: "1".into(),
            model_requested: None, model_resolved: Some("fixture-model".into()), task_revision: "1".into(), policy_revision: "1".into(),
            capability_revision: "3".into(), binding_generation: "7".into(), backend_kind: "FAKE".into(), choice: "foreign-candidate".into(),
            reason: "QUALIFIED_BOUNDED_SELECTION".into(), budget_units: 1, deadline_epoch_ms: 1000,
        };
        let snapshot = DecisionAuthoritySnapshot {
            operation_id: record.operation_id.clone(), candidate_id: record.choice.clone(), state_view_hash: record.state_view_hash.clone(),
            candidate_hash: record.candidate_hash.clone(), task_revision: record.task_revision.clone(), policy_revision: record.policy_revision.clone(),
            capability_revision: record.capability_revision.clone(), binding_id: "foreign-binding".into(), binding_generation: record.binding_generation.clone(),
            auth_revision: "2".into(), resource_ref: "foreign-pool".into(), resource_revision: "1".into(), capacity_total: 1,
            action_operation_id: action.into(), action_digest: digest('3'),
        };
        reserve_action(db, ActionReservation {
            operation_id: action.into(), semantic_digest: snapshot.action_digest.clone(), reservation_id: "foreign-action-reservation".into(),
            binding_id: snapshot.binding_id.clone(), session_id: "foreign-session".into(), execution_id: "foreign-execution".into(),
            runtime_instance_id: "foreign-runtime".into(), profile_id: "profile-one".into(), auth_revision: "1".into(), generation: "7".into(),
            lane: "work".into(), action_kind: "queue".into(), payload_hex: "7b7d".into(),
            commitment: crate::store::action::test_commitment("foreign-session", "foreign-execution", "7"),
        }).unwrap();
        super::publish_decision_snapshot(db, &snapshot).unwrap();
        super::commit_decision(db, &DecisionCommitInput {
            domain_id: "domain-two".into(), decision_id: "foreign-decision".into(), event_id: "foreign-decision-event".into(),
            receipt_id: "foreign-decision-receipt".into(), recorded_at: "2026-09-22T00:00:00Z".into(), record,
            resource_reservation_ref: "foreign-capacity-lease".into(), action_intent_ref: action.into(), required_capacity_units: 1,
        }).unwrap();
        let run = AppendDreamRun {
            domain_id: "domain-one".into(), run_id: "foreign-budget-run".into(), revision: "1".into(), operation_id: "foreign-budget-run-op".into(),
            event_id: "foreign-budget-run-event".into(), receipt_id: "foreign-budget-run-receipt".into(), recorded_at: "2026-09-22T00:00:00Z".into(),
            source_identity: "dream-fixture".into(), input_snapshot: manifest, dataset_namespace: "fixture-data".into(), dataset_split: "heldout".into(),
            dataset_split_hash: digest('4'), recipe_ref: recipe,
            budget_lease: DreamBudgetLease { lease_ref: "foreign-capacity-lease".into(), operation_id: "foreign-decision-op".into(), resource_ref: "foreign-pool".into(), resource_revision: "1".into(), units: 1 },
            evaluation_refs: vec![evaluation], previous: None,
        };
        assert!(append_dream_run(db, &run).is_err(), "foreign-domain budget lease was accepted");
    });
}
