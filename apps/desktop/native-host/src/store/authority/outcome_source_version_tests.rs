//! Native Outcome-source identity tests. Definitions until exact Windows execution.
//! Core/Action seeds are synthetic fixtures, not qualified Decision or native IO.
use super::bootstrap::{initialize_profile, OwnerIssuer};
use super::catalog::current_profile;
use super::outcome::{append_owner_override_outcome, OutcomeVersionRef, OwnerOutcomeAppend};
use super::transaction;
use crate::root::RootLock;
use crate::store::action::{apply_action_schema, reserve_action, ActionReservation};
use crate::store::atomic::{apply_core_schema, commit_domain_record, AtomicError, DomainRecordInput, Statement};
use crate::store::digest::content_hash;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

const ACTION: &str = "opr_11111111111111111111111111111111";
const WHEN: &str = "2026-09-21T00:00:00Z";
fn decision_bytes() -> Vec<u8> {
    format!("{{\"actionId\":\"{ACTION}\",\"backend\":\"RULES\",\"calibrationRef\":\"fixture-only\",\"candidateHash\":\"sha256:{}\",\"decisionId\":\"decision-one\",\"family\":\"RESOURCE_SELECTION\",\"mode\":\"fixture\",\"modelRequested\":\"none\",\"modelResolved\":\"none\",\"nativeConfidence\":null,\"probabilities\":{{}},\"questionVersion\":\"1\",\"sourceRevisions\":{{}},\"state\":\"COMMITTED\",\"stateViewHash\":\"sha256:{}\"}}", "c".repeat(64), "d".repeat(64)).into_bytes()
}
fn decision(domain: &str, version: &str) -> DomainRecordInput {
    DomainRecordInput {
        domain_id: domain.into(), object_type: "DecisionRecord".into(), object_id: "decision-one".into(),
        object_version: version.into(), object_bytes: decision_bytes(), native_identity: None,
        event_id: format!("decision-event-{version}"), stream_id: format!("fixture-decision-{version}"),
        expected_previous_counter: None, counter: "0".into(), event_type: "FixtureDecision".into(),
        occurred_at: WHEN.into(), event_bytes: br#"{"fixture":true}"#.to_vec(),
        receipt_id: format!("decision-receipt-{version}"), operation_id: format!("decision-operation-{version}"),
        receipt_type: "FixtureDecision".into(), recorded_at: WHEN.into(), receipt_bytes: br#"{"fixture":true}"#.to_vec(),
    }
}
fn outcome_bytes(revision: &str, status: &str, quality: &str) -> Vec<u8> {
    format!("{{\"actionId\":\"{ACTION}\",\"censorStatus\":\"{status}\",\"cost\":null,\"decisionId\":\"decision-one\",\"evidenceRefs\":[\"evidence-owner-one\"],\"labelSource\":\"OWNER_OVERRIDE\",\"latency\":null,\"observationWindow\":null,\"outcomeId\":\"outcome-one\",\"quality\":\"{quality}\",\"revision\":\"{revision}\",\"rework\":null,\"safetyEvents\":[]}}").into_bytes()
}
fn request(policy: &str, revocation: &str) -> OwnerOutcomeAppend {
    OwnerOutcomeAppend {
        domain_id: "domain-one".into(), operation_id: "outcome-operation-one".into(),
        event_id: "outcome-event-one".into(), receipt_id: "outcome-receipt-one".into(), recorded_at: WHEN.into(),
        policy_revision: policy.into(), revocation_head: revocation.into(), decision_version: "1".into(),
        decision_hash: content_hash(&decision_bytes()), action_operation_id: ACTION.into(),
        action_digest: format!("sha256:{}", "a".repeat(64)), previous: None,
        canonical_outcome: outcome_bytes("1", "OBSERVED", "owner-observation"),
    }
}
fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &OwnerIssuer, &str, &str)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-outcome-source-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_core_schema(&mut connection).unwrap();
    apply_action_schema(&mut connection).unwrap();
    let owner = initialize_profile(&mut connection, &root).unwrap();
    let profile = transaction::run(&mut connection, |tx| current_profile(tx)).unwrap();
    commit_domain_record(&mut connection, decision("domain-one", "1")).unwrap();
    reserve_action(&mut connection, ActionReservation {
        operation_id: ACTION.into(), semantic_digest: format!("sha256:{}", "a".repeat(64)),
        reservation_id: "reservation-one".into(), binding_id: "binding-one".into(), session_id: "session-one".into(), execution_id:"execution-one".into(),
        runtime_instance_id: "runtime-one".into(), profile_id: "runtime-account-one".into(),
        auth_revision: "1".into(), generation: "1".into(), lane: "work".into(), action_kind: "queue".into(), payload_hex: "7b7d".into(),
        commitment:crate::store::action::test_commitment("session-one","execution-one","1"),
    }).unwrap();
    run(&mut connection, &owner, &profile.policy_revision, &profile.revocation_head);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(&database).unwrap();
    for suffix in ["-wal", "-shm"] {
        match std::fs::remove_file(format!("{}{suffix}", database.display())) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => panic!("owned fixture cleanup: {error}"),
        }
    }
    std::fs::remove_dir(path).unwrap();
}
fn count_outcomes(connection: &mut VerifiedDatabaseConnection<'_>) -> String {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT count(*) FROM gogoke_objects WHERE object_type='OutcomeRecord'").unwrap();
    assert!(statement.step_row().unwrap());
    statement.column_text(0).unwrap()
}

#[test]
fn ov01_exact_replay_preserves_one_outcome() {
    fixture(|connection, owner, policy, revocation| {
        let input = request(policy, revocation);
        assert_eq!(append_owner_override_outcome(connection, owner, &input).unwrap().disposition, "COMMITTED");
        assert_eq!(append_owner_override_outcome(connection, owner, &input).unwrap().disposition, "RECONCILED");
        assert_eq!(count_outcomes(connection), "1");
    });
}

#[test]
fn ov02_equal_payload_hash_does_not_erase_decision_source_version() {
    fixture(|connection, owner, policy, revocation| {
        let mut input = request(policy, revocation);
        append_owner_override_outcome(connection, owner, &input).unwrap();
        commit_domain_record(connection, decision("domain-one", "2")).unwrap();
        input.decision_version = "2".into(); // Deliberately identical Decision payload/hash.
        assert!(matches!(append_owner_override_outcome(connection, owner, &input),
            Err(OrchestrationError::Atomic(AtomicError::OperationConflict))));
        assert_eq!(count_outcomes(connection), "1");
    });
}

#[test]
fn ov03_missing_decision_source_version_is_denied_before_write() {
    fixture(|connection, owner, policy, revocation| {
        let mut input = request(policy, revocation); input.decision_version = "2".into();
        assert!(append_owner_override_outcome(connection, owner, &input).is_err());
        assert_eq!(count_outcomes(connection), "0");
    });
}

#[test]
fn ov04_source_domain_cannot_be_inferred_from_a_decoy() {
    fixture(|connection, owner, policy, revocation| {
        let mut input = request(policy, revocation); input.domain_id = "domain-other".into();
        assert!(append_owner_override_outcome(connection, owner, &input).is_err());
        assert_eq!(count_outcomes(connection), "0");
    });
}

#[test]
fn ov05_stale_policy_and_revocation_are_rechecked_even_on_replay() {
    fixture(|connection, owner, policy, revocation| {
        let input = request(policy, revocation); append_owner_override_outcome(connection, owner, &input).unwrap();
        let mut stale = input.clone(); stale.policy_revision = "9999".into();
        assert!(append_owner_override_outcome(connection, owner, &stale).is_err());
        stale = input.clone(); stale.revocation_head = "9999".into();
        assert!(append_owner_override_outcome(connection, owner, &stale).is_err());
        assert_eq!(count_outcomes(connection), "1");
    });
}

#[test]
fn ov06_owner_override_cannot_impersonate_objective_or_self_report_producers() {
    fixture(|connection, owner, policy, revocation| {
        for label in ["OBJECTIVE", "INDEPENDENT_SEMANTIC", "SELF_REPORT"] {
            let mut input = request(policy, revocation);
            input.canonical_outcome = String::from_utf8(input.canonical_outcome).unwrap().replace("OWNER_OVERRIDE", label).into_bytes();
            assert!(append_owner_override_outcome(connection, owner, &input).is_err());
        }
        assert_eq!(count_outcomes(connection), "0");
    });
}

#[test]
fn ov07_correction_appends_and_old_operation_replay_does_not_undo_it() {
    fixture(|connection, owner, policy, revocation| {
        let first = request(policy, revocation); append_owner_override_outcome(connection, owner, &first).unwrap();
        let mut second = first.clone(); second.operation_id = "outcome-operation-two".into();
        second.event_id = "outcome-event-two".into(); second.receipt_id = "outcome-receipt-two".into();
        second.previous = Some(OutcomeVersionRef { revision: "1".into(), content_hash: content_hash(&first.canonical_outcome) });
        second.canonical_outcome = outcome_bytes("2", "CORRECTED", "owner-correction");
        append_owner_override_outcome(connection, owner, &second).unwrap();
        assert_eq!(append_owner_override_outcome(connection, owner, &first).unwrap().disposition, "RECONCILED");
        assert_eq!(count_outcomes(connection), "2");
        let statement = Statement::prepare(connection.as_ptr(), "SELECT CAST(canonical_json AS TEXT) FROM gogoke_objects WHERE domain_id='domain-one' AND object_type='OutcomeRecord' AND object_id='outcome-one' ORDER BY object_version").unwrap();
        assert!(statement.step_row().unwrap()); assert_eq!(statement.column_text(0).unwrap().as_bytes(), first.canonical_outcome.as_slice());
        assert!(statement.step_row().unwrap()); assert_eq!(statement.column_text(0).unwrap().as_bytes(), second.canonical_outcome.as_slice());
    });
}

#[test]
fn ov08_changed_action_identity_or_digest_fails_closed() {
    fixture(|connection, owner, policy, revocation| {
        let mut input = request(policy, revocation); input.action_operation_id = "opr_22222222222222222222222222222222".into();
        assert!(append_owner_override_outcome(connection, owner, &input).is_err());
        input = request(policy, revocation); input.action_digest = format!("sha256:{}", "b".repeat(64));
        assert!(append_owner_override_outcome(connection, owner, &input).is_err());
        assert_eq!(count_outcomes(connection), "0");
    });
}
