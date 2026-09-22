//! Native lifecycle-revision regressions. Definitions only until controlled Windows executes them.
use super::context::{apply_context_schema, commit_context_version, ContextCommand};
use super::context_state::{self, StateSnapshot};
use super::orchestration::OrchestrationError;
use super::same_open::{create_new, route_b_test_guard, VerifiedDatabaseConnection};
use crate::root::RootLock;
use std::time::{SystemTime, UNIX_EPOCH};

fn command(id: &str, version: &str) -> ContextCommand {
    ContextCommand {
        operation_id: format!("operation-{id}-{version}"), context_id: id.into(), version: version.into(),
        scope: "PROJECT".into(), domain_id: "domain-one".into(), kind: "fact".into(),
        content_hash: format!("sha256:{}", "a".repeat(64)), source_ref: "source://one".into(),
        source_hash: format!("sha256:{}", "b".repeat(64)), source_authority_kind: "repository".into(),
        source_authority_ref: "authority://one".into(), derived_from: vec![], supersedes: vec![],
        access_policy_revision: "1".into(), visibility: "OWNER_PRIVATE".into(),
        read_grant_refs: vec![], promotion: None,
    }
}

fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let root_path = std::env::temp_dir().join(format!("gogoke-state-revision-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root_path).unwrap();
    let root = RootLock::acquire(&root_path).unwrap();
    let database = root_path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    apply_context_schema(&mut connection).unwrap();
    run(&mut connection);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&root_path) {
        eprintln!("owned fixture retained: {} ({error})", root_path.display());
    }
}

fn snapshot(connection: &mut VerifiedDatabaseConnection<'_>, reference: &str) -> StateSnapshot {
    connection.execute("BEGIN IMMEDIATE").unwrap();
    let value = context_state::current(connection, "domain-one", reference).unwrap();
    connection.execute("COMMIT").unwrap();
    value
}

#[test]
fn new_context_begins_active_at_revision_one() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        assert_eq!(snapshot(connection, "one@1"), StateSnapshot { state: "ACTIVE".into(), revision: "1".into() });
    });
}

#[test]
fn supersede_and_descendant_invalidation_increment_each_changed_state_once() {
    fixture(|connection| {
        commit_context_version(connection, command("source", "1")).unwrap();
        let mut derived = command("derived", "1");
        derived.derived_from = vec!["source@1".into()];
        commit_context_version(connection, derived).unwrap();
        let mut replacement = command("source", "2");
        replacement.supersedes = vec!["source@1".into()];
        commit_context_version(connection, replacement).unwrap();
        assert_eq!(snapshot(connection, "source@1"), StateSnapshot { state: "SUPERSEDED".into(), revision: "2".into() });
        assert_eq!(snapshot(connection, "derived@1"), StateSnapshot { state: "STALE".into(), revision: "2".into() });
        assert_eq!(snapshot(connection, "source@2"), StateSnapshot { state: "ACTIVE".into(), revision: "1".into() });
    });
}

#[test]
fn already_nonactive_descendant_is_not_rewritten_or_revised() {
    fixture(|connection| {
        commit_context_version(connection, command("source", "1")).unwrap();
        let mut derived = command("derived", "1");
        derived.derived_from = vec!["source@1".into()];
        commit_context_version(connection, derived).unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        context_state::transition(connection, "domain-one", "derived@1", "ACTIVE", "ARCHIVED").unwrap();
        connection.execute("COMMIT").unwrap();
        let before = snapshot(connection, "derived@1");
        let mut replacement = command("source", "2");
        replacement.supersedes = vec!["source@1".into()];
        commit_context_version(connection, replacement).unwrap();
        assert_eq!(snapshot(connection, "derived@1"), before);
    });
}

#[test]
fn rollback_restores_both_state_and_revision() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        context_state::transition(connection, "domain-one", "one@1", "ACTIVE", "STALE").unwrap();
        connection.execute("ROLLBACK").unwrap();
        assert_eq!(snapshot(connection, "one@1"), StateSnapshot { state: "ACTIVE".into(), revision: "1".into() });
    });
}

#[test]
fn stale_context_cannot_be_reactivated_by_the_generic_transition_primitive() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        context_state::transition(connection, "domain-one", "one@1", "ACTIVE", "STALE").unwrap();
        assert!(matches!(
            context_state::transition(connection, "domain-one", "one@1", "STALE", "ACTIVE"),
            Err(OrchestrationError::Invalid("Context state transition not admitted"))
        ));
        connection.execute("COMMIT").unwrap();
        assert_eq!(snapshot(connection, "one@1"), StateSnapshot { state: "STALE".into(), revision: "2".into() });
    });
}

#[test]
fn revoked_context_cannot_be_reactivated_and_unimplemented_active_edges_fail_closed() {
    fixture(|connection| {
        connection.execute("INSERT INTO gogoke_context_states(domain_id,version_ref,state) VALUES ('domain-one','revoked@1','REVOKED')").unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        let _ = context_state::current(connection, "domain-one", "revoked@1").unwrap();
        assert!(context_state::transition(connection, "domain-one", "revoked@1", "REVOKED", "ACTIVE").is_err());
        connection.execute("ROLLBACK").unwrap();

        commit_context_version(connection, command("active", "1")).unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        for next in ["CONFLICTED", "REVOKED"] {
            assert!(matches!(
                context_state::transition(connection, "domain-one", "active@1", "ACTIVE", next),
                Err(OrchestrationError::Invalid("Context state transition not admitted"))
            ));
        }
        connection.execute("ROLLBACK").unwrap();
        assert_eq!(snapshot(connection, "active@1"), StateSnapshot { state: "ACTIVE".into(), revision: "1".into() });
    });
}

#[test]
fn revision_overflow_fails_before_state_change() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        connection.execute("UPDATE gogoke_context_state_revisions SET state_revision='18446744073709551615' WHERE domain_id='domain-one' AND version_ref='one@1'").unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(
            context_state::transition(connection, "domain-one", "one@1", "ACTIVE", "STALE"),
            Err(OrchestrationError::Invalid("Context state revision overflow"))
        ));
        connection.execute("ROLLBACK").unwrap();
        assert_eq!(snapshot(connection, "one@1"), StateSnapshot { state: "ACTIVE".into(), revision: "18446744073709551615".into() });
    });
}

#[test]
fn legacy_state_rows_are_backfilled_without_changing_state() {
    fixture(|connection| {
        connection.execute("INSERT INTO gogoke_context_states(domain_id,version_ref,state) VALUES ('domain-one','legacy@1','REVOKED')").unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        let value = context_state::current(connection, "domain-one", "legacy@1").unwrap();
        connection.execute("COMMIT").unwrap();
        assert_eq!(value, StateSnapshot { state: "REVOKED".into(), revision: "1".into() });
    });
}

#[test]
fn missing_or_mismatched_expected_state_fails_closed() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        assert!(context_state::transition(connection, "domain-one", "one@1", "STALE", "ACTIVE").is_err());
        assert!(context_state::current(connection, "domain-one", "missing@1").is_err());
        connection.execute("ROLLBACK").unwrap();
    });
}


#[test]
fn existing_revision_schema_never_silently_repairs_a_missing_revision_row() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        connection.execute("DELETE FROM gogoke_context_state_revisions WHERE domain_id='domain-one' AND version_ref='one@1'").unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(
            context_state::current(connection, "domain-one", "one@1"),
            Err(OrchestrationError::Invalid("Context state revision coverage mismatch"))
        ));
        connection.execute("ROLLBACK").unwrap();
    });
}


#[test]
fn state_revision_zero_is_rejected_by_the_persisted_schema() {
    fixture(|connection| {
        commit_context_version(connection, command("one", "1")).unwrap();
        assert!(connection.execute(
            "UPDATE gogoke_context_state_revisions SET state_revision='0' WHERE domain_id='domain-one' AND version_ref='one@1'"
        ).is_err());
        assert_eq!(snapshot(connection, "one@1"), StateSnapshot { state: "ACTIVE".into(), revision: "1".into() });
    });
}
