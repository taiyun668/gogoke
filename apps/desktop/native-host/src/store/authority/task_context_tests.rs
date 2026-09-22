use super::task_context::{
    commit_task_context_requirements, initialize_task_context_schema,
    read_task_context_requirements, CommitTaskContextRequirements, MandatoryContextRef,
};
use crate::root::RootLock;
use crate::store::atomic::{count_table, initialize_product_core_schema};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::route_b_test_guard;
use crate::store::session::open_product_database;
use std::time::{SystemTime, UNIX_EPOCH};

fn mandatory(domain: &str, context: &str, version: &str) -> MandatoryContextRef {
    MandatoryContextRef {
        source_domain_id: domain.into(),
        context_id: context.into(),
        version: version.into(),
    }
}

fn input(operation: &str, previous: Option<&str>, refs: Vec<MandatoryContextRef>) -> CommitTaskContextRequirements {
    CommitTaskContextRequirements {
        operation_id: operation.into(),
        domain_id: "domain-one".into(),
        task_id: "task-one".into(),
        expected_previous_revision: previous.map(str::to_owned),
        mandatory_refs: refs,
        event_id: format!("event-{operation}"),
        receipt_id: format!("receipt-{operation}"),
        recorded_at: "2026-09-21T00:00:00Z".into(),
    }
}

fn fixture(test: impl FnOnce(&mut crate::store::same_open::VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-task-context-{nonce}"));
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut db = open_product_database(&root, &database).unwrap();
    initialize_product_core_schema(&mut db).unwrap();
    initialize_task_context_schema(&mut db).unwrap();
    test(&mut db);
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(&database).ok();
    std::fs::remove_file(format!("{}-wal", database.display())).ok();
    std::fs::remove_file(format!("{}-shm", database.display())).ok();
    std::fs::remove_dir(path).ok();
}

#[test]
fn create_revise_read_and_idempotent_replay_keep_one_current_task_and_full_history() {
    fixture(|db| {
        let create = input("create", None, vec![mandatory("source-b", "ctx-b", "2"), mandatory("source-a", "ctx-a", "1")]);
        let committed = commit_task_context_requirements(db, &create).unwrap();
        assert_eq!(committed.disposition, "COMMITTED");
        assert_eq!(committed.current.task_revision, "1");
        assert_eq!(committed.current.mandatory_refs[0], mandatory("source-a", "ctx-a", "1"));
        assert_eq!(commit_task_context_requirements(db, &create).unwrap().disposition, "RECONCILED");

        let revise = input("revise", Some("1"), vec![mandatory("source-a", "ctx-a", "3")]);
        assert_eq!(commit_task_context_requirements(db, &revise).unwrap().current.task_revision, "2");
        assert_eq!(commit_task_context_requirements(db, &revise).unwrap().disposition, "RECONCILED");
        let current = read_task_context_requirements(db, "domain-one", "task-one").unwrap();
        assert_eq!(current.task_revision, "2");
        assert_eq!(current.mandatory_refs, vec![mandatory("source-a", "ctx-a", "3")]);
        assert_eq!(count_table(db, "gogoke_objects").unwrap(), 2);
        assert_eq!(count_table(db, "gogoke_events").unwrap(), 2);
        assert_eq!(count_table(db, "gogoke_receipts").unwrap(), 2);
    });
}

#[test]
fn missing_task_duplicate_bad_revision_stale_cas_and_conflicting_replay_fail_closed() {
    fixture(|db| {
        assert!(matches!(read_task_context_requirements(db, "domain-one", "missing"), Err(OrchestrationError::AccessDenied)));
        let duplicate = mandatory("source-a", "ctx-a", "1");
        assert!(commit_task_context_requirements(db, &input("duplicate", None, vec![duplicate.clone(), duplicate])).is_err());
        assert!(commit_task_context_requirements(db, &input("two-versions", None, vec![
            mandatory("source-a", "ctx-a", "1"), mandatory("source-a", "ctx-a", "2"),
        ])).is_err());
        let oversized = (0..65).map(|index| mandatory("source-a", &format!("ctx-{index}"), "1")).collect();
        assert!(commit_task_context_requirements(db, &input("oversized", None, oversized)).is_err());
        assert!(commit_task_context_requirements(db, &input("zero", Some("0"), vec![])).is_err());

        let create = input("create", None, vec![]);
        commit_task_context_requirements(db, &create).unwrap();
        assert!(matches!(
            commit_task_context_requirements(db, &input("stale", Some("9"), vec![])),
            Err(OrchestrationError::OperationConflict)
        ));
        let mut conflict = create.clone();
        conflict.mandatory_refs.push(mandatory("source-a", "ctx-a", "1"));
        assert!(matches!(
            commit_task_context_requirements(db, &conflict),
            Err(OrchestrationError::Atomic(crate::store::atomic::AtomicError::OperationConflict))
        ));
        assert_eq!(read_task_context_requirements(db, "domain-one", "task-one").unwrap().task_revision, "1");
    });
}

#[test]
fn task_change_makes_an_older_task_commit_replay_fail_closed() {
    fixture(|db| {
        let create = input("create", None, vec![]);
        commit_task_context_requirements(db, &create).unwrap();
        commit_task_context_requirements(db, &input("revise", Some("1"), vec![])).unwrap();
        assert!(matches!(commit_task_context_requirements(db, &create), Err(OrchestrationError::OperationConflict)));
    });
}

#[test]
fn rolled_back_head_cannot_hide_a_later_immutable_task_revision() {
    fixture(|db| {
        commit_task_context_requirements(db, &input("create", None, vec![mandatory("source-a", "ctx-a", "1")])).unwrap();
        commit_task_context_requirements(db, &input("revise", Some("1"), vec![mandatory("source-a", "ctx-a", "2")])).unwrap();
        db.execute("UPDATE gogoke_task_context_heads SET task_revision='1',content_hash=(SELECT json_extract(CAST(canonical_json AS TEXT),'$.contentHash') FROM gogoke_objects WHERE domain_id='domain-one' AND object_type='Task' AND object_id='task-one' AND object_version='1') WHERE domain_id='domain-one' AND task_id='task-one'").unwrap();
        assert!(read_task_context_requirements(db, "domain-one", "task-one").is_err());
        assert!(commit_task_context_requirements(db, &input("fork-from-rolled-back-head", Some("1"), vec![])).is_err());
        assert_eq!(count_table(db, "gogoke_objects").unwrap(), 2);
        assert_eq!(count_table(db, "gogoke_events").unwrap(), 2);
        assert_eq!(count_table(db, "gogoke_receipts").unwrap(), 2);
    });
}

#[test]
fn temp_trigger_on_task_head_is_schema_corruption_and_never_runs() {
    fixture(|db| {
        commit_task_context_requirements(db, &input("create", None, vec![])).unwrap();
        db.execute("CREATE TEMP TRIGGER temp_task_head_guard BEFORE UPDATE ON gogoke_task_context_heads BEGIN SELECT 1; END").unwrap();
        assert!(read_task_context_requirements(db, "domain-one", "task-one").is_err());
        assert!(commit_task_context_requirements(db, &input("revise", Some("1"), vec![])).is_err());
        assert_eq!(count_table(db, "gogoke_objects").unwrap(), 1);
        assert_eq!(count_table(db, "gogoke_events").unwrap(), 1);
        assert_eq!(count_table(db, "gogoke_receipts").unwrap(), 1);
    });
}
