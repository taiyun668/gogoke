//! Controlled Windows/Route-B tests; source definitions are not execution evidence.
use super::bootstrap::initialize_profile;
use super::catalog::current_profile;
use super::transaction;
use crate::root::RootLock;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn scratch(label: &str) -> PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("gogoke-authority-root-{label}-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}
fn cleanup(path: &std::path::Path) {
    std::fs::remove_file(path.join("state.sqlite")).unwrap();
    if let Err(error) = std::fs::remove_dir(path) { eprintln!("owned fixture retained: {error}"); }
}

#[test]
fn transaction_root_is_the_pinned_directory_not_the_database_file() {
    let _guard = route_b_test_guard();
    let path = scratch("identity");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    let expected = root.canonical_root().identity.opaque();
    assert_ne!(connection.identity().opaque(), expected, "distinct physical file and directory required");
    assert_eq!(connection.root_identity().opaque(), expected);
    let owner = initialize_profile(&mut connection, &root).unwrap();
    transaction::run(&mut connection, |tx| {
        assert_eq!(tx.root_identity(), expected);
        let profile = current_profile(tx)?;
        assert_eq!(profile.root_identity, expected);
        owner.check(&profile)
    }).unwrap();
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn unrelated_root_lock_cannot_initialize_authority_or_leave_partial_schema() {
    let _guard = route_b_test_guard();
    let first_path = scratch("bound");
    let other_path = scratch("unrelated");
    let first_root = RootLock::acquire(&first_path).unwrap();
    let other_root = RootLock::acquire(&other_path).unwrap();
    let mut connection = create_new(&first_root, &first_path.join("state.sqlite")).unwrap();
    assert!(matches!(initialize_profile(&mut connection, &other_root), Err(OrchestrationError::AccessDenied)));
    transaction::run(&mut connection, |tx| {
        assert_eq!(tx.query("SELECT count(*) FROM sqlite_schema WHERE name GLOB 'gogoke_authority_*'", &[], 1)?[0][0], "0");
        Ok(())
    }).unwrap();
    let owner = initialize_profile(&mut connection, &first_root).unwrap();
    let reopened = initialize_profile(&mut connection, &first_root).unwrap();
    assert_eq!(owner.principal_id(), reopened.principal_id());
    assert_eq!(owner.seat_id(), reopened.seat_id());
    connection.close_checked().unwrap();
    drop(other_root);
    drop(first_root);
    cleanup(&first_path);
    std::fs::remove_dir(other_path).unwrap();
}

#[test]
fn current_profile_rejects_a_stored_root_that_is_not_the_pinned_root() {
    let _guard = route_b_test_guard();
    let path = scratch("changed-profile");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    initialize_profile(&mut connection, &root).unwrap();
    connection.execute("UPDATE gogoke_authority_profile SET root_identity='forged-root'").unwrap();
    let result = transaction::run(&mut connection, |tx| current_profile(tx).map(|_| ()));
    assert!(matches!(result, Err(OrchestrationError::AccessDenied)));
    assert!(matches!(initialize_profile(&mut connection, &root), Err(OrchestrationError::AccessDenied)));
    transaction::run(&mut connection, |tx| {
        assert_eq!(tx.query("SELECT root_identity FROM gogoke_authority_profile", &[], 1)?[0][0], "forged-root");
        Ok(())
    }).unwrap();
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}
