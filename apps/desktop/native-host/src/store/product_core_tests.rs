//! Actual Route-B definitions; not executed by the SQL diagnostic instrument.
use super::*;
use crate::root::RootLock;
use crate::store::atomic::{admit_core_schema, apply_core_schema};
use crate::store::product_database::ProductDatabase;
use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
use std::time::{SystemTime, UNIX_EPOCH};

fn root_path() -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("gogoke-product-core-{}-{nonce}", std::process::id()))
}
fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
    let _guard = route_b_test_guard();
    let path = root_path();
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    let mut connection = create_new(&root, &database).unwrap();
    for sql in ["PRAGMA foreign_keys=ON", "PRAGMA journal_mode=WAL", "PRAGMA synchronous=FULL"] {
        connection.execute(sql).unwrap();
    }
    run(&mut connection);
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {} ({error})", path.display()); }
}
fn scalar(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> String {
    let statement = Statement::prepare(connection.as_ptr(), sql).unwrap();
    assert!(statement.step_row().unwrap());
    let value = statement.column_text(0).unwrap();
    assert!(!statement.step_row().unwrap());
    value
}
fn seed(connection: &mut VerifiedDatabaseConnection<'_>) {
    for sql in core_schema_statements() { connection.execute(&sql).unwrap(); }
}

#[test]
fn mixed_product_tables_keep_their_existing_rows() {
    fixture(|c| {
        c.execute("CREATE TABLE other_family(id TEXT PRIMARY KEY,value TEXT) STRICT").unwrap();
        c.execute("INSERT INTO other_family VALUES ('one','keep')").unwrap();
        initialize_product_core_schema(c).unwrap();
        assert_eq!(existing_count(c, &expected_schema().unwrap()).unwrap(), 7);
        assert_eq!(scalar(c, "SELECT value FROM other_family WHERE id='one'"), "keep");
    });
}

#[test]
fn repeated_initialization_retains_core_counter_and_schema() {
    fixture(|c| {
        initialize_product_core_schema(c).unwrap();
        c.execute("INSERT INTO gogoke_stream_heads(domain_id,stream_id,counter) VALUES ('d','s','18446744073709551615')").unwrap();
        initialize_product_core_schema(c).unwrap();
        assert_eq!(scalar(c, "SELECT counter FROM gogoke_stream_heads WHERE domain_id='d' AND stream_id='s'"), "18446744073709551615");
    });
}

#[test]
fn standalone_admission_still_rejects_other_tables() {
    fixture(|c| {
        apply_core_schema(c).unwrap();
        c.execute("CREATE TABLE unrelated(id TEXT) STRICT").unwrap();
        assert!(admit_core_schema(c).is_err());
        initialize_product_core_schema(c).unwrap();
        assert!(admit_core_schema(c).is_err());
    });
}

#[test]
fn partial_core_family_is_not_filled_in() {
    fixture(|c| {
        c.execute(&core_schema_statements()[0]).unwrap();
        assert!(initialize_product_core_schema(c).is_err());
        assert_eq!(existing_count(c, &expected_schema().unwrap()).unwrap(), 1);
    });
}

#[test]
fn missing_existing_index_is_not_silently_recreated() {
    fixture(|c| {
        seed(c); c.execute("DROP INDEX idx_gogoke_receipts_event").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
        assert_eq!(existing_count(c, &expected_schema().unwrap()).unwrap(), 6);
    });
}

#[test]
fn changed_quoted_check_value_is_not_schema_equivalent() {
    fixture(|c| {
        for sql in core_schema_statements() {
            c.execute(&sql.replace("counter = '0'", "counter = ' 0 '")).unwrap();
        }
        assert!(initialize_product_core_schema(c).is_err());
    });
}

#[test]
fn view_cannot_impersonate_a_core_table() {
    fixture(|c| {
        c.execute("CREATE VIEW gogoke_objects AS SELECT 1 AS fake").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
        assert_eq!(scalar(c, "SELECT type FROM main.sqlite_schema WHERE name='gogoke_objects'"), "view");
    });
}

#[test]
fn main_trigger_is_rejected_without_running_it() {
    fixture(|c| {
        seed(c);
        c.execute("CREATE TRIGGER core_effect AFTER INSERT ON gogoke_objects BEGIN SELECT 1; END").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
    });
}

#[test]
fn unknown_explicit_index_is_not_admitted() {
    fixture(|c| {
        seed(c);
        c.execute("CREATE UNIQUE INDEX unexpected_core_index ON gogoke_objects(object_id)").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
    });
}

#[test]
fn temporary_case_variant_table_cannot_shadow_main() {
    fixture(|c| {
        seed(c);
        c.execute("CREATE TEMP TABLE GOGOKE_OBJECTS(fake TEXT)").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
    });
}

#[test]
fn temporary_trigger_on_main_is_rejected() {
    fixture(|c| {
        seed(c);
        c.execute("CREATE TEMP TRIGGER temp_effect AFTER INSERT ON main.gogoke_objects BEGIN SELECT 1; END").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
    });
}

#[test]
fn caller_transaction_is_not_committed_or_rolled_back() {
    fixture(|c| {
        c.execute("CREATE TABLE caller_witness(id TEXT)").unwrap();
        c.execute("BEGIN IMMEDIATE").unwrap();
        c.execute("INSERT INTO caller_witness VALUES ('owned')").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
        assert_eq!(scalar(c, "SELECT id FROM caller_witness"), "owned");
        c.execute("ROLLBACK").unwrap();
        assert_eq!(scalar(c, "SELECT count(*) FROM caller_witness"), "0");
    });
}

#[test]
fn weakened_durability_is_not_silently_repaired() {
    fixture(|c| {
        c.execute("PRAGMA synchronous=NORMAL").unwrap();
        assert!(initialize_product_core_schema(c).is_err());
        assert_eq!(scalar(c, "PRAGMA synchronous"), "1");
        assert_eq!(existing_count(c, &expected_schema().unwrap()).unwrap(), 0);
    });
}

#[test]
fn actual_product_open_composes_core_and_keeps_one_profile_on_reopen() {
    let _guard = route_b_test_guard();
    let path = root_path();
    std::fs::create_dir(&path).unwrap();
    let root = RootLock::acquire(&path).unwrap();
    let database = path.join("state.sqlite");
    ProductDatabase::open(&root, &database).unwrap().close_checked().unwrap();
    ProductDatabase::open(&root, &database).unwrap().close_checked().unwrap();
    let connection = open_existing(&root, &database).unwrap();
    assert_eq!(existing_count(&connection, &expected_schema().unwrap()).unwrap(), 7);
    assert_eq!(scalar(&connection, "SELECT count(*) FROM gogoke_authority_profile"), "1");
    connection.close_checked().unwrap();
    drop(root);
    std::fs::remove_file(database).unwrap();
    if let Err(error) = std::fs::remove_dir(&path) { eprintln!("owned fixture retained: {} ({error})", path.display()); }
}
