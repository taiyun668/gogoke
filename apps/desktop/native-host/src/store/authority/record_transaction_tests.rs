//! Actual shared Route-B write groups; definitions until controlled Windows runs.
//! These generic probe records are not production Decision/Outcome records or
//! proof of actor admission, current resource capacity, or external dispatch.
use super::transaction;
use crate::root::RootLock;
use crate::store::action::{apply_action_schema, reserve_action, ActionReservation, ReserveDisposition};
use crate::store::atomic::{apply_core_schema, apply_domain_record_in_transaction, commit_domain_record,
    count_table, get_receipt, AtomicError, DomainRecordInput, DomainRecordReceipt, Statement};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, open_existing, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch() -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-record-composition-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}
fn cleanup(path: &std::path::Path) {
    let database = path.join("state.sqlite");
    std::fs::remove_file(&database).unwrap();
    for suffix in ["-wal", "-shm"] {
        let sidecar = format!("{}{suffix}", database.display());
        match std::fs::remove_file(sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("owned fixture sidecar cleanup: {error}"),
        }
    }
    std::fs::remove_dir(path).unwrap();
}
fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let path = scratch();
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    // Existing legacy core admission remains strict; initialize the original
    // schema before adding the existing ActionStore in this owned fixture only.
    apply_core_schema(&mut connection).unwrap();
    apply_action_schema(&mut connection).unwrap();
    run(&mut connection);
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}
fn record(domain: &str) -> DomainRecordInput {
    DomainRecordInput {
        domain_id: domain.into(), object_type: "transaction-probe".into(), object_id: "object-one".into(),
        object_version: "1".into(), object_bytes: br#"{"probe":"one"}"#.to_vec(), native_identity: None,
        event_id: "event-one".into(), stream_id: "stream-one".into(), expected_previous_counter: None,
        counter: "0".into(), event_type: "probe.created".into(), occurred_at: "2026-09-21T00:00:00Z".into(),
        event_bytes: br#"{"kind":"probe"}"#.to_vec(), receipt_id: "receipt-one".into(),
        operation_id: "operation-one".into(), receipt_type: "probe.stored".into(),
        recorded_at: "2026-09-21T00:00:00Z".into(), receipt_bytes: br#"{"stored":true}"#.to_vec(),
    }
}
fn action() -> ActionReservation {
    ActionReservation {
        operation_id: "opr_11111111111111111111111111111111".into(),
        semantic_digest: format!("sha256:{}", "a".repeat(64)), reservation_id: "reservation-one".into(),
        binding_id: "binding-one".into(), session_id: "session-one".into(), execution_id:"execution-one".into(), runtime_instance_id: "runtime-one".into(),
        profile_id: "profile-one".into(), auth_revision: "1".into(), generation: "1".into(),
        lane: "work".into(), action_kind: "queue".into(), payload_hex: "7b7d".into(),commitment:crate::store::action::test_commitment("session-one","execution-one","1"),
    }
}
fn action_count(connection: &mut VerifiedDatabaseConnection<'_>) -> String {
    let query = Statement::prepare(connection.as_ptr(), "SELECT count(*) FROM gogoke_action_reservations").unwrap();
    assert!(query.step_row().unwrap());
    query.column_text(0).unwrap()
}
fn assert_empty(connection: &mut VerifiedDatabaseConnection<'_>) {
    for table in ["gogoke_objects", "gogoke_events", "gogoke_receipts", "gogoke_stream_heads"] {
        assert_eq!(count_table(connection, table).unwrap(), 0, "{table}");
    }
    assert_eq!(action_count(connection), "0");
}

#[test]
fn record_group_requires_an_owning_transaction_before_any_write() {
    fixture(|connection| {
        assert!(matches!(apply_domain_record_in_transaction(connection, record("domain-one")),
            Err(AtomicError::InvalidRecord("record write group requires owning transaction"))));
        assert_empty(connection);
    });
}

#[test]
fn record_event_receipt_and_action_share_one_native_commit() {
    fixture(|connection| {
        let receipt = transaction::run(connection, |tx| {
            let receipt = tx.apply_domain_record(record("domain-one"))?;
            assert_eq!(tx.reserve_action(action())?, ReserveDisposition::Reserved);
            assert_eq!(tx.query("SELECT count(*) FROM gogoke_receipts", &[], 1)?[0][0], "1");
            Ok(receipt)
        }).unwrap();
        assert_eq!(receipt.disposition, "COMMITTED");
        for table in ["gogoke_objects", "gogoke_events", "gogoke_receipts", "gogoke_stream_heads"] {
            assert_eq!(count_table(connection, table).unwrap(), 1);
        }
        assert_eq!(action_count(connection), "1");
        assert_eq!(get_receipt(connection, "domain-one", "operation-one").unwrap().unwrap().operation_fingerprint,
            receipt.operation_fingerprint);
    });
}

#[test]
fn outer_error_rolls_back_all_record_and_action_rows() {
    fixture(|connection| {
        let result: transaction::Result<()> = transaction::run(connection, |tx| {
            tx.apply_domain_record(record("domain-one"))?;
            tx.reserve_action(action())?;
            Err(OrchestrationError::AccessDenied)
        });
        assert!(matches!(result, Err(OrchestrationError::AccessDenied)));
        assert_empty(connection);
    });
}

#[test]
fn action_tuple_conflict_cannot_commit_an_unrelated_record() {
    fixture(|connection| {
        reserve_action(connection, action()).unwrap();
        let mut changed = action();
        changed.session_id = "session-other".into();
        changed.commitment.child_session_id = "session-other".into();
        let result: transaction::Result<()> = transaction::run(connection, |tx| {
            tx.apply_domain_record(record("domain-one"))?;
            if matches!(tx.reserve_action(changed)?, ReserveDisposition::Conflict { .. }) {
                return Err(OrchestrationError::OperationConflict);
            }
            panic!("changed exact reservation tuple must conflict");
        });
        assert!(matches!(result, Err(OrchestrationError::OperationConflict)));
        assert_eq!(count_table(connection, "gogoke_receipts").unwrap(), 0);
        assert_eq!(count_table(connection, "gogoke_objects").unwrap(), 0);
        assert_eq!(count_table(connection, "gogoke_stream_heads").unwrap(), 0);
        assert_eq!(action_count(connection), "1");
    });
}

#[test]
fn record_identity_conflict_rolls_back_a_new_action_reservation() {
    fixture(|connection| {
        commit_domain_record(connection, record("domain-one")).unwrap();
        let mut changed = record("domain-one");
        changed.object_bytes = br#"{"probe":"changed"}"#.to_vec();
        let result = transaction::run(connection, |tx| {
            tx.reserve_action(action())?;
            tx.apply_domain_record(changed)
        });
        assert!(matches!(result, Err(OrchestrationError::Atomic(AtomicError::OperationConflict))));
        assert_eq!(count_table(connection, "gogoke_objects").unwrap(), 1);
        assert_eq!(count_table(connection, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(connection, "gogoke_receipts").unwrap(), 1);
        assert_eq!(action_count(connection), "0");
    });
}

#[test]
fn exact_storage_replay_adds_neither_records_nor_reservations() {
    fixture(|connection| {
        for expected in ["COMMITTED", "RECONCILED"] {
            let receipt = transaction::run(connection, |tx| {
                let receipt = tx.apply_domain_record(record("domain-one"))?;
                let disposition = tx.reserve_action(action())?;
                if expected == "COMMITTED" { assert_eq!(disposition, ReserveDisposition::Reserved); }
                else { assert_eq!(disposition, ReserveDisposition::Replay { state: "reserved".into() }); }
                Ok(receipt)
            }).unwrap();
            assert_eq!(receipt.disposition, expected);
        }
        assert_eq!(count_table(connection, "gogoke_receipts").unwrap(), 1);
        assert_eq!(action_count(connection), "1");
    });
}

#[test]
fn noncanonical_record_rolls_back_preceding_action_without_a_partial_receipt() {
    fixture(|connection| {
        let mut malformed = record("domain-one");
        malformed.object_bytes = br#"{ "probe": "one" }"#.to_vec();
        let result = transaction::run(connection, |tx| {
            tx.reserve_action(action())?;
            tx.apply_domain_record(malformed)
        });
        assert!(matches!(result, Err(OrchestrationError::Atomic(AtomicError::NonCanonicalJson(_)))));
        assert_empty(connection);
    });
}

#[test]
fn stale_stream_counter_rolls_back_a_preceding_action() {
    fixture(|connection| {
        commit_domain_record(connection, record("domain-one")).unwrap();
        let mut stale = record("domain-one");
        stale.object_id = "object-two".into(); stale.event_id = "event-two".into();
        stale.operation_id = "operation-two".into(); stale.receipt_id = "receipt-two".into();
        stale.expected_previous_counter = Some("1".into()); stale.counter = "2".into();
        let result = transaction::run(connection, |tx| { tx.reserve_action(action())?; tx.apply_domain_record(stale) });
        assert!(matches!(result, Err(OrchestrationError::Atomic(AtomicError::CounterConflict))));
        assert_eq!(count_table(connection, "gogoke_receipts").unwrap(), 1);
        assert_eq!(action_count(connection), "0");
    });
}

#[test]
fn identical_record_identifiers_in_distinct_domains_do_not_reconcile_together() {
    fixture(|connection| {
        for domain in ["domain-one", "domain-two"] {
            let receipt = transaction::run(connection, |tx| tx.apply_domain_record(record(domain))).unwrap();
            assert_eq!(receipt.disposition, "COMMITTED");
            assert_eq!(receipt.domain_id, domain);
        }
        assert_eq!(count_table(connection, "gogoke_receipts").unwrap(), 2);
        assert!(get_receipt(connection, "domain-three", "operation-one").unwrap().is_none());
    });
}

#[test]
fn checked_close_then_reopen_preserves_committed_record_and_action_evidence() {
    let _guard = route_b_test_guard();
    let path = scratch();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_core_schema(&mut connection).unwrap();
    apply_action_schema(&mut connection).unwrap();
    let committed: DomainRecordReceipt = transaction::run(&mut connection, |tx| {
        let receipt = tx.apply_domain_record(record("domain-one"))?;
        tx.reserve_action(action())?;
        Ok(receipt)
    }).unwrap();
    connection.close_checked().unwrap();
    let mut reopened = open_existing(&root, &database).unwrap();
    let stored = get_receipt(&mut reopened, "domain-one", "operation-one").unwrap().unwrap();
    assert_eq!(stored.operation_fingerprint, committed.operation_fingerprint);
    assert_eq!(stored.object_hash, committed.object_hash);
    assert_eq!(stored.event_hash, committed.event_hash);
    assert_eq!(stored.receipt_hash, committed.receipt_hash);
    assert_eq!(reserve_action(&mut reopened, action()).unwrap(), ReserveDisposition::Replay { state: "reserved".into() });
    reopened.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}
