//! Native runner for the fixed donor migration sequence.
//! Donor TypeScript is not rewritten. Ledger name matches Effect: effect_sql_migrations.

use super::atomic::{exec, Statement};
use super::donor_migrations::{DonorMigration, DynamicStep, DONOR_MIGRATIONS};
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

pub const LATEST_DONOR_MIGRATION: i64 = 53;

pub fn apply_donor_migrations(
    connection: &mut VerifiedDatabaseConnection<'_>,
    through: i64,
) -> Result<Vec<(i64, &'static str)>, OrchestrationError> {
    if through < 1 || through > LATEST_DONOR_MIGRATION {
        return Err(OrchestrationError::Invalid("migration bound"));
    }
    exec(
        connection,
        "CREATE TABLE IF NOT EXISTS effect_sql_migrations (
            migration_id INTEGER NOT NULL PRIMARY KEY,
            created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
            name TEXT NOT NULL
        )",
    )?;
    let applied = applied_ledger(connection)?;
    if let Some(&(id, ref name)) = applied.last() {
        if id > LATEST_DONOR_MIGRATION || id > through {
            return Err(OrchestrationError::Invalid("unknown newer schema"));
        }
        let expected = DONOR_MIGRATIONS
            .iter()
            .find(|item| item.id == id)
            .ok_or(OrchestrationError::Invalid("malformed migration ledger"))?;
        if expected.name != name {
            return Err(OrchestrationError::Invalid("migration slot collision"));
        }
    }
    let mut ran = Vec::new();
    for migration in DONOR_MIGRATIONS.iter().filter(|item| item.id <= through) {
        if applied.iter().any(|(id, _)| *id == migration.id) {
            continue;
        }
        exec(connection, "BEGIN IMMEDIATE")?;
        let result = apply_one(connection, migration);
        match result {
            Ok(()) => {
                let statement = Statement::prepare(
                    connection.as_ptr(),
                    "INSERT INTO effect_sql_migrations (migration_id, name) VALUES (?, ?)",
                )?;
                statement.bind_i64(1, migration.id)?;
                statement.bind_text(2, migration.name)?;
                statement.step_done()?;
                if let Err(error) = exec(connection, "COMMIT") {
                    let _ = connection.execute("ROLLBACK");
                    return Err(error.into());
                }
                ran.push((migration.id, migration.name));
            }
            Err(error) => {
                let _ = connection.execute("ROLLBACK");
                return Err(error);
            }
        }
    }
    exec(connection, "PRAGMA foreign_keys = ON")?;
    exec(connection, "PRAGMA journal_mode = WAL")?;
    exec(connection, "PRAGMA synchronous = FULL")?;
    Ok(ran)
}

fn applied_ledger(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<Vec<(i64, String)>, OrchestrationError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT migration_id, name FROM effect_sql_migrations ORDER BY migration_id",
    )?;
    let mut rows = Vec::new();
    while statement.step_row()? {
        let id = statement
            .column_text(0)?
            .parse::<i64>()
            .map_err(|_| OrchestrationError::Invalid("migration_id"))?;
        rows.push((id, statement.column_text(1)?));
    }
    Ok(rows)
}

fn apply_one(
    connection: &mut VerifiedDatabaseConnection<'_>,
    migration: &DonorMigration,
) -> Result<(), OrchestrationError> {
    for statement in migration.statements {
        exec(connection, statement)?;
    }
    match migration.dynamic {
        DynamicStep::None => Ok(()),
        DynamicStep::CopyLegacyThreadPullRequests => copy_legacy_pull_requests(connection),
    }
}

fn copy_legacy_pull_requests(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), OrchestrationError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT thread_id, updated_at, linked_pull_request_json
         FROM projection_threads
         WHERE linked_pull_request_json IS NOT NULL",
    )?;
    let mut rows = Vec::new();
    while statement.step_row()? {
        rows.push((
            statement.column_text(0)?,
            statement.column_text(1)?,
            statement.column_text(2)?,
        ));
    }
    drop(statement);
    for (thread_id, updated_at, json) in rows {
        let Some((host, repository, number, url)) = parse_legacy_pull_request(&json) else {
            continue;
        };
        let insert = Statement::prepare(
            connection.as_ptr(),
            "INSERT OR IGNORE INTO projection_thread_pull_requests (
                thread_id, host, repository, number, url, source, linked_at, snapshot_json, stack_json
             ) VALUES (?, ?, ?, ?, ?, 'manual', ?, NULL, NULL)",
        )?;
        insert.bind_text(1, &thread_id)?;
        insert.bind_text(2, &host)?;
        insert.bind_text(3, &repository)?;
        insert.bind_i64(4, number)?;
        insert.bind_text(5, &url)?;
        insert.bind_text(6, &updated_at)?;
        insert.step_done()?;
    }
    Ok(())
}

fn parse_legacy_pull_request(json: &str) -> Option<(String, String, i64, String)> {
    let repository = json_string_field(json, "repository")?;
    let url = json_string_field(json, "url")?;
    let number = json_number_field(json, "number")?;
    if number < 1 {
        return None;
    }
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("unknown")
        .trim()
        .to_ascii_lowercase();
    Some((
        if host.is_empty() { "unknown".into() } else { host },
        repository.trim().to_ascii_lowercase(),
        number,
        url,
    ))
}

fn json_string_field(json: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\":\"");
    let start = json.find(&needle)? + needle.len();
    let rest = json.get(start..)?;
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

fn json_number_field(json: &str, field: &str) -> Option<i64> {
    let needle = format!("\"{field}\":");
    let start = json.find(&needle)? + needle.len();
    let rest = json.get(start..)?.trim_start();
    let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub fn table_exists(
    connection: &mut VerifiedDatabaseConnection<'_>,
    name: &str,
) -> Result<bool, OrchestrationError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?",
    )?;
    statement.bind_text(1, name)?;
    Ok(statement.step_row()?)
}

pub fn column_exists(
    connection: &mut VerifiedDatabaseConnection<'_>,
    table: &str,
    column: &str,
) -> Result<bool, OrchestrationError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!("SELECT 1 FROM pragma_table_info('{table}') WHERE name = ?"),
    )?;
    statement.bind_text(1, column)?;
    Ok(statement.step_row()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(label: &str) -> (RootLock, std::path::PathBuf, std::path::PathBuf) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-mig-{label}-{nonce}"));
        std::fs::create_dir(&root_path).expect("root");
        let root = RootLock::acquire(&root_path).expect("lock");
        let database_path = root_path.join("main.db");
        (root, root_path, database_path)
    }

    fn cleanup(root: RootLock, root_path: std::path::PathBuf, database_path: std::path::PathBuf) {
        drop(root);
        std::fs::remove_file(&database_path).ok();
        let _ = std::fs::remove_file(format!("{}-wal", database_path.display()));
        let _ = std::fs::remove_file(format!("{}-shm", database_path.display()));
        std::fs::remove_dir(&root_path).ok();
    }

    #[test]
    fn prefixes_and_full_sequence_match_the_fixed_manifest() {
        let _guard = route_b_test_guard();
        let (root, root_path, database_path) = scratch("prefix");
        let mut connection = create_new(&root, &database_path).expect("open");
        let ran = apply_donor_migrations(&mut connection, 5).expect("through 5");
        assert_eq!(ran.len(), 5);
        assert!(table_exists(&mut connection, "orchestration_events").unwrap());
        assert!(table_exists(&mut connection, "projection_projects").unwrap());
        assert!(!column_exists(&mut connection, "projection_threads", "archived_at").unwrap());
        let more = apply_donor_migrations(&mut connection, 53).expect("through 53");
        assert_eq!(more.len(), 48);
        assert!(column_exists(&mut connection, "projection_threads", "archived_at").unwrap());
        assert!(table_exists(&mut connection, "pull_request_files_viewed").unwrap());
        let ledger = applied_ledger(&mut connection).expect("ledger");
        assert_eq!(ledger.len(), 53);
        assert_eq!(ledger[8], (9, "ProviderSessionRuntimeMode".into()));
        assert_eq!(ledger[52], (53, "PullRequestFilesViewed".into()));
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn canonicalize_model_selection_rewrites_rows() {
        let _guard = route_b_test_guard();
        let (root, root_path, database_path) = scratch("016");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_donor_migrations(&mut connection, 5).expect("base");
        connection
            .execute(
                "INSERT INTO projection_projects (
                    project_id, title, workspace_root, default_model, scripts_json, created_at, updated_at, deleted_at
                 ) VALUES ('p1', 't', 'C:/w', 'claude-3', '[]', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', NULL)",
            )
            .expect("seed");
        apply_donor_migrations(&mut connection, 16).expect("016");
        assert!(!column_exists(&mut connection, "projection_projects", "default_model").unwrap());
        assert!(
            column_exists(&mut connection, "projection_projects", "default_model_selection_json")
                .unwrap()
        );
        let statement = Statement::prepare(
            connection.as_ptr(),
            "SELECT default_model_selection_json FROM projection_projects WHERE project_id = 'p1'",
        )
        .expect("select");
        assert!(statement.step_row().unwrap());
        let json = statement.column_text(0).unwrap();
        assert!(json.contains("claudeAgent"), "{json}");
        assert!(json.contains("claude-3"), "{json}");
        drop(statement);
        let ledger = crate::store::same_open::open_ledger().expect("ledger");
        assert_eq!(ledger.stock_main_open, 0, "{ledger:?}");
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn migration_failure_and_unknown_newer_schema_preserve_the_original() {
        let _guard = route_b_test_guard();
        let (root, root_path, database_path) = scratch("fail");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_donor_migrations(&mut connection, 2).expect("base");
        connection
            .execute(
                "INSERT INTO effect_sql_migrations (migration_id, name) VALUES (99, 'FutureUnknown')",
            )
            .expect("future");
        let error = apply_donor_migrations(&mut connection, 53).expect_err("newer");
        assert!(
            matches!(error, OrchestrationError::Invalid("unknown newer schema")),
            "{error:?}"
        );
        assert!(table_exists(&mut connection, "orchestration_events").unwrap());
        assert!(!table_exists(&mut connection, "projection_projects").unwrap());
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn slot_collision_refuses_without_rewriting_the_ledger() {
        let _guard = route_b_test_guard();
        let (root, root_path, database_path) = scratch("slot");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_donor_migrations(&mut connection, 1).expect("one");
        connection
            .execute("UPDATE effect_sql_migrations SET name = 'SomebodyElsesMigration' WHERE migration_id = 1")
            .expect("collide");
        let error = apply_donor_migrations(&mut connection, 2).expect_err("collision");
        assert!(
            matches!(error, OrchestrationError::Invalid("migration slot collision")),
            "{error:?}"
        );
        let ledger = applied_ledger(&mut connection).expect("ledger");
        assert_eq!(ledger, vec![(1, "SomebodyElsesMigration".into())]);
        assert!(!table_exists(&mut connection, "orchestration_command_receipts").unwrap());
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }

    #[test]
    fn old_reader_refuses_a_newer_applied_schema() {
        let _guard = route_b_test_guard();
        let (root, root_path, database_path) = scratch("old-reader");
        let mut connection = create_new(&root, &database_path).expect("open");
        apply_donor_migrations(&mut connection, 53).expect("new");
        let error = apply_donor_migrations(&mut connection, 5).expect_err("old bound still sees 53");
        assert!(
            matches!(error, OrchestrationError::Invalid("unknown newer schema")),
            "{error:?}"
        );
        assert_eq!(applied_ledger(&mut connection).unwrap().len(), 53);
        connection.close_checked().expect("close");
        cleanup(root, root_path, database_path);
    }
}