//! Design 37 global instance storage. No caller-facing v37 dispatch exists yet.
//! This schema shares the verified product connection but never changes R2 authority tables.
mod home;
mod catalog;
mod registry;
mod temporary;

pub(crate) use home::{prepare_persistent_home, HomeError, PreparedInstanceHome};
pub(crate) use catalog::{discover_program, CatalogError};
pub(crate) use registry::{register_instance, record_observation, InstanceObservation,
    ObservationRequest, ProgramObservation, Registration, RegistrationDisposition, RegistryError};
pub(crate) use temporary::{create_temporary_home, close_temporary_home, cleanup_temporary_home,
    CreateTemporaryHome, TemporaryHomeError, TemporaryHomeReceipt, TemporaryKind,
    TransitionTemporaryHome};

use super::atomic::Statement;
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

const SCHEMA: [(&str, &str); 3] = [
    (
        "gogoke_v37_instances",
        "CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY, driver_id TEXT NOT NULL, home_ref TEXT NOT NULL UNIQUE, home_identity TEXT NOT NULL UNIQUE, program_digest TEXT NOT NULL, version TEXT NOT NULL, install_state TEXT NOT NULL CHECK(install_state IN ('UNKNOWN','INSTALLED','MISSING')), login_state TEXT NOT NULL CHECK(login_state IN ('UNKNOWN','LOGGED_IN','LOGGED_OUT')), revision INTEGER NOT NULL CHECK(revision >= 1)) STRICT",
    ),
    (
        "gogoke_v37_instance_homes",
        "CREATE TABLE gogoke_v37_instance_homes(home_id TEXT PRIMARY KEY, instance_id TEXT NOT NULL REFERENCES gogoke_v37_instances(instance_id), domain_id TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('SESSION','CALL')), owner_id TEXT NOT NULL, generation TEXT NOT NULL, directory_ref TEXT UNIQUE, directory_identity TEXT UNIQUE, state TEXT NOT NULL CHECK(state IN ('PREPARING','ACTIVE','CLOSE_UNKNOWN','CLOSED','CLEANUP_UNKNOWN','CLEANED','UNKNOWN')), revision INTEGER NOT NULL CHECK(revision >= 0)) STRICT",
    ),
    (
        "gogoke_v37_instance_operations",
        "CREATE TABLE gogoke_v37_instance_operations(request_id TEXT PRIMARY KEY, request_hex TEXT NOT NULL, target_id TEXT NOT NULL, phase TEXT NOT NULL CHECK(phase IN ('PREPARING','UNKNOWN','APPLIED','DENIED','FAILED')), receipt_json TEXT, native_receipt_id TEXT UNIQUE) STRICT",
    ),
];

fn observed_schema(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<Vec<(String, String)>, OrchestrationError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,19))='gogoke_v37_instance' ORDER BY name",
    )?;
    let mut rows = Vec::new();
    while statement.step_row()? {
        rows.push((statement.column_text(0)?, statement.column_text(1)?));
    }
    Ok(rows)
}

fn reject_shadow_or_side_effect_objects(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<(), OrchestrationError> {
    // A TEMP table can shadow an unqualified write, and a trigger may have an
    // arbitrary name. Check its target as well as names in both schemas.
    for query in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(substr(name,1,19))='gogoke_v37_instance' OR lower(substr(tbl_name,1,19))='gogoke_v37_instance' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(substr(tbl_name,1,19))='gogoke_v37_instance' LIMIT 1",
    ] {
        if Statement::prepare(connection.as_ptr(), query)?.step_row()? {
            return Err(OrchestrationError::AccessDenied);
        }
    }
    Ok(())
}

fn expected_schema() -> Vec<(String, String)> {
    let mut entries: Vec<_> = SCHEMA
        .iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

/// Called only after the existing native Owner issuer and root pin are verified.
/// An incomplete or changed family is corruption, not permission to recreate it.
pub(crate) fn initialize_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), OrchestrationError> {
    reject_shadow_or_side_effect_objects(connection)?;
    let expected = expected_schema();
    let observed = observed_schema(connection)?;
    if !observed.is_empty() {
        return if observed == expected {
            Ok(())
        } else {
            Err(OrchestrationError::AccessDenied)
        };
    }
    connection
        .execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let created = (|| {
        // Recheck inside the write transaction in case another trusted opener won.
        reject_shadow_or_side_effect_objects(connection)?;
        if !observed_schema(connection)?.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        for (_, sql) in SCHEMA {
            connection
                .execute(sql)
                .map_err(|error| OrchestrationError::Atomic(error.into()))?;
        }
        if observed_schema(connection)? != expected {
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(())
    })();
    match created {
        Ok(()) => connection
            .execute("COMMIT")
            .map_err(OrchestrationError::CommitUnknownWithCause),
        Err(error) => {
            connection
                .execute("ROLLBACK")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-v37-instance-schema-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        run(&mut connection);
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        if let Err(error) = std::fs::remove_dir(&path) {
            eprintln!("owned fixture retained: {} ({error})", path.display());
        }
    }

    #[test]
    fn creates_exact_schema_and_reopen_preserves_other_rows() {
        fixture(|connection| {
            connection
                .execute("CREATE TABLE unrelated(value TEXT) STRICT")
                .unwrap();
            connection
                .execute("INSERT INTO unrelated VALUES ('keep')")
                .unwrap();
            initialize_schema(connection).unwrap();
            initialize_schema(connection).unwrap();
            assert_eq!(observed_schema(connection).unwrap(), expected_schema());
            let statement = Statement::prepare(connection.as_ptr(), "SELECT value FROM unrelated").unwrap();
            assert!(statement.step_row().unwrap());
            assert_eq!(statement.column_text(0).unwrap(), "keep");
        });
    }

    #[test]
    fn partial_or_changed_family_is_rejected_without_repair() {
        fixture(|connection| {
            connection.execute(SCHEMA[0].1).unwrap();
            assert!(matches!(initialize_schema(connection), Err(OrchestrationError::AccessDenied)));
            assert_eq!(observed_schema(connection).unwrap().len(), 1);
        });
        fixture(|connection| {
            initialize_schema(connection).unwrap();
            connection.execute("DROP TABLE gogoke_v37_instance_operations").unwrap();
            connection.execute("CREATE TABLE gogoke_v37_instance_operations(request_id TEXT) STRICT").unwrap();
            assert!(matches!(initialize_schema(connection), Err(OrchestrationError::AccessDenied)));
        });
    }

    #[test]
    fn temp_shadow_and_arbitrarily_named_triggers_are_rejected() {
        fixture(|connection| {
            initialize_schema(connection).unwrap();
            connection.execute("CREATE TEMP TABLE gogoke_v37_instances(instance_id TEXT)").unwrap();
            assert!(matches!(initialize_schema(connection), Err(OrchestrationError::AccessDenied)));
            connection.execute("DROP TABLE temp.gogoke_v37_instances").unwrap();
            connection.execute("CREATE TRIGGER unrelated_name BEFORE INSERT ON gogoke_v37_instances BEGIN SELECT RAISE(ABORT,'blocked'); END").unwrap();
            assert!(matches!(initialize_schema(connection), Err(OrchestrationError::AccessDenied)));
        });
    }
}
