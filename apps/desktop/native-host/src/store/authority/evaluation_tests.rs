//! Durable Evaluation receipts consume current, typed Objective/Decision rows.
use super::evaluation::{
    append_evaluation, read_evaluation, AppendEvaluation, EvaluationEvidenceRef,
    EvaluationOutcomeRef,
};
use super::{DecisionAuthoritySnapshot, DecisionCommitInput, DurableDecisionRecord};
use crate::root::RootLock;
use crate::store::action::{apply_action_schema, reserve_action, ActionReservation};
use crate::store::atomic::{apply_core_schema, commit_domain_record, DomainRecordInput};
use crate::store::digest::content_hash;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn digest(c: char) -> String {
    format!("sha256:{}", c.to_string().repeat(64))
}
fn quote(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}
fn object(mut fields: Vec<(&str, String)>) -> String {
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
    (k, quote(v))
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, String)) {
    let _guard = route_b_test_guard();
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-evaluation-{}-{n}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let dbpath = path.join("state.sqlite");
    let mut db = create_new(&root, &dbpath).unwrap();
    apply_core_schema(&mut db).unwrap();
    apply_action_schema(&mut db).unwrap();
    super::initialize_profile(&mut db, &root).unwrap();
    super::initialize_decision_capacity_schema(&mut db).unwrap();
    let action = "opr_11111111111111111111111111111111";
    let record = DurableDecisionRecord {
        operation_id: "decision-op".into(),
        scenario_id: "DF02".into(),
        family: "RESOURCE_SELECTION".into(),
        state_view_hash: digest('a'),
        candidate_hash: digest('b'),
        question_version: "1".into(),
        rubric_version: "1".into(),
        model_requested: None,
        model_resolved: Some("fake-v1".into()),
        task_revision: "1".into(),
        policy_revision: "1".into(),
        capability_revision: "3".into(),
        binding_generation: "7".into(),
        backend_kind: "FAKE".into(),
        choice: "candidate-one".into(),
        reason: "QUALIFIED_BOUNDED_SELECTION".into(),
        budget_units: 1,
        deadline_epoch_ms: 1000,
    };
    let snapshot = DecisionAuthoritySnapshot {
        operation_id: record.operation_id.clone(),
        candidate_id: record.choice.clone(),
        state_view_hash: record.state_view_hash.clone(),
        candidate_hash: record.candidate_hash.clone(),
        task_revision: record.task_revision.clone(),
        policy_revision: record.policy_revision.clone(),
        capability_revision: record.capability_revision.clone(),
        binding_id: "binding-one".into(),
        binding_generation: record.binding_generation.clone(),
        auth_revision: "2".into(),
        resource_ref: "pool-one".into(),
        resource_revision: "5".into(),
        capacity_total: 2,
        action_operation_id: action.into(),
        action_digest: digest('c'),
    };
    reserve_action(
        &mut db,
        ActionReservation {
            operation_id: action.into(),
            semantic_digest: snapshot.action_digest.clone(),
            reservation_id: "action-reservation".into(),
            binding_id: snapshot.binding_id.clone(),
            session_id: "session-one".into(),
            execution_id: "execution-one".into(),
            runtime_instance_id: "runtime-one".into(),
            profile_id: "profile-one".into(),
            auth_revision: "1".into(),
            generation: "7".into(),
            lane: "work".into(),
            action_kind: "queue".into(),
            payload_hex: "7b7d".into(),
            commitment: crate::store::action::test_commitment("session-one", "execution-one", "7"),
        },
    )
    .unwrap();
    super::publish_decision_snapshot(&mut db, &snapshot).unwrap();
    let committed = super::commit_decision(
        &mut db,
        &DecisionCommitInput {
            domain_id: "domain-one".into(),
            decision_id: "decision-one".into(),
            event_id: "decision-event".into(),
            receipt_id: "decision-receipt".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            record,
            resource_reservation_ref: "capacity-lease-one".into(),
            action_intent_ref: action.into(),
            required_capacity_units: 1,
        },
    )
    .unwrap();
    let outcome = object(vec![
        s("decisionHash", &committed.replay.decision_content_hash),
        s("decisionId", "decision-one"),
        s("decisionRevision", "1"),
        s("domainId", "domain-one"),
        s("labelSource", "OBJECTIVE"),
        s("outcomeId", "objective-one"),
        s("revision", "1"),
    ]);
    let outcome_hash = content_hash(outcome.as_bytes());
    let outcome_event = object(vec![
        s("contentHash", &outcome_hash),
        s("outcomeId", "objective-one"),
        ("previousHash", "null".into()),
        s("revision", "1"),
    ]);
    commit_domain_record(
        &mut db,
        DomainRecordInput {
            domain_id: "domain-one".into(),
            object_type: "OutcomeRecord".into(),
            object_id: "objective-one".into(),
            object_version: "1".into(),
            object_bytes: outcome.into_bytes(),
            native_identity: None,
            event_id: "objective-event".into(),
            stream_id: "objective-stream".into(),
            expected_previous_counter: None,
            counter: "0".into(),
            event_type: "ObjectiveOutcomeAppended".into(),
            occurred_at: "2026-09-22T00:00:00Z".into(),
            event_bytes: outcome_event.into_bytes(),
            receipt_id: "objective-receipt".into(),
            operation_id: "objective-operation".into(),
            receipt_type: "ObjectiveOutcomeAppended".into(),
            recorded_at: "2026-09-22T00:00:00Z".into(),
            receipt_bytes: br#"{"schema":"gogoke.objective-outcome.v1"}"#.to_vec(),
        },
    )
    .unwrap();
    run(&mut db, committed.replay.decision_content_hash);
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
fn request(decision_hash: &str) -> AppendEvaluation {
    AppendEvaluation {
        domain_id: "domain-one".into(),
        evaluation_id: "evaluation-one".into(),
        revision: "1".into(),
        operation_id: "evaluation-operation".into(),
        event_id: "evaluation-event".into(),
        receipt_id: "evaluation-receipt".into(),
        recorded_at: "2026-09-22T00:00:00Z".into(),
        source_identity: "native-scorer".into(),
        outcome_refs: vec![EvaluationOutcomeRef {
            outcome_id: "objective-one".into(),
            revision: "1".into(),
            content_hash: outcome_hash(decision_hash),
        }],
        decision_family: "RESOURCE_SELECTION".into(),
        scorer_version: "scorer-v1".into(),
        rubric_version: "rubric-v1".into(),
        calibration_key: "calibration-key".into(),
        calibration_version: "1".into(),
        dataset_namespace: "s1r4-fixtures".into(),
        dataset_split: "heldout-1".into(),
        evidence_refs: vec![EvaluationEvidenceRef {
            object_type: "DecisionRecord".into(),
            object_id: "decision-one".into(),
            object_version: "1".into(),
            content_hash: decision_hash.into(),
        }],
        metrics_hash: digest('e'),
        safety_status: "CLEAR".into(),
        privacy_status: "REDACTED".into(),
        previous: None,
    }
}
fn outcome_hash(decision_hash: &str) -> String {
    let bytes = object(vec![
        s("decisionHash", decision_hash),
        s("decisionId", "decision-one"),
        s("decisionRevision", "1"),
        s("domainId", "domain-one"),
        s("labelSource", "OBJECTIVE"),
        s("outcomeId", "objective-one"),
        s("revision", "1"),
    ]);
    content_hash(bytes.as_bytes())
}

#[test]
fn evaluation_append_read_replay_binds_exact_outcome_and_decision() {
    fixture(|db, decision_hash| {
        let input = request(&decision_hash);
        let committed = append_evaluation(db, &input).unwrap();
        assert_eq!(committed.disposition, "COMMITTED");
        let read = read_evaluation(db, "domain-one", "evaluation-one", "1").unwrap();
        assert_eq!(read.content_hash, committed.object_hash);
        assert_eq!(read.decision_family, "RESOURCE_SELECTION");
    assert_eq!(
        append_evaluation(db, &input).unwrap().disposition,
        "RECONCILED"
    );
    let mut correction=input.clone();correction.revision="2".into();correction.operation_id="evaluation-operation-two".into();correction.event_id="evaluation-event-two".into();correction.receipt_id="evaluation-receipt-two".into();correction.previous=Some(super::EvaluationVersionRef{revision:"1".into(),content_hash:read.content_hash.clone()});correction.metrics_hash=digest('f');
    append_evaluation(db,&correction).unwrap();
    assert_eq!(read_evaluation(db,"domain-one","evaluation-one","1").unwrap().canonical_evaluation,read.canonical_evaluation);
    assert_eq!(read_evaluation(db,"domain-one","evaluation-one","2").unwrap().revision,"2");
        let mut stale = input.clone();
        stale.outcome_refs[0].content_hash = digest('9');
        stale.operation_id = "other-op".into();
        stale.event_id = "other-event".into();
        stale.receipt_id = "other-receipt".into();
        assert!(append_evaluation(db, &stale).is_err());
        let mut cross = input.clone();
        cross.domain_id = "domain-two".into();
        cross.operation_id = "cross-op".into();
        cross.event_id = "cross-event".into();
        cross.receipt_id = "cross-receipt".into();
        assert!(append_evaluation(db, &cross).is_err());
        let mut evidence = input.clone();
        evidence.evidence_refs[0].content_hash = digest('8');
        evidence.operation_id = "bad-evidence".into();
        evidence.event_id = "bad-evidence-event".into();
        evidence.receipt_id = "bad-evidence-receipt".into();
        assert!(append_evaluation(db, &evidence).is_err());
    });
}
