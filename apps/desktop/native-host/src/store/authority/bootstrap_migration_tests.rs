use super::super::catalog;
use super::super::transaction;
use super::{
    initialize_profile, migrate_v1_to_v2, validate_foreign_keys, validate_v1_profile,
    verify_schema_v1,
};
use crate::root::RootLock;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::{create_new, route_b_test_guard};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const V1: &str = include_str!("schema_v1.sql");

fn scratch(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "gogoke-authority-v1-{label}-{}-{nonce}",
        std::process::id()
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path.join("state.sqlite"));
    if let Err(error) = std::fs::remove_dir(path) {
        eprintln!("owned migration fixture retained: {error}");
    }
}

fn v1(connection: &mut crate::store::same_open::VerifiedDatabaseConnection<'_>, root: &RootLock) {
    connection.execute(V1).unwrap();
    let root_identity = root.canonical_root().identity.opaque();
    connection.execute(&format!(
        "INSERT INTO gogoke_authority_profile(singleton,schema_revision,profile_id,root_identity,owner_principal_id,owner_seat_id,issuer_id,policy_revision,revocation_head) VALUES(1,'1','profile-old','{root_identity}','principal-owner','seat-owner','issuer-root','1','0')"
    )).unwrap();
    let child_issuer = catalog::seat_issuer("principal-owner", "seat-owner");
    let mut inserts = String::new();
    for revision in 1..=70 {
        inserts.push_str(&format!(
            "INSERT INTO gogoke_authority_grants VALUES('grant-root','{revision}','principal-owner','seat-owner','context.read','NONE','domain-one','domain-one','PROJECT','issuer-root',NULL,NULL,'1','0',2);"
        ));
    }
    inserts.push_str(&format!(
        "INSERT INTO gogoke_authority_grants VALUES('grant-child','1','principal-child','seat-child','context.read','NONE','domain-one','domain-one','PROJECT','{child_issuer}','grant-root','70','1','0',1);"
    ));
    inserts.push_str("INSERT INTO gogoke_authority_grant_heads VALUES('grant-root','70',0);INSERT INTO gogoke_authority_grant_heads VALUES('grant-child','1',0);");
    inserts.push_str(
        "INSERT INTO gogoke_authority_events VALUES(1,'BOOTSTRAP','issuer-root','','0','1','0');",
    );
    for revision in 1..=70 {
        inserts.push_str(&format!(
            "INSERT INTO gogoke_authority_events VALUES({},'ISSUE','issuer-root','grant-root','{revision}','1','0');",
            revision + 1
        ));
    }
    inserts.push_str(&format!(
        "INSERT INTO gogoke_authority_events VALUES(72,'ISSUE','{child_issuer}','grant-child','1','1','0');"
    ));
    connection.execute(&inserts).unwrap();
}

#[test]
fn v1_migration_preserves_more_than_sixty_four_rows_parent_lineage_and_events() {
    let _guard = route_b_test_guard();
    let path = scratch("full");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    v1(&mut connection, &root);
    let owner = initialize_profile(&mut connection, &root).unwrap();
    assert_eq!(owner.principal_id(), "principal-owner");
    transaction::run(&mut connection, |tx| {
        let counts = tx.query(
            "SELECT (SELECT count(*) FROM main.gogoke_authority_grants),(SELECT count(*) FROM main.gogoke_authority_context_grant_payloads),(SELECT count(*) FROM main.gogoke_authority_events),(SELECT schema_revision FROM main.gogoke_authority_profile)",
            &[], 4)?;
        assert_eq!(counts, vec![vec![String::from("71"), String::from("71"), String::from("72"), String::from("2")]]);
        assert_eq!(tx.query("SELECT parent_grant_id,parent_revision FROM main.gogoke_authority_grants WHERE grant_id='grant-child'", &[], 2)?, vec![vec![String::from("grant-root"), String::from("70")]]);
        assert_eq!(tx.query("SELECT count(*) FROM main.sqlite_schema WHERE name LIKE '%_v1'", &[], 1)?[0][0], "0");
        assert_eq!(tx.query("SELECT count(*) FROM pragma_foreign_key_check", &[], 1)?[0][0], "0");
        Ok(())
    }).unwrap();
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn migration_failure_rolls_back_table_renames_and_all_copied_rows() {
    let _guard = route_b_test_guard();
    let path = scratch("rollback");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    v1(&mut connection, &root);
    let failed: transaction::Result<()> = transaction::run(&mut connection, |tx| {
        verify_schema_v1(tx)?;
        validate_v1_profile(tx, &root.canonical_root().identity.opaque())?;
        validate_foreign_keys(tx)?;
        migrate_v1_to_v2(tx)?;
        Err(OrchestrationError::AccessDenied)
    });
    assert!(failed.is_err());
    transaction::run(&mut connection, |tx| {
        verify_schema_v1(tx)?;
        assert_eq!(
            tx.query(
                "SELECT schema_revision FROM main.gogoke_authority_profile",
                &[],
                1
            )?[0][0],
            "1"
        );
        assert_eq!(
            tx.query("SELECT count(*) FROM main.gogoke_authority_grants", &[], 1)?[0][0],
            "71"
        );
        assert_eq!(
            tx.query(
                "SELECT count(*) FROM main.sqlite_schema WHERE name LIKE '%_v1'",
                &[],
                1
            )?[0][0],
            "0"
        );
        Ok(())
    })
    .unwrap();
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn partial_schema_and_temp_shadow_are_rejected_before_bootstrap() {
    let _guard = route_b_test_guard();
    let path = scratch("temp-shadow");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    connection
        .execute("CREATE TEMP TABLE gogoke_authority_profile(singleton INTEGER)")
        .unwrap();
    assert!(initialize_profile(&mut connection, &root).is_err());
    transaction::run(&mut connection, |tx| {
        assert_eq!(
            tx.query(
                "SELECT count(*) FROM main.sqlite_schema WHERE name GLOB 'gogoke_authority_*'",
                &[],
                1
            )?[0][0],
            "0"
        );
        Ok(())
    })
    .unwrap();
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn any_temp_trigger_targeting_an_authority_table_is_rejected() {
    let _guard = route_b_test_guard();
    let path = scratch("temp-trigger");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    initialize_profile(&mut connection, &root).unwrap();
    connection.execute(
        "CREATE TEMP TRIGGER unrelated_trigger_name AFTER INSERT ON main.GOGOKE_AUTHORITY_EVENTS BEGIN SELECT 1; END",
    ).unwrap();
    assert!(initialize_profile(&mut connection, &root).is_err());
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn a_partial_main_authority_schema_is_not_bootstrapped_over() {
    let _guard = route_b_test_guard();
    let path = scratch("partial-main");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    connection
        .execute("CREATE TABLE gogoke_authority_profile(singleton INTEGER, schema_revision TEXT)")
        .unwrap();
    assert!(initialize_profile(&mut connection, &root).is_err());
    transaction::run(&mut connection, |tx| {
        assert_eq!(tx.query("SELECT count(*) FROM main.sqlite_schema WHERE lower(substr(name,1,17))='gogoke_authority_'", &[], 1)?[0][0], "1");
        Ok(())
    }).unwrap();
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn uppercase_temp_authority_shadow_is_rejected_case_insensitively() {
    let _guard = route_b_test_guard();
    let path = scratch("uppercase-temp");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    initialize_profile(&mut connection, &root).unwrap();
    connection
        .execute("CREATE TEMP TABLE GOGOKE_AUTHORITY_GRANTS(grant_id TEXT)")
        .unwrap();
    assert!(initialize_profile(&mut connection, &root).is_err());
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}

#[test]
fn unexpected_v1_trigger_fails_closed_without_migration() {
    let _guard = route_b_test_guard();
    let path = scratch("trigger");
    let root = RootLock::acquire(&path).unwrap();
    let mut connection = create_new(&root, &path.join("state.sqlite")).unwrap();
    v1(&mut connection, &root);
    connection.execute("CREATE TRIGGER gogoke_authority_bad AFTER INSERT ON gogoke_authority_events BEGIN SELECT 1; END").unwrap();
    assert!(initialize_profile(&mut connection, &root).is_err());
    assert!(transaction::run(&mut connection, |tx| verify_schema_v1(tx)).is_err());
    connection.close_checked().unwrap();
    drop(root);
    cleanup(&path);
}
