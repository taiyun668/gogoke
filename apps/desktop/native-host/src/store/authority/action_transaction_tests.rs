//! Actual Route-B/SQLite transaction composition tests, controlled Windows only.
//! No fake grant or live dispatch: the temporary probe is a transaction witness,
//! not the production Decision/capacity schema. Definitions are not executions.
use super::transaction;
use crate::root::RootLock;
use crate::store::action::{
    apply_action_schema, record_action_outcome, reserve_action, reserve_action_in_transaction,
    ActionReservation, ReserveDisposition,
};
use crate::store::atomic::Statement;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use std::time::{SystemTime, UNIX_EPOCH};

fn input() -> ActionReservation {
    ActionReservation {
        operation_id: "opr_11111111111111111111111111111111".into(),
        semantic_digest: format!("sha256:{}", "a".repeat(64)),
        reservation_id: "reservation-one".into(), binding_id: "binding-one".into(),
        session_id: "session-one".into(), execution_id:"execution-one".into(), runtime_instance_id: "runtime-one".into(),
        profile_id: "profile-one".into(), auth_revision: "1".into(), generation: "1".into(),
        lane: "work".into(), action_kind: "queue".into(), payload_hex: "7b7d".into(),
        commitment:crate::store::action::test_commitment("session-one","execution-one","1"),
    }
}
fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-action-compose-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_action_schema(&mut connection).unwrap();
    connection.execute("CREATE TEMP TABLE transaction_probe (id TEXT PRIMARY KEY) STRICT").unwrap();
    run(&mut connection);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {error}"); }
}
fn count(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> String {
    let statement = Statement::prepare(connection.as_ptr(), sql).unwrap();
    assert!(statement.step_row().unwrap());
    let result = statement.column_text(0).unwrap();
    assert!(!statement.step_row().unwrap());
    result
}

#[test]
fn in_transaction_primitive_rejects_autocommit_without_creating_a_row() {
    fixture(|connection| {
        assert!(matches!(reserve_action_in_transaction(connection, input()),
            Err(OrchestrationError::Invalid("action reservation requires owning transaction"))));
        assert_eq!(count(connection, "SELECT count(*) FROM gogoke_action_reservations"), "0");
    });
}

#[test]
fn outer_commit_owns_reservation_and_neighbor_write() {
    fixture(|connection| {
        let result = transaction::run(connection, |tx| {
            let result = tx.reserve_action(input())?;
            tx.write("INSERT INTO transaction_probe(id) VALUES(?)", &["witness"])?;
            Ok(result)
        }).unwrap();
        assert_eq!(result, ReserveDisposition::Reserved);
        assert_eq!(count(connection, "SELECT count(*) FROM gogoke_action_reservations"), "1");
        assert_eq!(count(connection, "SELECT count(*) FROM transaction_probe"), "1");
        assert_eq!(reserve_action(connection, input()).unwrap(),
            ReserveDisposition::Replay { state: "reserved".into() });
    });
}

#[test]
fn outer_error_rolls_back_reservation_and_neighbor_write() {
    fixture(|connection| {
        let result = transaction::run(connection, |tx| {
            tx.write("INSERT INTO transaction_probe(id) VALUES(?)", &["witness"])?;
            assert_eq!(tx.reserve_action(input())?, ReserveDisposition::Reserved);
            Err::<(), _>(OrchestrationError::Invalid("controlled rollback witness"))
        });
        assert!(matches!(result, Err(OrchestrationError::Invalid("controlled rollback witness"))));
        assert_eq!(count(connection, "SELECT count(*) FROM gogoke_action_reservations"), "0");
        assert_eq!(count(connection, "SELECT count(*) FROM transaction_probe"), "0");
    });
}

#[test]
fn helper_never_commits_the_callers_explicit_transaction() {
    fixture(|connection| {
        connection.execute("BEGIN IMMEDIATE").unwrap();
        assert_eq!(reserve_action_in_transaction(connection, input()).unwrap(), ReserveDisposition::Reserved);
        connection.execute("ROLLBACK").unwrap();
        assert_eq!(count(connection, "SELECT count(*) FROM gogoke_action_reservations"), "0");
    });
}

#[test]
fn second_reservation_identity_conflict_rolls_back_the_whole_outer_group() {
    fixture(|connection| {
        let result = transaction::run(connection, |tx| {
            tx.reserve_action(input())?;
            tx.write("INSERT INTO transaction_probe(id) VALUES(?)", &["witness"])?;
            let mut other = input();
            other.operation_id = "opr_22222222222222222222222222222222".into();
            // A different operation must not steal the same UNIQUE reservation.
            tx.reserve_action(other)?;
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(count(connection, "SELECT count(*) FROM gogoke_action_reservations"), "0");
        assert_eq!(count(connection, "SELECT count(*) FROM transaction_probe"), "0");
    });
}

fn fields(value: &ActionReservation) -> [&str; 13] {
    [&value.operation_id, &value.semantic_digest, &value.reservation_id, &value.binding_id,
     &value.session_id, &value.execution_id, &value.runtime_instance_id, &value.profile_id, &value.auth_revision,
     &value.generation, &value.lane, &value.action_kind, &value.payload_hex]
}

#[test]
fn every_tuple_field_remains_bound_before_and_after_unknown_outcome() {
    fixture(|connection| {
        let baseline = input();
        reserve_action(connection, baseline.clone()).unwrap();
        let mutations: [fn(&mut ActionReservation); 12] = [
            |x| x.semantic_digest = format!("sha256:{}", "b".repeat(64)),
            |x| x.reservation_id = "reservation-two".into(),
            |x| x.binding_id = "binding-two".into(),
            |x| {x.session_id = "session-two".into();x.commitment.child_session_id="session-two".into();},
            |x| {x.execution_id = "execution-two".into();x.commitment.child_execution_id="execution-two".into();},
            |x| x.runtime_instance_id = "runtime-two".into(),
            |x| x.profile_id = "profile-two".into(),
            |x| x.auth_revision = "2".into(),
            |x| {x.generation = "2".into();x.commitment.child_generation="2".into();},
            |x| x.lane = "control".into(),
            |x| x.action_kind = "steer".into(),
            |x| x.payload_hex = "7b2278223a317d".into(),
        ];
        for state in ["reserved", "outcome-unknown"] {
            if state == "outcome-unknown" {
                record_action_outcome(connection, &baseline.reservation_id, &baseline.operation_id,
                    &baseline.semantic_digest, state, "EOF", "").unwrap();
            }
            for (index, mutate) in mutations.iter().enumerate() {
                let mut changed = baseline.clone(); mutate(&mut changed);
                assert_eq!(fields(&baseline).iter().zip(fields(&changed).iter())
                    .filter(|(left, right)| left != right).count(), 1, "field {index}");
                let result = transaction::run(connection, |tx| tx.reserve_action(changed)).unwrap();
                assert_eq!(result, ReserveDisposition::Conflict { existing_digest: baseline.semantic_digest.clone() }, "field {index}");
                let replay = transaction::run(connection, |tx| tx.reserve_action(baseline.clone())).unwrap();
                assert_eq!(replay, ReserveDisposition::Replay { state: state.into() });
            }
        }
    });
}

#[test]
fn malformed_input_in_outer_group_rolls_back_earlier_writes() {
    fixture(|connection| {
        let result = transaction::run(connection, |tx| {
            tx.write("INSERT INTO transaction_probe(id) VALUES(?)", &["witness"])?;
            let mut invalid = input(); invalid.generation = "01".into();
            tx.reserve_action(invalid)?;
            Ok(())
        });
        assert!(matches!(result, Err(OrchestrationError::Invalid("action reservation"))));
        assert_eq!(count(connection, "SELECT count(*) FROM gogoke_action_reservations"), "0");
        assert_eq!(count(connection, "SELECT count(*) FROM transaction_probe"), "0");
    });
}

#[test]
fn legacy_wrapper_returns_to_autocommit_and_keeps_replay_semantics() {
    fixture(|connection| {
        assert_eq!(reserve_action(connection, input()).unwrap(), ReserveDisposition::Reserved);
        assert!(matches!(reserve_action_in_transaction(connection, input()),
            Err(OrchestrationError::Invalid("action reservation requires owning transaction"))));
        assert_eq!(reserve_action(connection, input()).unwrap(),
            ReserveDisposition::Replay { state: "reserved".into() });
    });
}
