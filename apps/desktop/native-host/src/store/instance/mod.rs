//! Design 37 global instance storage. No caller-facing v37 dispatch exists yet.
//! This schema shares the verified product connection but never changes R2 authority tables.
mod home;
mod catalog;
mod provider_catalog;
pub(crate) mod provider_login;
mod cap;
mod management;
mod managed_cli;
mod program_source;
mod registry;
mod resolver;
mod reprobe;
mod temporary;
mod private_history;
mod credential_registry;
mod grok_home_grants;
mod legacy_fence;
pub(crate) mod holder_disappearance;

pub(crate) use legacy_fence::{initialize_legacy_fence_schema, capture_legacy_fence,
    read_legacy_fence, read_legacy_step, begin_legacy_acl_step, validate_legacy_acl_write, finish_legacy_acl_step,
    LegacyFenceCapture, LegacyFenceRecord, LegacyCustodyRow, LegacyAclStep,
    LegacyStepPhase, LegacyPhysicalProof, LegacyStepRequest, LegacyStepFinish};

pub(crate) use home::{prepare_persistent_home, HomeError, PreparedInstanceHome};
pub(crate) use catalog::{discover_program, known_new_version, locate_pinned_program, CatalogError};
pub(crate) use cap::{read_instance_concurrency_cap, set_instance_concurrency_cap};
pub(crate) use management::{read_instance_profiles, set_instance_profile, record_qualified_account,
    read_instance_evidence, tombstone_unused_instance, InstanceProfile, InstanceEvidence,
    InstanceManagementError, QualifiedAccount, QualifiedAccountSource};
pub(crate) use managed_cli::{managed_cli_root, inspect_staged_official_cli, read_managed_cli,
    record_managed_cli_stage, confirm_managed_cli_launch, record_managed_cli_failure,
    record_managed_cli_progress, record_official_cli_notice, read_fixed_official_cli,
    locate_ready_managed_program, locate_ready_managed_program_from_db,
    uninstall_managed_cli, ManagedCliCopy, ManagedCliError, VerifiedOfficialCli};
pub(crate) use program_source::{bind_managed_instance_program, migrate_quiescent_legacy_instances,
    locate_bound_instance_program,
    ProgramSourceError};
pub(crate) use registry::{preflight_register_request, reconcile_register_replay,
    register_instance, record_observation, repin_program, reconcile_program_repin,
    InstanceObservation, ObservationRequest, ProgramObservation, Registration,
    RegistrationDisposition, RegistrationJournalPhase, RegistrationPreflight,
    RegistrationReplay, RegistryError, ProgramRepin, ProgramRepinReceipt};
pub(crate) use resolver::{resolve_codex_instance_home,
    resolve_codex_session_launch_homes, resolve_provider_session_launch_homes, InstanceLaunchHomes, LaunchHomeError,
    ResolvedDirectory};
pub(crate) use reprobe::{read_current_capability_reprobe, CapabilityReprobeEvidence};
pub(crate) use temporary::{create_temporary_home, close_temporary_home, cleanup_temporary_home,
    CreateTemporaryHome, TemporaryHomeError, TemporaryHomeReceipt, TemporaryKind,
    TransitionTemporaryHome};
pub(crate) use private_history::{create_initial_private_history, resume_private_history,
    resolve_private_history_directory,
    read_private_history_generation, bind_private_history_continuation_in_transaction,
    verify_private_history, PrivateHistoryLaunch, PrivateHistorySource,
    PrivateHistoryGeneration, StoppedPrivateHistory, PrivateHistoryReceipt, PrivateHistoryError};
pub(crate) use credential_registry::{initialize_credential_schema, record_credential_backend,
    read_configured_credential_backend, read_usable_credential_backend, bind_credential_object, read_credential_object,
    read_credential_aliases, begin_credential_alias, complete_credential_alias,
    recover_pending_credential_remove,
    read_credential_profiles, begin_credential_profile, complete_credential_profile,
    read_completed_profile_revoke,
    CredentialRegistryError, CredentialBackend, CredentialStartupSelector, BackendSource,
    CredentialObjectInput, CredentialObjectRecord, CredentialIntentDisposition,
    CredentialAliasAction, CredentialAliasResult, CredentialAliasIntent,
    CredentialAliasRecord, CredentialAliasIntentReceipt, CredentialAliasPhysicalReceipt,
    CredentialProfileAction, CredentialProfileResult, CredentialProfileIntent,
    CredentialProfileRecord, CredentialProfileIntentReceipt};
pub(crate) use grok_home_grants::{initialize_grok_home_grant_schema,
    current_domain as current_grok_home_domain, begin_grok_grant, read_grok_grants,
    begin_grok_effect, finish_grok_effect, read_grok_effects, set_grok_grant_phase,
    bind_grok_original_process,
    GrokDomain, GrokGrant, GrokEffect};

pub(crate) fn resolve_grok_original_home(db:&VerifiedDatabaseConnection<'_>,
    root:&crate::root::RootLock,instance_id:&str)->Result<ResolvedDirectory,registry::RegistryError>{
    let (path,identity)=registry::resolve_registered_provider_home(db,root,instance_id,"grok")?;
    Ok(ResolvedDirectory{path,identity})
}

use super::atomic::Statement;
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

const SCHEMA: [(&str, &str); 10] = [
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
    (
        "gogoke_v37_instance_caps",
        "CREATE TABLE gogoke_v37_instance_caps(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),concurrency_cap INTEGER NOT NULL CHECK(concurrency_cap > 0)) STRICT",
    ),
    ("gogoke_v37_instance_histories", private_history::HISTORY_SCHEMA),
    ("gogoke_v37_instance_history_generations", private_history::GENERATIONS_SCHEMA),
    ("gogoke_v37_instance_profiles", "CREATE TABLE gogoke_v37_instance_profiles(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),display_name TEXT NOT NULL,enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),connected_model_source TEXT,tombstoned INTEGER NOT NULL DEFAULT 0 CHECK(tombstoned IN (0,1)),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_instance_evidence", "CREATE TABLE gogoke_v37_instance_evidence(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),account_masked TEXT,subscription TEXT,account_confirmed_at TEXT,account_source TEXT,available_models_json TEXT,models_source TEXT,models_observed_at TEXT,models_program_digest TEXT,detect_error TEXT,detect_error_at TEXT) STRICT"),
    ("gogoke_v37_instance_cli_copies", "CREATE TABLE gogoke_v37_instance_cli_copies(driver_id TEXT PRIMARY KEY,state TEXT NOT NULL CHECK(state IN ('NOT_INSTALLED','DOWNLOADING','INSTALLING','UPGRADING','STAGED','PROBED','READY','INSTALL_FAILED','BLOCKED','PROBE_UNKNOWN','UPGRADE_FAILED','UNINSTALLING')),version TEXT,archive_sha256 TEXT,image_sha256 TEXT,stage_name TEXT,previous_version TEXT,previous_image_sha256 TEXT,previous_stage_name TEXT,progress_bytes INTEGER NOT NULL DEFAULT 0,raw_error TEXT,checked_at TEXT,official_notice TEXT,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
    ("gogoke_v37_instance_program_sources", "CREATE TABLE gogoke_v37_instance_program_sources(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),source TEXT NOT NULL CHECK(source='MANAGED'),stage_name TEXT NOT NULL,program_digest TEXT NOT NULL,version TEXT NOT NULL,home_identity TEXT NOT NULL,registration_request_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT"),
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

fn previous_schema() -> Vec<(String, String)> {
    let mut entries: Vec<_> = SCHEMA[..3]
        .iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned()))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn pre_history_schema() -> Vec<(String, String)> {
    let mut entries: Vec<_> = SCHEMA[..4].iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
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
    if observed == expected {
        return Ok(());
    }
    if !observed.is_empty() && observed != previous_schema() && observed != pre_history_schema()
        && observed != sorted_schema_prefix(6) && observed != sorted_schema_prefix(7)
        && observed != sorted_schema_prefix(8) && observed != sorted_schema_prefix(9) {
        return Err(OrchestrationError::AccessDenied);
    }
    connection
        .execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    let created = (|| {
        // Recheck inside the write transaction in case another trusted opener won.
        reject_shadow_or_side_effect_objects(connection)?;
        if observed_schema(connection)? != observed {
            return Err(OrchestrationError::AccessDenied);
        }
        let to_create: &[(&str, &str)] = &SCHEMA[observed.len()..];
        for (_, sql) in to_create {
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

fn sorted_schema_prefix(count: usize) -> Vec<(String, String)> {
    let mut entries: Vec<_> = SCHEMA[..count].iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
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
    fn exact_pre_cap_schema_migrates_without_setting_cap_or_losing_instance() {
        fixture(|connection| {
            for (_, sql) in &SCHEMA[..3] {
                connection.execute(sql).unwrap();
            }
            connection.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
            assert_eq!(observed_schema(connection).unwrap(), previous_schema());
            initialize_schema(connection).unwrap();
            assert_eq!(observed_schema(connection).unwrap(), expected_schema());
            assert!(matches!(read_instance_concurrency_cap(connection, "instanceA"),
                Err(OrchestrationError::AccessDenied)));
            let query = Statement::prepare(connection.as_ptr(),
                "SELECT login_state,home_identity FROM main.gogoke_v37_instances WHERE instance_id='instanceA'").unwrap();
            assert!(query.step_row().unwrap());
            assert_eq!(query.column_text(0).unwrap(), "LOGGED_IN");
            assert_eq!(query.column_text(1).unwrap(), "identityA");
            initialize_schema(connection).unwrap();
        });
    }

    #[test]
    fn exact_four_table_schema_migrates_preserving_instance_and_cap() {
        fixture(|connection| {
            for (_, sql) in &SCHEMA[..4] { connection.execute(sql).unwrap(); }
            connection.execute("INSERT INTO main.gogoke_v37_instances VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
            connection.execute("INSERT INTO main.gogoke_v37_instance_caps VALUES('instanceA',3)").unwrap();
            assert_eq!(observed_schema(connection).unwrap(), pre_history_schema());
            initialize_schema(connection).unwrap();
            assert_eq!(observed_schema(connection).unwrap(), expected_schema());
            assert_eq!(read_instance_concurrency_cap(connection, "instanceA").unwrap(), 3);
            let query = Statement::prepare(connection.as_ptr(),
                "SELECT home_identity,login_state FROM main.gogoke_v37_instances WHERE instance_id='instanceA'").unwrap();
            assert!(query.step_row().unwrap());
            assert_eq!(query.column_text(0).unwrap(), "identityA");
            assert_eq!(query.column_text(1).unwrap(), "LOGGED_IN");
            initialize_schema(connection).unwrap();
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
