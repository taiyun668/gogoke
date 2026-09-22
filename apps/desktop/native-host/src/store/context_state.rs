//! Monotonic revision index for the EXISTING Context lifecycle state authority.
//! gogoke_context_states remains the only state truth. This companion stores only
//! the revision of that truth so reads/assembly can detect ABA without copying state.
use super::atomic::Statement;
use super::orchestration::OrchestrationError;
use super::same_open::VerifiedDatabaseConnection;

type Result<T> = std::result::Result<T, OrchestrationError>;

const SCHEMA: &str = "CREATE TABLE gogoke_context_state_revisions (domain_id TEXT NOT NULL,version_ref TEXT NOT NULL,state_revision TEXT NOT NULL CHECK(length(state_revision) BETWEEN 1 AND 20 AND state_revision NOT GLOB '*[^0-9]*' AND substr(state_revision,1,1)<>'0'),PRIMARY KEY(domain_id,version_ref),FOREIGN KEY(domain_id,version_ref) REFERENCES gogoke_context_states(domain_id,version_ref) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";
const STATES: &[&str] = &["ACTIVE", "SUPERSEDED", "CONFLICTED", "STALE", "REVOKED", "ARCHIVED"];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct StateSnapshot {
    pub state: String,
    pub revision: String,
}

fn invalid<T>(detail: &'static str) -> Result<T> {
    Err(OrchestrationError::Invalid(detail))
}

fn require_transaction(connection: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    unsafe extern "C" {
        fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
    if unsafe { sqlite3_get_autocommit(connection.as_ptr()) } != 0 {
        return invalid("Context state revision requires owning transaction");
    }
    Ok(())
}

fn valid_state(value: &str) -> bool {
    STATES.contains(&value)
}

fn allowed_transition(expected: &str, next: &str) -> bool {
    matches!(
        (expected, next),
        ("ACTIVE", "SUPERSEDED") | ("ACTIVE", "STALE") | ("ACTIVE", "ARCHIVED")
    )
}

fn parse_revision(value: &str) -> Result<u64> {
    if value.is_empty() || value.len() > 20 || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return invalid("Context state revision is not canonical");
    }
    let parsed = value.parse::<u64>()
        .map_err(|_| OrchestrationError::Invalid("Context state revision overflow"))?;
    if parsed == 0 {
        return invalid("Context state revision must be positive");
    }
    Ok(parsed)
}

fn execute(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> Result<()> {
    Statement::prepare(connection.as_ptr(), sql)?.step_done()?;
    Ok(())
}

pub(super) fn ensure_schema(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    require_transaction(connection)?;
    let schema = Statement::prepare(
        connection.as_ptr(),
        "SELECT type,sql FROM sqlite_schema WHERE name='gogoke_context_state_revisions'",
    )?;
    let created = if schema.step_row()? {
        if schema.column_text(0)? != "table" || schema.column_text(1)? != SCHEMA || schema.step_row()? {
            return invalid("Context state revision schema mismatch");
        }
        false
    } else {
        drop(schema);
        execute(connection, SCHEMA)?;
        true
    };
    let trigger = Statement::prepare(
        connection.as_ptr(),
        "SELECT name FROM sqlite_schema WHERE type='trigger' AND tbl_name='gogoke_context_state_revisions' LIMIT 1",
    )?;
    if trigger.step_row()? {
        return invalid("Context state revision trigger not allowed");
    }
    drop(trigger);

    // Backfill only while introducing the revision table. Once the table exists,
    // a missing row is corruption and must fail closed instead of resetting history.
    if created {
        execute(
            connection,
            "INSERT INTO gogoke_context_state_revisions(domain_id,version_ref,state_revision) SELECT s.domain_id,s.version_ref,'1' FROM gogoke_context_states s",
        )?;
    }
    for sql in [
        "SELECT 1 FROM gogoke_context_states s LEFT JOIN gogoke_context_state_revisions r ON r.domain_id=s.domain_id AND r.version_ref=s.version_ref WHERE r.version_ref IS NULL LIMIT 1",
        "SELECT 1 FROM gogoke_context_state_revisions r LEFT JOIN gogoke_context_states s ON s.domain_id=r.domain_id AND s.version_ref=r.version_ref WHERE s.version_ref IS NULL LIMIT 1",
    ] {
        if Statement::prepare(connection.as_ptr(), sql)?.step_row()? {
            return invalid("Context state revision coverage mismatch");
        }
    }
    Ok(())
}

pub(super) fn initialize_context_state_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<()> {
    connection.execute("BEGIN IMMEDIATE")
        .map_err(|error| OrchestrationError::Atomic(error.into()))?;
    match ensure_schema(connection) {
        Ok(()) => {
            if connection.execute("COMMIT").is_err() {
                return Err(OrchestrationError::CommitUnknown);
            }
            Ok(())
        }
        Err(error) => {
            if connection.execute("ROLLBACK").is_err() {
                return Err(OrchestrationError::CommitUnknown);
            }
            Err(error)
        }
    }
}

pub(super) fn insert_initial(
    connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str, version_ref: &str,
) -> Result<()> {
    ensure_schema(connection)?;
    let state = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO gogoke_context_states(domain_id,version_ref,state) VALUES (?,?,'ACTIVE')",
    )?;
    state.bind_text(1, domain_id)?;
    state.bind_text(2, version_ref)?;
    state.step_done()?;
    let revision = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO gogoke_context_state_revisions(domain_id,version_ref,state_revision) VALUES (?,?,'1')",
    )?;
    revision.bind_text(1, domain_id)?;
    revision.bind_text(2, version_ref)?;
    revision.step_done()?;
    Ok(())
}

pub(super) fn current(
    connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str, version_ref: &str,
) -> Result<StateSnapshot> {
    ensure_schema(connection)?;
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT s.state,r.state_revision FROM gogoke_context_states s JOIN gogoke_context_state_revisions r ON r.domain_id=s.domain_id AND r.version_ref=s.version_ref WHERE s.domain_id=? AND s.version_ref=?",
    )?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, version_ref)?;
    if !statement.step_row()? {
        return invalid("Context state missing");
    }
    let state = statement.column_text(0)?;
    let revision = statement.column_text(1)?;
    if statement.step_row()? || !valid_state(&state) {
        return invalid("Context state identity mismatch");
    }
    parse_revision(&revision)?;
    Ok(StateSnapshot { state, revision })
}

pub(super) fn transition(
    connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str, version_ref: &str,
    expected: &str, next: &str,
) -> Result<StateSnapshot> {
    if !valid_state(expected) || !valid_state(next) {
        return invalid("Context state transition value");
    }
    let before = current(connection, domain_id, version_ref)?;
    if before.state != expected {
        return invalid("Context state transition mismatch");
    }
    if expected == next {
        return Ok(before);
    }
    // Only transitions with an implemented S1 operation are admitted. The
    // presence of a lifecycle enum value is not authority to transition to it.
    if !allowed_transition(expected, next) {
        return invalid("Context state transition not admitted");
    }
    let next_revision = parse_revision(&before.revision)?.checked_add(1)
        .ok_or(OrchestrationError::Invalid("Context state revision overflow"))?
        .to_string();

    let state = Statement::prepare(
        connection.as_ptr(),
        "UPDATE gogoke_context_states SET state=? WHERE domain_id=? AND version_ref=? AND state=?",
    )?;
    state.bind_text(1, next)?;
    state.bind_text(2, domain_id)?;
    state.bind_text(3, version_ref)?;
    state.bind_text(4, expected)?;
    state.step_done()?;

    let revision = Statement::prepare(
        connection.as_ptr(),
        "UPDATE gogoke_context_state_revisions SET state_revision=? WHERE domain_id=? AND version_ref=? AND state_revision=?",
    )?;
    revision.bind_text(1, &next_revision)?;
    revision.bind_text(2, domain_id)?;
    revision.bind_text(3, version_ref)?;
    revision.bind_text(4, &before.revision)?;
    revision.step_done()?;

    let after = current(connection, domain_id, version_ref)?;
    if after.state != next || after.revision != next_revision {
        return invalid("Context state transition did not persist");
    }
    Ok(after)
}

pub(super) fn transition_active_to_stale(
    connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str, version_ref: &str,
) -> Result<bool> {
    let before = current(connection, domain_id, version_ref)?;
    if before.state != "ACTIVE" {
        return Ok(false);
    }
    transition(connection, domain_id, version_ref, "ACTIVE", "STALE")?;
    Ok(true)
}
