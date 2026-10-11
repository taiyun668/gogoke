//! A.1 ledger storage on the product's existing verified SQLite connection.
//! Legacy event bodies stay in orchestration_events. V37 updates and both
//! histories share one cursor on the product's verified SQLite connection.
//! These storage functions do not authenticate a caller: native H must bind
//! the session and reader identity before invoking them.

use super::atomic::{
    exec, require_canonical_json, AtomicError, Json, JsonString, Parser, Statement,
};
use super::same_open::VerifiedDatabaseConnection;
use crate::process::OriginBoundFrame;
use std::collections::BTreeMap;

const RAW_SOURCE_MAX_BYTES: usize = 1024 * 1024;
const RAW_SOURCE_OLD_COLUMNS: [&str; 11] = [
    "operation_id",
    "process_ticket",
    "custodian_nonce",
    "domain_id",
    "session_id",
    "generation",
    "source_epoch",
    "source_cursor",
    "raw_bytes",
    "state",
    "resolved_event_id",
];
const RAW_SOURCE_COLUMNS: [&str; 12] = [
    "operation_id",
    "process_ticket",
    "custodian_nonce",
    "domain_id",
    "session_id",
    "generation",
    "source_epoch",
    "source_cursor",
    "raw_bytes",
    "state",
    "resolved_event_id",
    "no_event_reason",
];
const RAW_SOURCE_SCHEMA: &str = "CREATE TABLE main.v37_ledger_raw_source (
    operation_id TEXT NOT NULL,
    process_ticket TEXT NOT NULL,
    custodian_nonce TEXT NOT NULL,
    domain_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    generation TEXT NOT NULL,
    source_epoch TEXT NOT NULL,
    source_cursor TEXT NOT NULL CHECK (
        length(source_cursor) BETWEEN 1 AND 20
        AND source_cursor NOT GLOB '*[^0-9]*'
        AND source_cursor <> '0'
        AND substr(source_cursor, 1, 1) <> '0'
    ),
    raw_bytes BLOB NOT NULL CHECK (
        length(raw_bytes) BETWEEN 1 AND 1048576
        AND substr(raw_bytes, -1, 1) = X'0A'
    ),
    state TEXT NOT NULL CHECK (state IN ('PENDING', 'RESOLVED', 'NO_EVENT')),
    resolved_event_id TEXT,
    no_event_reason TEXT,
    PRIMARY KEY (operation_id, source_epoch, source_cursor),
    UNIQUE (process_ticket, source_epoch, source_cursor),
    CHECK (
        (state = 'PENDING' AND resolved_event_id IS NULL AND no_event_reason IS NULL)
        OR
        (state = 'RESOLVED' AND resolved_event_id IS NOT NULL AND no_event_reason IS NULL)
        OR
        (state = 'NO_EVENT' AND resolved_event_id IS NULL
         AND no_event_reason IS NOT NULL
         AND length(no_event_reason) BETWEEN 1 AND 128)
    )
) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerPosition {
    pub(crate) epoch: String,
    pub(crate) cursor: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionPurpose {
    Work,
    Handoff,
    SideChat,
    FormalReview,
    Secretary,
}

impl SessionPurpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Work => "WORK",
            Self::Handoff => "HANDOFF",
            Self::SideChat => "SIDE_CHAT",
            Self::FormalReview => "FORMAL_REVIEW",
            Self::Secretary => "SECRETARY",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Tier {
    Project,
    Seat,
    Session,
    Side,
    Global,
}

impl Tier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Project => "PROJECT",
            Self::Seat => "SEAT",
            Self::Session => "SESSION",
            Self::Side => "SIDE",
            Self::Global => "GLOBAL",
        }
    }

    fn parse(value: &str) -> Result<Self, AtomicError> {
        match value {
            "PROJECT" => Ok(Self::Project),
            "SEAT" => Ok(Self::Seat),
            "SESSION" => Ok(Self::Session),
            "SIDE" => Ok(Self::Side),
            "GLOBAL" => Ok(Self::Global),
            _ => Err(AtomicError::DurabilityContractFailed(format!(
                "unknown ledger tier: {value}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionRegistration {
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) session_id: String,
    pub(crate) purpose: SessionPurpose,
    pub(crate) side_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventInput {
    pub(crate) event_id: String,
    pub(crate) source_epoch: String,
    /// The normalized K-LEDGER event cursor. It is contiguous within the
    /// session/epoch event stream and is independent of raw frame ordinals.
    pub(crate) source_cursor: String,
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) session_id: String,
    pub(crate) tier: Tier,
    pub(crate) side_id: Option<String>,
    pub(crate) occurred_at: String,
    /// Canonical ACP session/update object, produced by the adapter normalizer.
    pub(crate) update_json: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LedgerEvent {
    pub(crate) cursor: u64,
    pub(crate) input: EventInput,
}

/// A reader is always an already-registered native session. Formal reviews
/// have no ledger read path. Secretary access is deliberately a separate API.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Reader {
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EventPage {
    pub(crate) position: LedgerPosition,
    pub(crate) events: Vec<LedgerEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Subscription {
    pub(crate) id: String,
    pub(crate) reader_session_id: String,
    pub(crate) epoch: String,
    pub(crate) cursor: u64,
    pub(crate) revision: u64,
    pub(crate) active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SubscriptionPage {
    pub(crate) subscription: Subscription,
    pub(crate) events: Vec<LedgerEvent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawSourceKey {
    pub(crate) operation_id: String,
    pub(crate) source_epoch: String,
    /// The ordinal of this raw protocol frame under the native H reader.
    /// It is not the normalized K-LEDGER event cursor.
    pub(crate) source_cursor: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RawSourceState {
    Pending,
    Resolved,
    NoEvent,
}

impl RawSourceState {
    fn parse(value: &str) -> Result<Self, AtomicError> {
        match value {
            "PENDING" => Ok(Self::Pending),
            "RESOLVED" => Ok(Self::Resolved),
            "NO_EVENT" => Ok(Self::NoEvent),
            _ => Err(AtomicError::DurabilityContractFailed(format!(
                "unknown raw source state: {value}"
            ))),
        }
    }
}

/// An internal source frame. Raw bytes are returned only to the native
/// normalizer/recovery path; ordinary ledger pages and receipts never carry
/// this object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawSourceRecord {
    pub(crate) key: RawSourceKey,
    pub(crate) process_ticket: String,
    pub(crate) custodian_nonce: String,
    pub(crate) domain_id: String,
    pub(crate) session_id: String,
    pub(crate) generation: String,
    pub(crate) raw_bytes: Vec<u8>,
    pub(crate) state: RawSourceState,
    pub(crate) resolved_event_id: Option<String>,
    pub(crate) no_event_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawSourceResolution {
    pub(crate) key: RawSourceKey,
    pub(crate) event_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RawSourceNoEventResolution {
    pub(crate) key: RawSourceKey,
    pub(crate) reason: String,
}

fn required(value: &str, field: &'static str) -> Result<(), AtomicError> {
    if value.is_empty() || value.len() > 4096 || value.contains('\0') {
        Err(AtomicError::InvalidRecord(field))
    } else {
        Ok(())
    }
}

fn cursor(value: &str) -> Result<u64, AtomicError> {
    value.parse::<u64>().map_err(|error| {
        AtomicError::DurabilityContractFailed(format!("invalid ledger cursor {value}: {error}"))
    })
}

fn raw_cursor(value: &str) -> Result<u64, AtomicError> {
    let parsed = value
        .parse::<u64>()
        .ok()
        .filter(|number| *number > 0 && number.to_string() == value)
        .ok_or(AtomicError::InvalidRecord("sourceCursor"))?;
    if parsed > i64::MAX as u64 {
        return Err(AtomicError::InvalidRecord("sourceCursor"));
    }
    Ok(parsed)
}

fn raw_bytes(value: &[u8]) -> Result<(), AtomicError> {
    if value.is_empty() || value.len() > RAW_SOURCE_MAX_BYTES || value.last() != Some(&b'\n') {
        return Err(AtomicError::InvalidRecord("rawSourceFrame"));
    }
    Ok(())
}

fn raw_no_event_reason(value: &str) -> Result<(), AtomicError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(AtomicError::InvalidRecord("noEventReason"));
    }
    Ok(())
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn blob_hex(statement: &Statement, column: i32) -> Result<Vec<u8>, AtomicError> {
    let encoded = statement.column_text(column)?;
    if encoded.is_empty() || encoded.len() % 2 != 0 {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 raw source blob encoding is invalid".into(),
        ));
    }
    let mut decoded = Vec::with_capacity(encoded.len() / 2);
    for pair in encoded.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0]).ok_or_else(|| {
            AtomicError::DurabilityContractFailed(
                "A.1 raw source blob encoding is invalid".into(),
            )
        })?;
        let low = hex_nibble(pair[1]).ok_or_else(|| {
            AtomicError::DurabilityContractFailed(
                "A.1 raw source blob encoding is invalid".into(),
            )
        })?;
        decoded.push((high << 4) | low);
    }
    raw_bytes(&decoded)?;
    Ok(decoded)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HSourceBinding {
    domain_id: String,
    session_id: String,
    generation: String,
    process_ticket: String,
    custodian_nonce: String,
}

fn h_source_binding(
    connection: &VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    expected: Option<&RawSourceRecord>,
    frame: Option<&OriginBoundFrame>,
    recovery: bool,
) -> Result<HSourceBinding, AtomicError> {
    required(operation_id, "operationId")?;
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT a.domain_id, a.session_id, a.generation,
                c.ticket, c.custodian_nonce, a.phase, c.state,
                COALESCE(a.stop_fact_id, ''), COALESCE(c.stop_proof_hash, '')
         FROM main.gogoke_v37_h_process_episode AS a
         JOIN main.gogoke_coordination_process_custody AS c
           ON c.operation_id = a.process_operation_id
          AND c.domain_id = a.domain_id
          AND c.generation = a.generation
         WHERE a.process_operation_id = ?",
    )?;
    statement.bind_text(1, operation_id)?;
    if !statement.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    let binding = HSourceBinding {
        domain_id: statement.column_text(0)?,
        session_id: statement.column_text(1)?,
        generation: statement.column_text(2)?,
        process_ticket: statement.column_text(3)?,
        custodian_nonce: statement.column_text(4)?,
    };
    let claim_state = statement.column_text(5)?;
    let custody_state = statement.column_text(6)?;
    let claim_stop_fact = statement.column_text(7)?;
    let custody_stop_proof = statement.column_text(8)?;
    let stopped_recovery = expected.is_some()
        && claim_state == "STOPPED" && custody_state == "STOPPED"
        && !claim_stop_fact.is_empty() && claim_stop_fact == custody_stop_proof;
    // The custodian may have committed its exact stop proof while H's stop
    // receipt transaction rolled back. Previously captured bytes still need
    // their original A resolution before that same stop request can finish.
    let stopped_before_h_receipt = expected.is_some()
        && matches!(claim_state.as_str(),"PREPARED"|"ACTIVE"|"UNKNOWN")
        && custody_state == "STOPPED" && !custody_stop_proof.is_empty()
        && claim_stop_fact.is_empty();
    let state_ok = if recovery {
        matches!(
            (claim_state.as_str(), custody_state.as_str()),
            ("PREPARED" | "ACTIVE", "ACTIVE" | "UNKNOWN")
                | ("UNKNOWN", "UNKNOWN" | "STOPPED")
        ) || stopped_recovery || stopped_before_h_receipt
    } else {
        // A held native stdout object still proves its original source when
        // an uncertain input/commit fenced current custody. This permits
        // saving facts, never another input or a launch after restart.
        (matches!(claim_state.as_str(),"PREPARED"|"ACTIVE")
            && matches!(custody_state.as_str(),"ACTIVE"|"UNKNOWN"))
            || (claim_state=="UNKNOWN" && custody_state=="UNKNOWN"
                && frame.is_some())
    };
    if !state_ok || statement.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    if let Some(record) = expected {
        if record.key.operation_id != operation_id
            || record.domain_id != binding.domain_id
            || record.session_id != binding.session_id
            || record.generation != binding.generation
            || record.process_ticket != binding.process_ticket
            || record.custodian_nonce != binding.custodian_nonce
        {
            return Err(AtomicError::OperationConflict);
        }
    }
    if let Some(frame) = frame {
        let custody = frame.custody();
        if custody.ticket.opaque() != binding.process_ticket
            || custody.custodian_nonce != binding.custodian_nonce
            || custody.binding.domain_id != binding.domain_id
            || custody.binding.generation != binding.generation
        {
            return Err(AtomicError::OperationConflict);
        }
        if custody_state=="UNKNOWN" {
            let identity=Statement::prepare(connection.as_ptr(),
                "SELECT pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id FROM main.gogoke_coordination_process_custody WHERE operation_id=?1 AND ticket=?2 AND custodian_nonce=?3")?;
            identity.bind_text(1,operation_id)?;identity.bind_text(2,custody.ticket.opaque())?;
            identity.bind_text(3,&custody.custodian_nonce)?;
            if !identity.step_row()? || identity.column_text(0)?!=custody.identity.pid.to_string()
                || identity.column_text(1)?!=custody.identity.creation_time_100ns.to_string()
                || identity.column_text(2)?!=custody.identity.image_path.to_string_lossy()
                || identity.column_text(3)?!=custody.binding.binary_digest_sha256
                || identity.column_text(4)?!=custody.binding.profile_id || identity.step_row()? {
                return Err(AtomicError::OperationConflict);
            }
        }
    }
    Ok(binding)
}

fn read_raw_source(
    connection: &VerifiedDatabaseConnection<'_>,
    key: &RawSourceKey,
) -> Result<Option<RawSourceRecord>, AtomicError> {
    raw_cursor(&key.source_cursor)?;
    for (value, field) in [
        (&key.operation_id, "operationId"),
        (&key.source_epoch, "sourceEpoch"),
    ] {
        required(value, field)?;
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT operation_id, process_ticket, custodian_nonce, domain_id,
                session_id, generation, source_epoch, source_cursor,
                hex(raw_bytes), state, COALESCE(resolved_event_id, ''),
                COALESCE(no_event_reason, '')
         FROM main.v37_ledger_raw_source
         WHERE operation_id = ? AND source_epoch = ? AND source_cursor = ?",
    )?;
    statement.bind_text(1, &key.operation_id)?;
    statement.bind_text(2, &key.source_epoch)?;
    statement.bind_text(3, &key.source_cursor)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    let state = RawSourceState::parse(&statement.column_text(9)?)?;
    let resolved = statement.column_text(10)?;
    let no_event_reason = statement.column_text(11)?;
    let resolved_event_id = if resolved.is_empty() {
        None
    } else {
        Some(resolved)
    };
    let no_event_reason = if no_event_reason.is_empty() {
        None
    } else {
        raw_no_event_reason(&no_event_reason)?;
        Some(no_event_reason)
    };
    let record = RawSourceRecord {
        key: RawSourceKey {
            operation_id: statement.column_text(0)?,
            source_epoch: statement.column_text(6)?,
            source_cursor: statement.column_text(7)?,
        },
        process_ticket: statement.column_text(1)?,
        custodian_nonce: statement.column_text(2)?,
        domain_id: statement.column_text(3)?,
        session_id: statement.column_text(4)?,
        generation: statement.column_text(5)?,
        raw_bytes: blob_hex(&statement, 8)?,
        state,
        resolved_event_id,
        no_event_reason,
    };
    let state_shape_ok = matches!(
        (
            &record.state,
            &record.resolved_event_id,
            &record.no_event_reason,
        ),
        (RawSourceState::Pending, None, None)
            | (RawSourceState::Resolved, Some(_), None)
            | (RawSourceState::NoEvent, None, Some(_))
    );
    if !state_shape_ok || statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 raw source row is inconsistent".into(),
        ));
    }
    Ok(Some(record))
}

fn raw_key(operation_id: &str, source_epoch: &str, source_cursor: &str) -> Result<RawSourceKey, AtomicError> {
    let key = RawSourceKey {
        operation_id: operation_id.to_owned(),
        source_epoch: source_epoch.to_owned(),
        source_cursor: source_cursor.to_owned(),
    };
    read_raw_key(&key)?;
    Ok(key)
}

fn read_raw_key(key: &RawSourceKey) -> Result<(), AtomicError> {
    for (value, field) in [
        (&key.operation_id, "operationId"),
        (&key.source_epoch, "sourceEpoch"),
    ] {
        required(value, field)?;
    }
    raw_cursor(&key.source_cursor).map(|_| ())
}

fn raw_source_columns(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<Vec<String>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "PRAGMA main.table_info('v37_ledger_raw_source')",
    )?;
    let mut columns = Vec::new();
    while statement.step_row()? {
        columns.push(statement.column_text(1)?);
    }
    Ok(columns)
}

/// The raw journal was introduced before its terminal no-event disposition.
/// Preserve already-captured rows when an existing 8a database is opened by
/// the extended schema; an unknown shape fails closed instead of being guessed.
fn ensure_raw_source_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), AtomicError> {
    let columns = raw_source_columns(connection)?;
    if columns.is_empty()
        || columns
            .iter()
            .map(|value| value.as_str())
            .eq(RAW_SOURCE_COLUMNS.iter().copied())
    {
        return Ok(());
    }
    if !columns
        .iter()
        .map(|value| value.as_str())
        .eq(RAW_SOURCE_OLD_COLUMNS.iter().copied())
    {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 raw source schema shape is unknown".into(),
        ));
    }
    connection
        .execute("BEGIN IMMEDIATE")
        .map_err(AtomicError::from)?;
    let migrated: Result<(), crate::store::same_open::SameOpenError> = (|| {
        connection.execute(
            "ALTER TABLE main.v37_ledger_raw_source RENAME TO v37_ledger_raw_source_legacy",
        )?;
        connection.execute(RAW_SOURCE_SCHEMA)?;
        connection.execute(
            "INSERT INTO main.v37_ledger_raw_source
             (operation_id, process_ticket, custodian_nonce, domain_id, session_id,
              generation, source_epoch, source_cursor, raw_bytes, state,
              resolved_event_id, no_event_reason)
             SELECT operation_id, process_ticket, custodian_nonce, domain_id, session_id,
                    generation, source_epoch, source_cursor, raw_bytes, state,
                    resolved_event_id, NULL
             FROM main.v37_ledger_raw_source_legacy",
        )?;
        connection.execute("DROP TABLE main.v37_ledger_raw_source_legacy")?;
        Ok(())
    })();
    match migrated {
        Ok(()) => connection
            .execute("COMMIT")
            .map_err(AtomicError::from),
        Err(error) => {
            let _ = connection.execute("ROLLBACK");
            Err(AtomicError::from(error))
        }
    }
}

// Compare with the DDL actually executed by schema.sql, rather than a second
// hand-written approximation of the installed table. The only older shape is
// the same definition before SECRETARY was added to the purpose CHECK.
fn session_schema_definitions() -> Result<(String, String), AtomicError> {
    let marker = "CREATE TABLE IF NOT EXISTS v37_ledger_session (";
    let fragment = "'FORMAL_REVIEW', 'SECRETARY'";
    let source = include_str!("schema.sql");
    let (_, tail) = source.split_once(marker).ok_or_else(|| {
        AtomicError::DurabilityContractFailed("A.1 session DDL missing".into())
    })?;
    let (body, _) = tail.split_once(") STRICT;").ok_or_else(|| {
        AtomicError::DurabilityContractFailed("A.1 session DDL terminator missing".into())
    })?;
    // SQLite stores this CREATE TABLE without IF NOT EXISTS in sqlite_schema.
    // Keep the remaining bytes exact; schema.sql still executes the original
    // IF NOT EXISTS form for fresh databases.
    let current = format!("CREATE TABLE v37_ledger_session ({body}) STRICT");
    if current.matches(fragment).count() != 1 {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 session purpose DDL is unknown".into(),
        ));
    }
    let old = current.replacen(fragment, "'FORMAL_REVIEW'", 1);
    Ok((old, current))
}

fn session_schema_state(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<Option<String>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT type, sql FROM main.sqlite_schema WHERE name = 'v37_ledger_session' COLLATE NOCASE",
    )?;
    if !statement.step_row()? {
        return Ok(None);
    }
    if statement.column_text(0)? != "table" {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 session schema is not a table".into(),
        ));
    }
    let sql = statement.column_text(1)?;
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 session schema is ambiguous".into(),
        ));
    }
    Ok(Some(sql))
}

fn check_session_schema_dependencies(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<(), AtomicError> {
    // A table rename would rewrite dependent SQL. Accept only its implicit
    // primary-key autoindex; refuse unknown indexes, triggers, views and FKs.
    for query in [
        "SELECT name FROM main.sqlite_schema
         WHERE (tbl_name = 'v37_ledger_session' AND type IN ('index', 'trigger')
                AND (type <> 'index' OR name <> 'sqlite_autoindex_v37_ledger_session_1'
                     OR sql IS NOT NULL))
            OR (type IN ('view', 'trigger')
                AND instr(lower(sql), 'v37_ledger_session') > 0) LIMIT 1",
        "SELECT name FROM temp.sqlite_schema
         WHERE (tbl_name = 'v37_ledger_session' AND type IN ('index', 'trigger'))
            OR (type IN ('view', 'trigger')
                AND instr(lower(sql), 'v37_ledger_session') > 0) LIMIT 1",
        "SELECT s.name FROM main.sqlite_schema AS s
         JOIN pragma_foreign_key_list(s.name, 'main') AS f
         WHERE s.type = 'table' AND lower(f.\"table\") = 'v37_ledger_session'
         LIMIT 1",
        "SELECT s.name FROM temp.sqlite_schema AS s
         JOIN pragma_foreign_key_list(s.name, 'temp') AS f
         WHERE s.type = 'table' AND lower(f.\"table\") = 'v37_ledger_session'
         LIMIT 1",
    ] {
        if Statement::prepare(connection.as_ptr(), query)?.step_row()? {
            return Err(AtomicError::DurabilityContractFailed(
                "A.1 session schema has an unknown dependency".into(),
            ));
        }
    }
    if scalar(connection, "SELECT COUNT(*) FROM main.sqlite_schema
        WHERE type = 'index' AND tbl_name = 'v37_ledger_session'")? != "1" {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 session primary-key index shape is unknown".into(),
        ));
    }
    Ok(())
}

/// Rebuild only the exact four-purpose table, preserving every registered
/// identity and purpose. Unknown schema or dependencies cannot be guessed.
fn ensure_session_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), AtomicError> {
    let (old, current) = session_schema_definitions()?;
    let Some(actual) = session_schema_state(connection)? else {
        return Ok(());
    };
    if actual != old && actual != current {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 session schema shape is unknown".into(),
        ));
    }
    check_session_schema_dependencies(connection)?;
    if actual == current {
        return Ok(());
    }
    connection.execute("BEGIN IMMEDIATE")?;
    let migrated: Result<(), AtomicError> = (|| {
        if session_schema_state(connection)?.as_deref() != Some(old.as_str()) {
            return Err(AtomicError::DurabilityContractFailed(
                "A.1 session schema changed before migration".into(),
            ));
        }
        check_session_schema_dependencies(connection)?;
        if Statement::prepare(connection.as_ptr(),
            "SELECT name FROM main.sqlite_schema
             WHERE lower(name) = 'v37_ledger_session_legacy' LIMIT 1")?.step_row()? {
            return Err(AtomicError::DurabilityContractFailed(
                "A.1 session migration name is occupied".into(),
            ));
        }
        connection.execute("ALTER TABLE main.v37_ledger_session RENAME TO v37_ledger_session_legacy")?;
        connection.execute(&current)?;
        connection.execute("INSERT INTO main.v37_ledger_session
            (session_id, domain_id, seat_id, purpose, side_id)
            SELECT session_id, domain_id, seat_id, purpose, side_id
            FROM main.v37_ledger_session_legacy")?;
        if scalar(connection, "SELECT COUNT(*) FROM main.v37_ledger_session")?
            != scalar(connection, "SELECT COUNT(*) FROM main.v37_ledger_session_legacy")? {
            return Err(AtomicError::DurabilityContractFailed(
                "A.1 session migration row count changed".into(),
            ));
        }
        connection.execute("DROP TABLE main.v37_ledger_session_legacy")?;
        if session_schema_state(connection)?.as_deref() != Some(current.as_str()) {
            return Err(AtomicError::DurabilityContractFailed(
                "A.1 session schema verification failed".into(),
            ));
        }
        Ok(())
    })();
    match migrated {
        Ok(()) => match connection.execute("COMMIT") {
            Ok(()) => Ok(()),
            Err(commit_error) => {
                let rollback = connection.execute("ROLLBACK");
                Err(AtomicError::DurabilityContractFailed(format!(
                    "A.1 session migration commit failed: {commit_error:?}; rollback: {rollback:?}"
                )))
            }
        },
        Err(error) => {
            if let Err(rollback_error) = connection.execute("ROLLBACK") {
                return Err(AtomicError::DurabilityContractFailed(format!(
                    "A.1 session migration failed: {error}; rollback failed: {rollback_error:?}"
                )));
            }
            Err(error)
        }
    }
}

fn registered(
    connection: &VerifiedDatabaseConnection<'_>,
    session_id: &str,
) -> Result<Option<SessionRegistration>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT domain_id, seat_id, session_id, purpose, COALESCE(side_id, '')
         FROM v37_ledger_session WHERE session_id = ?",
    )?;
    statement.bind_text(1, session_id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    let purpose = match statement.column_text(3)?.as_str() {
        "WORK" => SessionPurpose::Work,
        "HANDOFF" => SessionPurpose::Handoff,
        "SIDE_CHAT" => SessionPurpose::SideChat,
        "FORMAL_REVIEW" => SessionPurpose::FormalReview,
        "SECRETARY" => SessionPurpose::Secretary,
        value => {
            return Err(AtomicError::DurabilityContractFailed(format!(
                "unknown session purpose: {value}"
            )))
        }
    };
    let side = statement.column_text(4)?;
    let result = SessionRegistration {
        domain_id: statement.column_text(0)?,
        seat_id: statement.column_text(1)?,
        session_id: statement.column_text(2)?,
        purpose,
        side_id: if side.is_empty() { None } else { Some(side) },
    };
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "duplicate session registration".into(),
        ));
    }
    Ok(Some(result))
}

/// Native composition reads the existing registration; this lookup grants no authority.
pub(crate) fn read_registered_session(connection: &VerifiedDatabaseConnection<'_>,
    session_id: &str) -> Result<Option<SessionRegistration>, AtomicError> {
    registered(connection, session_id)
}

/// Called by native H only after a fresh session has been admitted. Replaying
/// an identical registration is safe; changing purpose or binding is refused.
pub(crate) fn register_session(
    connection: &mut VerifiedDatabaseConnection<'_>,
    registration: &SessionRegistration,
) -> Result<(), AtomicError> {
    for (value, field) in [
        (&registration.domain_id, "domainId"),
        (&registration.seat_id, "seatId"),
        (&registration.session_id, "sessionId"),
    ] {
        required(value, field)?;
    }
    if (registration.purpose == SessionPurpose::SideChat) != registration.side_id.is_some() {
        return Err(AtomicError::InvalidRecord("sideId"));
    }
    if let Some(side) = &registration.side_id {
        required(side, "sideId")?;
    }
    if let Some(existing) = registered(connection, &registration.session_id)? {
        return if existing == *registration {
            Ok(())
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_session (domain_id, seat_id, session_id, purpose, side_id)
         VALUES (?, ?, ?, ?, ?)",
    )?;
    statement.bind_text(1, &registration.domain_id)?;
    statement.bind_text(2, &registration.seat_id)?;
    statement.bind_text(3, &registration.session_id)?;
    statement.bind_text(4, registration.purpose.as_str())?;
    if let Some(side) = &registration.side_id {
        statement.bind_text(5, side)?;
    }
    statement.step_done()
}

/// L0 supplies the verified old thread identity. An unbound old row stays
/// unreadable; storage never guesses a seat from its event payload.
pub(crate) fn bind_legacy_thread(
    connection: &mut VerifiedDatabaseConnection<'_>,
    thread_id: &str,
    registration: &SessionRegistration,
) -> Result<(), AtomicError> {
    required(thread_id, "threadId")?;
    if registration.purpose != SessionPurpose::Work
        || registered(connection, &registration.session_id)?.as_ref() != Some(registration)
    {
        return Err(AtomicError::InvalidRecord("legacy session binding"));
    }
    let existing = Statement::prepare(
        connection.as_ptr(),
        "SELECT domain_id, seat_id, session_id FROM v37_ledger_legacy_binding WHERE thread_id = ?",
    )?;
    existing.bind_text(1, thread_id)?;
    if existing.step_row()? {
        return if existing.column_text(0)? == registration.domain_id
            && existing.column_text(1)? == registration.seat_id
            && existing.column_text(2)? == registration.session_id
        {
            Ok(())
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_legacy_binding (thread_id, domain_id, seat_id, session_id)
         VALUES (?, ?, ?, ?)",
    )?;
    statement.bind_text(1, thread_id)?;
    statement.bind_text(2, &registration.domain_id)?;
    statement.bind_text(3, &registration.seat_id)?;
    statement.bind_text(4, &registration.session_id)?;
    statement.step_done()
}

/// D calls this in its side-chat deletion transaction. Referenced lead
/// events live outside this tier and are never touched.
pub(crate) fn delete_side_events(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    side_id: &str,
) -> Result<(), AtomicError> {
    required(domain_id, "domainId")?;
    required(side_id, "sideId")?;
    // Preserve the source stream's terminal cursor before removing its
    // projected rows.  A deleted side stream is a durable tombstone: it may
    // not be resumed with a cursor that would hide the deletion.
    let mark = Statement::prepare(
        connection.as_ptr(),
        "UPDATE v37_ledger_source_stream
         SET state = 'TOMBSTONED'
         WHERE EXISTS (
             SELECT 1 FROM v37_ledger_index AS i
             WHERE i.source_kind = 'v37' AND i.domain_id = ?
               AND i.tier = 'SIDE' AND i.side_id = ?
               AND i.session_id = v37_ledger_source_stream.session_id
               AND i.source_epoch = v37_ledger_source_stream.source_epoch
         )",
    )?;
    mark.bind_text(1, domain_id)?;
    mark.bind_text(2, side_id)?;
    mark.step_done()?;
    let statement = Statement::prepare(
        connection.as_ptr(),
        "DELETE FROM v37_ledger_index WHERE source_kind = 'v37'
         AND domain_id = ? AND tier = 'SIDE' AND side_id = ?",
    )?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, side_id)?;
    statement.step_done()
}

fn read_event(statement: &Statement) -> Result<LedgerEvent, AtomicError> {
    let side = statement.column_text(8)?;
    let update_json = if statement.column_text(11)? == "legacy" {
        let mut meta = BTreeMap::new();
        meta.insert(
            JsonString::from("legacyEventType"),
            Json::String(statement.column_text(12)?.into()),
        );
        meta.insert(
            JsonString::from("legacyEventId"),
            Json::String(statement.column_text(1)?.into()),
        );
        meta.insert(
            JsonString::from("legacyPayload"),
            Parser::parse(&statement.column_text(13)?)?,
        );
        let mut update = BTreeMap::new();
        update.insert(
            JsonString::from("sessionUpdate"),
            Json::String("session_info_update".into()),
        );
        update.insert(JsonString::from("_meta"), Json::Object(meta));
        Json::Object(update).canonical()
    } else {
        statement.column_text(10)?
    };
    Ok(LedgerEvent {
        cursor: cursor(&statement.column_text(0)?)?,
        input: EventInput {
            event_id: statement.column_text(1)?,
            source_cursor: statement.column_text(2)?,
            source_epoch: statement.column_text(3)?,
            domain_id: statement.column_text(4)?,
            seat_id: statement.column_text(5)?,
            session_id: statement.column_text(6)?,
            tier: Tier::parse(&statement.column_text(7)?)?,
            side_id: if side.is_empty() { None } else { Some(side) },
            occurred_at: statement.column_text(9)?,
            update_json,
        },
    })
}

const EVENT_COLUMNS: &str = "i.cursor, i.source_event_id,
    COALESCE(i.source_cursor, CAST(e.sequence AS TEXT)),
    COALESCE(i.source_epoch, (SELECT epoch FROM v37_ledger_meta WHERE singleton = 1)),
    COALESCE(i.domain_id, b.domain_id), COALESCE(i.seat_id, b.seat_id),
    COALESCE(i.session_id, b.session_id), COALESCE(i.tier, 'SEAT'),
    COALESCE(i.side_id, ''), COALESCE(i.occurred_at, e.occurred_at),
    COALESCE(i.update_json, ''), i.source_kind,
    COALESCE(e.event_type, ''), COALESCE(e.payload_json, '')";
const EVENT_SOURCE: &str = "FROM v37_ledger_index AS i
    LEFT JOIN orchestration_events AS e
      ON i.source_kind = 'legacy' AND e.event_id = i.source_event_id
    LEFT JOIN v37_ledger_legacy_binding AS b
      ON i.source_kind = 'legacy' AND b.thread_id = e.stream_id";

fn existing_event(
    connection: &VerifiedDatabaseConnection<'_>,
    event_id: &str,
) -> Result<Option<LedgerEvent>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
                  WHERE i.source_event_id = ? AND i.source_kind = 'v37'"
        ),
    )?;
    statement.bind_text(1, event_id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    Ok(Some(read_event(&statement)?))
}

/// Capture one exact provider stdout frame before normalization. The frame's
/// custody is the only source of process labels: operation/session/generation
/// are resolved through H on this same connection, while the ticket, nonce,
/// domain and generation must match the custody object that read the bytes.
/// No caller-selected label can create a journal row. `source_cursor` is the
/// raw frame ordinal; normalized event cursors are supplied later by `record`.
pub(crate) fn capture_raw_source(
    connection: &mut VerifiedDatabaseConnection<'_>,
    frame: &OriginBoundFrame,
    operation_id: &str,
    source_epoch: &str,
    source_cursor: &str,
) -> Result<RawSourceRecord, AtomicError> {
    let key = raw_key(operation_id, source_epoch, source_cursor)?;
    raw_bytes(frame.bytes())?;
    let binding = h_source_binding(connection, operation_id, None, Some(frame), false)?;
    if let Some(existing) = read_raw_source(connection, &key)? {
        if existing.process_ticket == binding.process_ticket
            && existing.custodian_nonce == binding.custodian_nonce
            && existing.domain_id == binding.domain_id
            && existing.session_id == binding.session_id
            && existing.generation == binding.generation
            && existing.raw_bytes == frame.bytes()
        {
            return Ok(existing);
        }
        return Err(AtomicError::OperationConflict);
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO main.v37_ledger_raw_source
         (operation_id, process_ticket, custodian_nonce, domain_id, session_id,
          generation, source_epoch, source_cursor, raw_bytes, state, no_event_reason)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'PENDING', NULL)",
    )?;
    statement.bind_text(1, operation_id)?;
    statement.bind_text(2, &binding.process_ticket)?;
    statement.bind_text(3, &binding.custodian_nonce)?;
    statement.bind_text(4, &binding.domain_id)?;
    statement.bind_text(5, &binding.session_id)?;
    statement.bind_text(6, &binding.generation)?;
    statement.bind_text(7, source_epoch)?;
    statement.bind_text(8, source_cursor)?;
    statement.bind_blob(9, frame.bytes())?;
    statement.step_done()?;
    read_raw_source(connection, &key)?.ok_or_else(|| {
        AtomicError::DurabilityContractFailed("A.1 raw source missing after capture".into())
    })
}

/// Recover a pending frame for the native normalizer. H may be COMMITTED with
/// UNKNOWN custody after host restart; that pair is accepted only when the
/// durable operation/ticket/session/generation binding still matches. A
/// RELEASED claim is accepted only for an already-captured row whose custody
/// is STOPPED and whose stored binding still matches.
pub(crate) fn read_pending_raw_source(
    connection: &VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    source_epoch: &str,
    source_cursor: &str,
) -> Result<Option<RawSourceRecord>, AtomicError> {
    let record=read_captured_raw_source(connection,operation_id,source_epoch,source_cursor)?;
    Ok(record.filter(|record|record.state==RawSourceState::Pending))
}

/// Native C readback needs the original captured card after A terminalizes
/// its source. The same H recovery binding remains mandatory for every
/// state; terminalization never creates another raw history or a new grant.
pub(crate) fn read_captured_raw_source(
    connection: &VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    source_epoch: &str,
    source_cursor: &str,
) -> Result<Option<RawSourceRecord>, AtomicError> {
    let key = raw_key(operation_id, source_epoch, source_cursor)?;
    let Some(record) = read_raw_source(connection, &key)? else {
        return Ok(None);
    };
    h_source_binding(connection, operation_id, Some(&record), None, true)?;
    Ok(Some(record))
}

/// Link a pending raw frame to the already-normalized K-LEDGER event. This
/// updates only the journal row; normalized query pages and their global
/// cursor stay unchanged. Repeating the exact resolution is idempotent, while
/// a different event identity is a conflict. The raw frame ordinal is linked
/// by event ID and custody/session/epoch binding; it need not equal the
/// normalized event cursor.
pub(crate) fn resolve_raw_source(
    connection: &mut VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    source_epoch: &str,
    source_cursor: &str,
    event_id: &str,
) -> Result<RawSourceResolution, AtomicError> {
    let key = raw_key(operation_id, source_epoch, source_cursor)?;
    required(event_id, "eventId")?;
    let existing = read_raw_source(connection, &key)?
        .ok_or(AtomicError::InvalidRecord("rawSource"))?;
    if existing.state == RawSourceState::Resolved {
        return if existing.resolved_event_id.as_deref() == Some(event_id) {
            Ok(RawSourceResolution {
                key,
                event_id: event_id.to_owned(),
            })
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    if existing.state == RawSourceState::NoEvent {
        return Err(AtomicError::OperationConflict);
    }
    let binding = h_source_binding(connection, operation_id, Some(&existing), None, true)?;
    let event = Statement::prepare(
        connection.as_ptr(),
        "SELECT domain_id, session_id, source_epoch
         FROM v37_ledger_index
         WHERE source_kind = 'v37' AND source_event_id = ?",
    )?;
    event.bind_text(1, event_id)?;
    if !event.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    if event.column_text(0)? != binding.domain_id
        || event.column_text(1)? != binding.session_id
        || event.column_text(2)? != existing.key.source_epoch
        || event.step_row()?
    {
        return Err(AtomicError::OperationConflict);
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "UPDATE main.v37_ledger_raw_source
         SET state = 'RESOLVED', resolved_event_id = ?
         WHERE operation_id = ? AND source_epoch = ? AND source_cursor = ?
           AND state = 'PENDING' AND resolved_event_id IS NULL
           AND no_event_reason IS NULL",
    )?;
    statement.bind_text(1, event_id)?;
    statement.bind_text(2, operation_id)?;
    statement.bind_text(3, source_epoch)?;
    statement.bind_text(4, source_cursor)?;
    statement.step_done()?;
    let resolved = read_raw_source(connection, &key)?
        .ok_or_else(|| AtomicError::DurabilityContractFailed("A.1 raw source disappeared".into()))?;
    if resolved.state != RawSourceState::Resolved
        || resolved.resolved_event_id.as_deref() != Some(event_id)
    {
        return Err(AtomicError::OperationConflict);
    }
    Ok(RawSourceResolution {
        key,
        event_id: event_id.to_owned(),
    })
}

/// Mark a captured provider frame as terminal without manufacturing a
/// normalized ledger event. This covers JSON-RPC replies/notifications (and
/// other protocol frames) that are valid source bytes but carry no K-LEDGER
/// update. The raw frame ordinal is terminalized independently of the
/// normalized event stream. The reason is a bounded code, never raw protocol
/// text.
pub(crate) fn resolve_raw_source_no_event(
    connection: &mut VerifiedDatabaseConnection<'_>,
    operation_id: &str,
    source_epoch: &str,
    source_cursor: &str,
    reason: &str,
) -> Result<RawSourceNoEventResolution, AtomicError> {
    let key = raw_key(operation_id, source_epoch, source_cursor)?;
    raw_no_event_reason(reason)?;
    let existing = read_raw_source(connection, &key)?
        .ok_or(AtomicError::InvalidRecord("rawSource"))?;
    if existing.state == RawSourceState::NoEvent {
        return if existing.no_event_reason.as_deref() == Some(reason) {
            Ok(RawSourceNoEventResolution {
                key,
                reason: reason.to_owned(),
            })
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    if existing.state == RawSourceState::Resolved {
        return Err(AtomicError::OperationConflict);
    }
    h_source_binding(connection, operation_id, Some(&existing), None, true)?;
    let statement = Statement::prepare(
        connection.as_ptr(),
        "UPDATE main.v37_ledger_raw_source
         SET state = 'NO_EVENT', resolved_event_id = NULL, no_event_reason = ?
         WHERE operation_id = ? AND source_epoch = ? AND source_cursor = ?
           AND state = 'PENDING' AND resolved_event_id IS NULL
           AND no_event_reason IS NULL",
    )?;
    statement.bind_text(1, reason)?;
    statement.bind_text(2, operation_id)?;
    statement.bind_text(3, source_epoch)?;
    statement.bind_text(4, source_cursor)?;
    statement.step_done()?;
    let resolved = read_raw_source(connection, &key)?
        .ok_or_else(|| AtomicError::DurabilityContractFailed("A.1 raw source disappeared".into()))?;
    if resolved.state != RawSourceState::NoEvent
        || resolved.no_event_reason.as_deref() != Some(reason)
    {
        return Err(AtomicError::OperationConflict);
    }
    Ok(RawSourceNoEventResolution {
        key,
        reason: reason.to_owned(),
    })
}

/// Append one normalized update. The source ID is the idempotency key; a
/// replay with different bytes or labels conflicts. Caller serializes writes
/// on the same verified connection and never exposes this as raw IPC.
pub(crate) fn record(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &EventInput,
) -> Result<LedgerEvent, AtomicError> {
    for (value, field) in [
        (&input.event_id, "eventId"),
        (&input.source_epoch, "sourceEpoch"),
        (&input.source_cursor, "sourceCursor"),
        (&input.domain_id, "domainId"),
        (&input.seat_id, "seatId"),
        (&input.session_id, "sessionId"),
        (&input.occurred_at, "occurredAt"),
    ] {
        required(value, field)?;
    }
    if input
        .source_cursor
        .parse::<u64>()
        .ok()
        .filter(|number| number.to_string() == input.source_cursor)
        .is_none()
    {
        return Err(AtomicError::InvalidRecord("sourceCursor"));
    }
    require_canonical_json(input.update_json.as_bytes(), "session/update")?;
    let Json::Object(update) = Parser::parse(&input.update_json)? else {
        return Err(AtomicError::InvalidRecord("session/update object"));
    };
    let Some(Json::String(kind)) = update.get(&JsonString::from("sessionUpdate")) else {
        return Err(AtomicError::InvalidRecord("sessionUpdate"));
    };
    if !matches!(
        kind.to_well_formed_string().as_deref(),
        Some(
            "user_message_chunk"
                | "agent_message_chunk"
                | "agent_thought_chunk"
                | "tool_call"
                | "tool_call_update"
                | "plan"
                | "available_commands_update"
                | "current_mode_update"
                | "config_option_update"
                | "session_info_update"
                | "usage_update"
        )
    ) {
        return Err(AtomicError::InvalidRecord("sessionUpdate"));
    }
    let session = registered(connection, &input.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered session"))?;
    if session.domain_id != input.domain_id || session.seat_id != input.seat_id {
        return Err(AtomicError::OperationConflict);
    }
    if input.tier == Tier::Side {
        if session.purpose != SessionPurpose::SideChat || session.side_id != input.side_id {
            return Err(AtomicError::InvalidRecord("side-chat tier"));
        }
    } else if input.side_id.is_some() || session.purpose == SessionPurpose::SideChat {
        return Err(AtomicError::InvalidRecord("side-chat event"));
    }
    if session.purpose == SessionPurpose::FormalReview && input.tier == Tier::Global {
        return Err(AtomicError::InvalidRecord("formal review global tier"));
    }
    if let Some(existing) = existing_event(connection, &input.event_id)? {
        return if existing.input == *input {
            Ok(existing)
        } else {
            Err(AtomicError::OperationConflict)
        };
    }
    let source_cursor = cursor(&input.source_cursor)?;
    if source_cursor > i64::MAX as u64 {
        return Err(AtomicError::InvalidRecord("sourceCursor"));
    }
    let stream = Statement::prepare(
        connection.as_ptr(),
        "SELECT last_cursor, state FROM v37_ledger_source_stream
         WHERE session_id = ? AND source_epoch = ?",
    )?;
    stream.bind_text(1, &input.session_id)?;
    stream.bind_text(2, &input.source_epoch)?;
    let stream_exists = stream.step_row()?;
    if stream_exists {
        let last = cursor(&stream.column_text(0)?)?;
        let state = stream.column_text(1)?;
        let expected = last.saturating_add(1);
        if state != "ACTIVE" || source_cursor != expected {
            let failure = if state != "ACTIVE" { "source stream tombstoned" }
                else if source_cursor > expected { "source stream gap" }
                else { "source stream duplicate or rewind" };
            return Err(AtomicError::DurabilityContractFailed(format!(
                "A.1 {failure}: session={} epoch={} expected={} received={} state={state}",
                input.session_id,
                input.source_epoch,
                expected,
                source_cursor,
            )));
        }
    } else if source_cursor != 1 {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 source stream gap: session={} epoch={} expected=1 received={source_cursor}",
            input.session_id, input.source_epoch
        )));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_index
         (source_event_id, source_kind, source_cursor, source_epoch, domain_id, seat_id,
          session_id, tier, side_id, occurred_at, update_json)
         VALUES (?, 'v37', ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )?;
    statement.bind_text(1, &input.event_id)?;
    statement.bind_text(2, &input.source_cursor)?;
    statement.bind_text(3, &input.source_epoch)?;
    statement.bind_text(4, &input.domain_id)?;
    statement.bind_text(5, &input.seat_id)?;
    statement.bind_text(6, &input.session_id)?;
    statement.bind_text(7, input.tier.as_str())?;
    if let Some(side) = &input.side_id {
        statement.bind_text(8, side)?;
    }
    statement.bind_text(9, &input.occurred_at)?;
    statement.bind_text(10, &input.update_json)?;
    statement.step_done()?;
    existing_event(connection, &input.event_id)?
        .ok_or_else(|| AtomicError::DurabilityContractFailed("appended event missing".into()))
}

/// Read only rows permitted by the persisted reader session. Cursor gaps are
/// expected: hidden rows retain their global positions and are never emitted.
pub(crate) fn query(
    connection: &VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    after: &LedgerPosition,
    limit: u32,
) -> Result<EventPage, AtomicError> {
    let session = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if session.domain_id != reader.domain_id || session.seat_id != reader.seat_id {
        return Err(AtomicError::OperationConflict);
    }
    if session.purpose == SessionPurpose::FormalReview {
        return Err(AtomicError::InvalidRecord(
            "formal review cannot read ledger",
        ));
    }
    if session.purpose == SessionPurpose::Secretary {
        return Err(AtomicError::InvalidRecord(
            "secretary requires a dedicated ledger API",
        ));
    }
    let position = recover(connection)?;
    if after.epoch != position.epoch || after.cursor > position.cursor {
        return Err(AtomicError::OperationConflict);
    }
    if limit == 0 || limit > 1000 {
        return Err(AtomicError::InvalidRecord("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
         WHERE i.cursor > ? AND COALESCE(i.domain_id, b.domain_id) = ?
           AND (i.source_kind = 'v37' OR b.thread_id IS NOT NULL)
           AND (COALESCE(i.tier, 'SEAT') = 'PROJECT' OR
                (COALESCE(i.tier, 'SEAT') = 'SEAT' AND
                    (? = 'SIDE_CHAT' OR COALESCE(i.seat_id, b.seat_id) = ?)) OR
                (i.tier = 'SESSION' AND (? = 'SIDE_CHAT' OR i.session_id = ?)) OR
                (i.tier = 'SIDE' AND i.side_id = ?))
         ORDER BY i.cursor LIMIT ?"
        ),
    )?;
    statement.bind_i64(1, after.cursor as i64)?;
    statement.bind_text(2, &reader.domain_id)?;
    statement.bind_text(3, session.purpose.as_str())?;
    statement.bind_text(4, &reader.seat_id)?;
    statement.bind_text(5, session.purpose.as_str())?;
    statement.bind_text(6, &reader.session_id)?;
    statement.bind_text(7, session.side_id.as_deref().unwrap_or(""))?;
    statement.bind_i64(8, i64::from(limit))?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(read_event(&statement)?);
    }
    Ok(EventPage { position, events })
}

/// Read only the persisted side-chat registration's own transcript. The
/// identity predicate precedes LIMIT so unrelated ledger rows cannot consume
/// this page; the returned position remains the global ledger head.
pub(crate) fn query_own_side(
    connection: &VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    after: &LedgerPosition,
    limit: u32,
) -> Result<EventPage, AtomicError> {
    let session = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if session.domain_id != reader.domain_id || session.seat_id != reader.seat_id {
        return Err(AtomicError::OperationConflict);
    }
    if session.purpose != SessionPurpose::SideChat {
        return Err(AtomicError::InvalidRecord("side-chat reader required"));
    }
    let side_id = session.side_id.as_deref()
        .ok_or(AtomicError::InvalidRecord("sideId"))?;
    let position = recover(connection)?;
    if after.epoch != position.epoch || after.cursor > position.cursor {
        return Err(AtomicError::OperationConflict);
    }
    if limit == 0 || limit > 1000 {
        return Err(AtomicError::InvalidRecord("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
             WHERE i.cursor > ? AND i.source_kind = 'v37'
               AND i.domain_id = ? AND i.tier = 'SIDE' AND i.side_id = ?
             ORDER BY i.cursor LIMIT ?"
        ),
    )?;
    statement.bind_i64(1, after.cursor as i64)?;
    statement.bind_text(2, &session.domain_id)?;
    statement.bind_text(3, side_id)?;
    statement.bind_i64(4, i64::from(limit))?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(read_event(&statement)?);
    }
    Ok(EventPage { position, events })
}

/// Privileged secretary route. Native H must supply a verified global seat;
/// project and side readers must never be routed to this function.
pub(crate) fn query_global(
    connection: &VerifiedDatabaseConnection<'_>,
    after: &LedgerPosition,
    limit: u32,
) -> Result<EventPage, AtomicError> {
    let position = recover(connection)?;
    if after.epoch != position.epoch || after.cursor > position.cursor {
        return Err(AtomicError::OperationConflict);
    }
    if limit == 0 || limit > 1000 {
        return Err(AtomicError::InvalidRecord("limit"));
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        &format!(
            "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
             WHERE i.cursor > ? AND (i.source_kind = 'v37' OR b.thread_id IS NOT NULL)
             ORDER BY i.cursor LIMIT ?"
        ),
    )?;
    statement.bind_i64(1, after.cursor as i64)?;
    statement.bind_i64(2, i64::from(limit))?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(read_event(&statement)?);
    }
    Ok(EventPage { position, events })
}

/// Read the enduring conversation of the currently designated Secretary seat.
/// The native host must first establish the Owner/global reader through its
/// original H proof; this storage read does not authenticate a caller. The
/// reader's persisted NativeV2 incarnation is the only history identity.
/// Released historical sessions remain readable, but absent or ambiguous
/// original H associations never become conversation sources.
pub(crate) fn query_secretary_history(
    connection: &VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    after: &LedgerPosition,
    limit: u32,
) -> Result<EventPage, AtomicError> {
    let registration = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if reader.domain_id != "global" || registration.domain_id != reader.domain_id
        || registration.seat_id != reader.seat_id
        || registration.purpose != SessionPurpose::Secretary
    {
        return Err(AtomicError::OperationConflict);
    }
    // Resolve the incarnation from persisted H/E facts, never a caller label.
    let identity = Statement::prepare(connection.as_ptr(),
        "SELECT v.seat_incarnation
           FROM main.gogoke_v37_session_binding_v2 v
           JOIN main.gogoke_v37_h_claim h ON h.domain_id=v.domain_id
             AND h.session_id=v.session_id AND h.instance_id=v.selected_instance_id
             AND h.state IN ('COMMITTED','STOPPED','RELEASED')
           JOIN main.gogoke_v37_h_owner_binding owner ON owner.binding_id=h.binding_id
             AND owner.domain_id=h.domain_id AND owner.instance_id=h.instance_id
             AND owner.kind='SESSION' AND owner.owner_id=h.session_id
             AND owner.generation=h.generation
           JOIN main.gogoke_v37_instance_homes home ON home.home_id=h.home_id
             AND home.domain_id=h.domain_id AND home.instance_id=h.instance_id
             AND home.kind='SESSION' AND home.owner_id=h.session_id
             AND home.generation=h.generation
           JOIN main.gogoke_v37_seat_secretary d ON d.singleton=1
             AND d.domain_id='global' AND d.seat_id=v.seat_id
             AND d.incarnation=v.seat_incarnation
           JOIN main.gogoke_v37_seats s ON s.domain_id=d.domain_id
             AND s.seat_id=d.seat_id AND s.incarnation=d.incarnation
             AND s.layer='USER' AND s.kind='LONG' AND s.state<>'RECLAIMED'
          WHERE v.domain_id='global' AND v.session_id=?1 AND v.seat_id=?2
            AND v.provenance='NATIVE_V2'")?;
    identity.bind_text(1, &reader.session_id)?;
    identity.bind_text(2, &reader.seat_id)?;
    if !identity.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    let incarnation = identity.column_text(0)?;
    if identity.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    let position = recover(connection)?;
    if after.epoch != position.epoch || after.cursor > position.cursor {
        return Err(AtomicError::OperationConflict);
    }
    if limit == 0 || limit > 1000 {
        return Err(AtomicError::InvalidRecord("limit"));
    }
    // Scope and original-source eligibility precede LIMIT. The original raw
    // ordinal is not the normalized event cursor, and the episode generation
    // is not the current H claim generation after resume.
    let statement = Statement::prepare(connection.as_ptr(), &format!(
        "SELECT {EVENT_COLUMNS} {EVENT_SOURCE}
         JOIN main.v37_ledger_session src ON src.session_id=i.session_id
           AND src.domain_id='global' AND src.seat_id=?1
           AND src.purpose='SECRETARY'
         JOIN main.gogoke_v37_session_binding_v2 v ON v.domain_id=src.domain_id
           AND v.session_id=src.session_id AND v.seat_id=src.seat_id
           AND v.seat_incarnation=?2 AND v.provenance='NATIVE_V2'
         WHERE i.cursor>?3 AND i.source_kind='v37'
           AND i.domain_id='global' AND i.tier='GLOBAL' AND i.side_id IS NULL
           AND EXISTS (
             SELECT 1 FROM main.v37_ledger_raw_source r
             JOIN main.gogoke_v37_h_process_episode episode
               ON episode.process_operation_id=r.operation_id
               AND episode.domain_id=r.domain_id AND episode.session_id=r.session_id
               AND episode.generation=r.generation
               AND episode.instance_id=v.selected_instance_id
               AND episode.seat_id=v.seat_id
               AND episode.seat_incarnation=v.seat_incarnation
             JOIN main.gogoke_coordination_process_custody custody
               ON custody.operation_id=episode.process_operation_id
               AND custody.domain_id=r.domain_id AND custody.generation=r.generation
               AND custody.ticket=r.process_ticket
               AND custody.custodian_nonce=r.custodian_nonce
             WHERE r.state='RESOLVED' AND r.no_event_reason IS NULL
               AND r.resolved_event_id=i.source_event_id
               AND r.domain_id=i.domain_id AND r.session_id=i.session_id
               AND r.source_epoch=i.source_epoch)
         ORDER BY i.cursor LIMIT ?4"
    ))?;
    statement.bind_text(1, &reader.seat_id)?;
    statement.bind_text(2, &incarnation)?;
    statement.bind_i64(3, after.cursor as i64)?;
    statement.bind_i64(4, i64::from(limit))?;
    let mut events = Vec::new();
    while statement.step_row()? {
        events.push(read_event(&statement)?);
    }
    Ok(EventPage { position, events })
}

fn subscription(
    connection: &VerifiedDatabaseConnection<'_>,
    id: &str,
) -> Result<Option<Subscription>, AtomicError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT subscription_id, reader_id, epoch, cursor, revision, state
         FROM v37_ledger_subscription WHERE subscription_id = ?",
    )?;
    statement.bind_text(1, id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    let result = Subscription {
        id: statement.column_text(0)?,
        reader_session_id: statement.column_text(1)?,
        epoch: statement.column_text(2)?,
        cursor: cursor(&statement.column_text(3)?)?,
        revision: cursor(&statement.column_text(4)?)?,
        active: statement.column_text(5)? == "ACTIVE",
    };
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "duplicate subscription".into(),
        ));
    }
    Ok(Some(result))
}

/// A subscription starts from a checked position and is durably pinned to
/// one reader session. Polling it after restart resumes from its stored cursor.
pub(crate) fn subscribe(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    id: &str,
    after: &LedgerPosition,
    limit: u32,
) -> Result<SubscriptionPage, AtomicError> {
    subscribe_with_scope(connection, reader, id, after, limit, false)
}

pub(crate) fn subscribe_global(
    connection: &mut VerifiedDatabaseConnection<'_>, reader: &Reader, id: &str,
    after: &LedgerPosition, limit: u32,
) -> Result<SubscriptionPage, AtomicError> {
    subscribe_with_scope(connection, reader, id, after, limit, true)
}

fn subscribe_with_scope(
    connection: &mut VerifiedDatabaseConnection<'_>, reader: &Reader, id: &str,
    after: &LedgerPosition, limit: u32, global: bool,
) -> Result<SubscriptionPage, AtomicError> {
    required(id, "subscriptionId")?;
    if subscription(connection, id)?.is_some() {
        return Err(AtomicError::OperationConflict);
    }
    let registration = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if registration.domain_id != reader.domain_id || registration.seat_id != reader.seat_id
        || (global && (registration.purpose != SessionPurpose::Secretary || reader.domain_id != "global"))
        || (!global && registration.purpose == SessionPurpose::Secretary) {
        return Err(AtomicError::OperationConflict);
    }
    let page = if global { query_global(connection, after, limit)? }
        else { query(connection, reader, after, limit)? };
    let position = if page.events.len() == usize::try_from(limit).unwrap_or(usize::MAX) {
        page.events
            .last()
            .map_or(after.cursor, |event| event.cursor)
    } else {
        page.position.cursor
    };
    let kind = if global { "GLOBAL" } else if registration.purpose == SessionPurpose::SideChat {
        "SIDE"
    } else {
        "PROJECT"
    };
    let statement = Statement::prepare(
        connection.as_ptr(),
        "INSERT INTO v37_ledger_subscription
         (subscription_id, reader_kind, domain_id, reader_id, cursor, epoch, revision, state)
         VALUES (?, ?, ?, ?, ?, ?, 1, 'ACTIVE')",
    )?;
    statement.bind_text(1, id)?;
    statement.bind_text(2, kind)?;
    statement.bind_text(3, &reader.domain_id)?;
    statement.bind_text(4, &reader.session_id)?;
    statement.bind_i64(5, position as i64)?;
    statement.bind_text(6, &page.position.epoch)?;
    statement.step_done()?;
    Ok(SubscriptionPage {
        subscription: subscription(connection, id)?.ok_or_else(|| {
            AtomicError::DurabilityContractFailed("subscription missing after insert".into())
        })?,
        events: page.events,
    })
}

/// Every poll rechecks the persisted reader binding, purpose and scope.
/// A requested cursor must equal the durable cursor: silent rewind and gap
/// acknowledgement are both refused.
pub(crate) fn resume_subscription(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    id: &str,
    after: &LedgerPosition,
    limit: u32,
) -> Result<SubscriptionPage, AtomicError> {
    resume_with_scope(connection, reader, id, after, limit, false)
}

pub(crate) fn resume_global_subscription(
    connection: &mut VerifiedDatabaseConnection<'_>, reader: &Reader, id: &str,
    after: &LedgerPosition, limit: u32,
) -> Result<SubscriptionPage, AtomicError> {
    resume_with_scope(connection, reader, id, after, limit, true)
}

fn resume_with_scope(
    connection: &mut VerifiedDatabaseConnection<'_>, reader: &Reader, id: &str,
    after: &LedgerPosition, limit: u32, global: bool,
) -> Result<SubscriptionPage, AtomicError> {
    let registration = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if registration.domain_id != reader.domain_id || registration.seat_id != reader.seat_id
        || (global && (registration.purpose != SessionPurpose::Secretary || reader.domain_id != "global")) {
        return Err(AtomicError::OperationConflict);
    }
    let expected_scope = if global { "GLOBAL" } else if registration.purpose == SessionPurpose::SideChat { "SIDE" } else { "PROJECT" };
    let old = subscription(connection, id)?.ok_or(AtomicError::InvalidRecord("subscriptionId"))?;
    let scope = subscription_scope(connection, id)?;
    if !old.active
        || old.reader_session_id != reader.session_id
        || old.epoch != after.epoch
        || old.cursor != after.cursor
        || scope != expected_scope
    {
        return Err(AtomicError::OperationConflict);
    }
    let page = if global { query_global(connection, after, limit)? }
        else { query(connection, reader, after, limit)? };
    let position = if page.events.len() == usize::try_from(limit).unwrap_or(usize::MAX) {
        page.events
            .last()
            .map_or(after.cursor, |event| event.cursor)
    } else {
        page.position.cursor
    };
    let statement = Statement::prepare(
        connection.as_ptr(),
        "UPDATE v37_ledger_subscription SET cursor = ?, revision = revision + 1
         WHERE subscription_id = ? AND reader_id = ? AND cursor = ?
           AND revision = ? AND state = 'ACTIVE'",
    )?;
    statement.bind_i64(1, position as i64)?;
    statement.bind_text(2, id)?;
    statement.bind_text(3, &reader.session_id)?;
    statement.bind_i64(4, old.cursor as i64)?;
    statement.bind_i64(5, old.revision as i64)?;
    statement.step_done()?;
    let next = subscription(connection, id)?.ok_or_else(|| {
        AtomicError::DurabilityContractFailed("subscription missing after update".into())
    })?;
    if next.revision != old.revision + 1 || next.cursor != position {
        return Err(AtomicError::OperationConflict);
    }
    Ok(SubscriptionPage {
        subscription: next,
        events: page.events,
    })
}

pub(crate) fn end_subscription(
    connection: &mut VerifiedDatabaseConnection<'_>,
    reader: &Reader,
    id: &str,
    expected_revision: u64,
) -> Result<Subscription, AtomicError> {
    end_with_scope(connection, reader, id, expected_revision, false)
}

pub(crate) fn end_global_subscription(
    connection: &mut VerifiedDatabaseConnection<'_>, reader: &Reader, id: &str,
    expected_revision: u64,
) -> Result<Subscription, AtomicError> {
    end_with_scope(connection, reader, id, expected_revision, true)
}

fn subscription_scope(connection: &VerifiedDatabaseConnection<'_>, id: &str)
    -> Result<String, AtomicError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT reader_kind FROM v37_ledger_subscription WHERE subscription_id = ?")?;
    row.bind_text(1, id)?;
    if !row.step_row()? { return Err(AtomicError::InvalidRecord("subscriptionId")); }
    let kind = row.column_text(0)?;
    if row.step_row()? { return Err(AtomicError::OperationConflict); }
    Ok(kind)
}

fn end_with_scope(
    connection: &mut VerifiedDatabaseConnection<'_>, reader: &Reader, id: &str,
    expected_revision: u64, global: bool,
) -> Result<Subscription, AtomicError> {
    let registration = registered(connection, &reader.session_id)?
        .ok_or(AtomicError::InvalidRecord("unregistered reader"))?;
    if registration.domain_id != reader.domain_id || registration.seat_id != reader.seat_id
        || (global && (registration.purpose != SessionPurpose::Secretary || reader.domain_id != "global")) {
        return Err(AtomicError::OperationConflict);
    }
    let expected_scope = if global { "GLOBAL" } else if registration.purpose == SessionPurpose::SideChat { "SIDE" } else { "PROJECT" };
    let old = subscription(connection, id)?.ok_or(AtomicError::InvalidRecord("subscriptionId"))?;
    let scope = subscription_scope(connection, id)?;
    if !old.active
        || old.reader_session_id != reader.session_id
        || old.revision != expected_revision
        || scope != expected_scope
    {
        return Err(AtomicError::OperationConflict);
    }
    // Validate the current reader and purpose at end as on every read.
    let position = recover(connection)?;
    let after = LedgerPosition { epoch: position.epoch, cursor: old.cursor };
    let _ = if global { query_global(connection, &after, 1)? }
        else { query(connection, reader, &after, 1)? };
    let statement = Statement::prepare(
        connection.as_ptr(),
        "UPDATE v37_ledger_subscription SET state = 'ENDED', revision = revision + 1
         WHERE subscription_id = ? AND reader_id = ? AND revision = ? AND state = 'ACTIVE'",
    )?;
    statement.bind_text(1, id)?;
    statement.bind_text(2, &reader.session_id)?;
    statement.bind_i64(3, expected_revision as i64)?;
    statement.step_done()?;
    let next = subscription(connection, id)?.ok_or_else(|| {
        AtomicError::DurabilityContractFailed("subscription missing after end".into())
    })?;
    if next.active || next.revision != old.revision + 1 {
        return Err(AtomicError::OperationConflict);
    }
    Ok(next)
}

fn scalar(connection: &VerifiedDatabaseConnection<'_>, sql: &str) -> Result<String, AtomicError> {
    let statement = Statement::prepare(connection.as_ptr(), sql)?;
    if !statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger scalar row missing".into(),
        ));
    }
    let value = statement.column_text(0)?;
    if statement.step_row()? {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger scalar returned multiple rows".into(),
        ));
    }
    Ok(value)
}

/// The schema script owns one BEGIN IMMEDIATE/COMMIT. On any error the caller
/// must discard the connection; it must never serve it after uncertain COMMIT.
pub(crate) fn initialize_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<LedgerPosition, AtomicError> {
    ensure_session_schema(connection)?;
    ensure_raw_source_schema(connection)?;
    exec(connection, include_str!("schema.sql"))?;
    recover(connection)
}

/// Recover the durable epoch/cursor and verify every old event still resolves
/// through exactly the original source ID. No second history is consulted.
pub(crate) fn recover(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<LedgerPosition, AtomicError> {
    let missing = scalar(
        connection,
        "SELECT COUNT(*) FROM orchestration_events AS e
         LEFT JOIN v37_ledger_index AS i
           ON i.source_event_id = e.event_id AND i.source_kind = 'legacy'
         WHERE i.cursor IS NULL",
    )?;
    let orphaned = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_index AS i
         LEFT JOIN orchestration_events AS e ON e.event_id = i.source_event_id
         WHERE i.source_kind = 'legacy' AND e.event_id IS NULL",
    )?;
    if missing != "0" || orphaned != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 legacy index divergence: missing={missing}, orphaned={orphaned}"
        )));
    }
    let invalid_v37 = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_index AS i
         LEFT JOIN v37_ledger_session AS s ON s.session_id = i.session_id
         WHERE i.source_kind = 'v37' AND
           (s.session_id IS NULL OR s.domain_id <> i.domain_id OR
            s.seat_id <> i.seat_id OR
            (i.tier = 'SIDE' AND (s.purpose <> 'SIDE_CHAT' OR s.side_id <> i.side_id)) OR
            (i.tier <> 'SIDE' AND (i.side_id IS NOT NULL OR s.purpose = 'SIDE_CHAT')) OR
            (i.tier = 'GLOBAL' AND s.purpose = 'FORMAL_REVIEW'))",
    )?;
    if invalid_v37 != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 v37 session/index divergence: {invalid_v37}"
        )));
    }
    let invalid_binding = scalar(
        connection,
        "SELECT COUNT(*) FROM v37_ledger_legacy_binding AS b
         LEFT JOIN v37_ledger_session AS s ON s.session_id = b.session_id
         WHERE s.session_id IS NULL OR s.purpose <> 'WORK' OR
           s.domain_id <> b.domain_id OR s.seat_id <> b.seat_id",
    )?;
    if invalid_binding != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 legacy binding divergence: {invalid_binding}"
        )));
    }
    let invalid_stream = scalar(
        connection,
        "WITH stream_rows AS (
            SELECT session_id, source_epoch, COUNT(*) AS row_count,
                   MIN(CAST(source_cursor AS INTEGER)) AS first_cursor,
                   COUNT(DISTINCT source_cursor) AS distinct_cursor_count
            FROM v37_ledger_index
            WHERE source_kind = 'v37'
            GROUP BY session_id, source_epoch
         )
         SELECT COUNT(*) FROM v37_ledger_source_stream AS s
          LEFT JOIN stream_rows AS rows
            ON rows.session_id = s.session_id AND rows.source_epoch = s.source_epoch
         WHERE s.last_cursor < 1 OR
           (s.state = 'ACTIVE' AND (
              COALESCE(rows.row_count, 0) <> s.last_cursor OR
              COALESCE(rows.first_cursor, 0) <> 1 OR
              COALESCE(rows.distinct_cursor_count, 0) <> COALESCE(rows.row_count, 0)
           ))",
    )?;
    if invalid_stream != "0" {
        return Err(AtomicError::DurabilityContractFailed(format!(
            "A.1 source stream divergence: {invalid_stream}"
        )));
    }
    let epoch = scalar(
        connection,
        "SELECT epoch FROM v37_ledger_meta WHERE singleton = 1",
    )?;
    if epoch.is_empty() {
        return Err(AtomicError::DurabilityContractFailed(
            "A.1 ledger epoch missing".into(),
        ));
    }
    let cursor = scalar(
        connection,
        "SELECT COALESCE((SELECT seq FROM sqlite_sequence
            WHERE name = 'v37_ledger_index'), 0)",
    )?
    .parse::<u64>()
    .map_err(|error| {
        AtomicError::DurabilityContractFailed(format!("A.1 invalid ledger cursor: {error}"))
    })?;
    Ok(LedgerPosition { epoch, cursor })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::authority;
    use crate::store::digest::content_hash;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use crate::process::{
        NativeBinding, OriginBoundFrame, PrepareRequest, PreparedCustody, ProcessCustodian,
        ProcessLaunch,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn scratch_root() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-ledger-{nonce}"));
        std::fs::create_dir(&path).expect("scratch root");
        path
    }

    fn session(
        domain: &str,
        seat: &str,
        id: &str,
        purpose: SessionPurpose,
        side: Option<&str>,
    ) -> SessionRegistration {
        SessionRegistration {
            domain_id: domain.into(),
            seat_id: seat.into(),
            session_id: id.into(),
            purpose,
            side_id: side.map(str::to_owned),
        }
    }

    fn event_at(
        id: &str,
        session: &SessionRegistration,
        tier: Tier,
        source_cursor: &str,
    ) -> EventInput {
        EventInput {
            event_id: id.into(),
            source_epoch: "source-epoch".into(),
            source_cursor: source_cursor.into(),
            domain_id: session.domain_id.clone(),
            seat_id: session.seat_id.clone(),
            session_id: session.session_id.clone(),
            tier,
            side_id: session.side_id.clone(),
            occurred_at: "2026-09-29T00:00:00Z".into(),
            update_json: r#"{"sessionUpdate":"agent_message_chunk"}"#.into(),
        }
    }

    fn event(id: &str, session: &SessionRegistration, tier: Tier) -> EventInput {
        event_at(id, session, tier, "1")
    }

    #[test]
    fn secretary_history_filters_original_identity_before_paging() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("secretary-history.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(&mut connection, "CREATE TABLE orchestration_events
            (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
             occurred_at TEXT, event_type TEXT, payload_json TEXT)").expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        crate::store::instance::initialize_schema(&mut connection).expect("F schema");
        crate::store::seat::initialize_schema(&mut connection).expect("E schema");
        crate::store::session_transport::initialize_admission_schema(&mut connection)
            .expect("H schema");
        crate::store::session_transport::session_binding::initialize_schema(&mut connection)
            .expect("NativeV2 schema");
        authority::initialize_process_custody_schema(&mut connection).expect("custody schema");
        exec(&mut connection, "INSERT INTO gogoke_v37_instances
            (instance_id,driver_id,home_ref,home_identity,program_digest,version,
             install_state,login_state,revision)
            VALUES('instance','codex','history-home','history-identity','sha256:fixture',
                   'fixture','INSTALLED','LOGGED_OUT',1);
            INSERT INTO gogoke_v37_seats
            (domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision)
            VALUES('global','secretary','current','USER','LONG','instance','BUSY',1,1),
                  ('global','other','other-incarnation','USER','LONG','instance','BUSY',1,1);
            INSERT INTO gogoke_v37_seat_secretary
            (singleton,domain_id,seat_id,incarnation,request_id,fingerprint)
            VALUES(1,'global','secretary','current','designate','fixture')"
        ).expect("real E/F identity schema");
        let entries = [
            ("reader", "secretary", "current", "NATIVE_V2", "COMMITTED", SessionPurpose::Secretary),
            ("foreign", "other", "current", "NATIVE_V2", "COMMITTED", SessionPurpose::Secretary),
            ("old-incarnation", "secretary", "previous", "NATIVE_V2", "RELEASED", SessionPurpose::Secretary),
            ("legacy", "secretary", "current", "LEGACY_V1", "RELEASED", SessionPurpose::Secretary),
            ("work", "secretary", "current", "NATIVE_V2", "COMMITTED", SessionPurpose::Work),
            ("unbound", "secretary", "current", "NATIVE_V2", "COMMITTED", SessionPurpose::Secretary),
            ("unresolved", "secretary", "current", "NATIVE_V2", "COMMITTED", SessionPurpose::Secretary),
            ("good-old", "secretary", "current", "NATIVE_V2", "RELEASED", SessionPurpose::Secretary),
        ];
        let mut custodians = Vec::new();
        for (id, seat, incarnation, provenance, state, purpose) in entries {
            let registration = session("global", seat, id, purpose, None);
            register_session(&mut connection, &registration).expect("register");
            let exact_source = id != "unbound" && id != "unresolved";
            let mut prepared_source = None;
            if exact_source {
                let command = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"))
                    .join("System32").join("cmd.exe");
                let mut launch = ProcessLaunch::new(command.clone());
                launch.arguments = vec!["/D".into(), "/C".into(), format!("echo {id}")];
                launch.protocol_stdio = true;
                launch.persistent_protocol_stdio = true;
                let mut custodian = ProcessCustodian::new().expect("native custodian");
                let prepared = custodian.prepare(&PrepareRequest {
                    binding: NativeBinding {
                        binary_digest_sha256: content_hash(&fs::read(&command).expect("cmd bytes")),
                        profile_id: format!("history-{id}"), domain_id: "global".into(),
                        generation: "1".into(),
                    }, launch,
                }).expect("prepare original process");
                authority::record_prepared_process(&mut connection, &format!("operation-{id}"), &prepared)
                    .expect("original custody");
                prepared_source = Some((custodian, prepared));
            }
            if id != "unbound" {
                exec(&mut connection, &format!(
                    "INSERT INTO gogoke_v37_session_binding_v2
                     (domain_id,session_id,seat_id,seat_incarnation,
                      seat_authorization_generation,selected_instance_id,provenance)
                     VALUES('global','{id}','{seat}','{incarnation}',1,'instance','{provenance}');
                     INSERT INTO gogoke_v37_h_owner_binding
                     (binding_id,instance_id,domain_id,kind,owner_id,generation,state)
                     VALUES('binding-{id}','instance','global','SESSION','{id}','1','ACTIVE');
                     INSERT INTO gogoke_v37_instance_homes
                     (home_id,instance_id,domain_id,kind,owner_id,generation,state,revision)
                     VALUES('home-{id}','instance','global','SESSION','{id}','1','ACTIVE',1);
                     INSERT INTO gogoke_v37_h_claim
                     (domain_id,session_id,instance_id,home_id,binding_id,generation,
                      state,revision,process_operation_id)
                     VALUES('global','{id}','instance','home-{id}','binding-{id}',
                            '1','COMMITTED',2,{});{}",
                    if exact_source {format!("'operation-{id}'")} else {"NULL".into()},
                    if exact_source {format!(
                        "INSERT INTO gogoke_v37_h_process_episode
                         (domain_id,request_id,session_id,generation,raw_hex,
                          previous_revision,result_revision,process_operation_id,
                          instance_id,home_id,binding_id,seat_id,seat_incarnation,phase)
                         VALUES('global','open-{id}','{id}','1','6f70656e',1,2,
                                'operation-{id}','instance','home-{id}','binding-{id}',
                                '{seat}','{incarnation}','ACTIVE')"
                    )} else {String::new()}
                )).expect("original H relationship");
            }
            if let Some((mut custodian, prepared)) = prepared_source {
                custodian.activate(&prepared).expect("activate source");
                authority::mark_process_active(&mut connection, &format!("operation-{id}"), &prepared)
                    .expect("active custody");
                let frame = custodian.read_persistent_child_frame(&prepared.ticket,
                    Duration::from_secs(5)).expect("original frame");
                capture_raw_source(&mut connection, &frame, &format!("operation-{id}"),
                    "source-epoch", "1").expect("capture exact source");
                custodians.push(custodian);
            }
            let mut input = event(id, &registration, Tier::Global);
            if id == "good-old" {
                input.update_json = r#"{"sessionUpdate":"agent_message_chunk","text":"UNKNOWN"}"#.into();
            }
            record(&mut connection, &input).expect("source event");
            if exact_source {
                let before = query_secretary_history(&connection,
                    &Reader {domain_id:"global".into(),seat_id:"secretary".into(),session_id:"reader".into()},
                    &start, 100).expect("unresolved source excluded");
                assert!(!before.events.iter().any(|event| event.input.event_id == id));
                resolve_raw_source(&mut connection, &format!("operation-{id}"),
                    "source-epoch", "1", id).expect("resolve original source");
                if state == "RELEASED" {
                    exec(&mut connection, &format!(
                        "INSERT INTO gogoke_v37_h_owner_binding
                         (binding_id,instance_id,domain_id,kind,owner_id,generation,state)
                         VALUES('binding-{id}-2','instance','global','SESSION','{id}','2','ACTIVE');
                         INSERT INTO gogoke_v37_instance_homes
                         (home_id,instance_id,domain_id,kind,owner_id,generation,state,revision)
                         VALUES('home-{id}-2','instance','global','SESSION','{id}','2','ACTIVE',1);
                         UPDATE gogoke_v37_h_claim SET state='RELEASED',generation='2',
                           binding_id='binding-{id}-2',home_id='home-{id}-2',
                           process_operation_id=NULL WHERE session_id='{id}'"
                    )).expect("released historical session");
                }
            }
        }
        let reader = Reader {domain_id:"global".into(),seat_id:"secretary".into(),session_id:"reader".into()};
        let first = query_secretary_history(&connection, &reader, &start, 1).expect("first page");
        assert_eq!(first.events.iter().map(|event| event.input.event_id.as_str()).collect::<Vec<_>>(), vec!["reader"]);
        let second = query_secretary_history(&connection, &reader,
            &LedgerPosition {epoch:start.epoch.clone(),cursor:first.events[0].cursor}, 1).expect("second page");
        assert_eq!(second.events[0].input.event_id, "good-old");
        assert_eq!(second.events[0].input.source_epoch, "source-epoch");
        assert_eq!(second.events[0].input.source_cursor, "1");
        assert!(second.events[0].input.update_json.contains("UNKNOWN"));
        assert_eq!(second.position.epoch, start.epoch);
        assert_eq!(second.position.cursor, 8);
        assert_eq!(query_secretary_history(&connection, &reader, &start, 100)
            .expect("only original secretary sources").events.len(), 2);
        exec(&mut connection,
            "UPDATE v37_ledger_raw_source SET process_ticket='wrong-ticket'
             WHERE resolved_event_id='good-old'"
        ).expect("break original raw custody link");
        assert!(query_secretary_history(&connection, &reader,
            &LedgerPosition {epoch:start.epoch.clone(),cursor:first.events[0].cursor}, 1)
            .expect("broken original source excluded").events.is_empty());
        exec(&mut connection,
            "UPDATE v37_ledger_raw_source
             SET process_ticket=(SELECT ticket FROM gogoke_coordination_process_custody
                                 WHERE operation_id='operation-good-old')
             WHERE resolved_event_id='good-old'"
        ).expect("restore original raw custody link");
        assert!(query(&connection, &reader, &start, 1).is_err());
        for denied in [
            Reader {session_id:"foreign".into(),seat_id:"other".into(),..reader.clone()},
            Reader {session_id:"legacy".into(),..reader.clone()},
            Reader {session_id:"unbound".into(),..reader.clone()},
            Reader {session_id:"work".into(),..reader.clone()},
        ] {
            assert!(query_secretary_history(&connection, &denied, &start, 1).is_err());
        }
        assert!(query_secretary_history(&connection, &reader,
            &LedgerPosition {epoch:"wrong".into(),cursor:0}, 1).is_err());
        assert!(query_secretary_history(&connection, &reader, &start, 1001).is_err());
        exec(&mut connection,
            "UPDATE gogoke_v37_h_owner_binding SET kind='CALL' WHERE owner_id='reader'"
        ).expect("break original reader association");
        assert!(query_secretary_history(&connection, &reader, &start, 1).is_err());
        connection.close_checked().expect("close");
    }

    pub(crate) fn initialize_raw_h_fixture(
        connection: &mut VerifiedDatabaseConnection<'_>,
    ) {
        exec(
            connection,
            "CREATE TABLE gogoke_v37_h_owner_binding(
                binding_id TEXT PRIMARY KEY,
                instance_id TEXT NOT NULL,
                domain_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                owner_id TEXT NOT NULL,
                generation TEXT NOT NULL,
                state TEXT NOT NULL
            ) STRICT;
            CREATE TABLE gogoke_v37_h_claim(
                domain_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                instance_id TEXT NOT NULL,
                home_id TEXT NOT NULL,
                binding_id TEXT NOT NULL,
                generation TEXT NOT NULL,
                state TEXT NOT NULL,
                revision INTEGER NOT NULL,
                process_operation_id TEXT UNIQUE,
                stop_fact_id TEXT,
                PRIMARY KEY(domain_id, session_id)
            ) STRICT;
            CREATE TABLE gogoke_v37_h_process_episode(
                domain_id TEXT NOT NULL,session_id TEXT NOT NULL,generation TEXT NOT NULL,
                process_operation_id TEXT UNIQUE,phase TEXT NOT NULL,stop_fact_id TEXT
            ) STRICT",
        )
        .expect("H raw source fixture schema");
        authority::initialize_process_custody_schema(connection)
            .expect("process custody fixture schema");
    }

    pub(crate) fn raw_process_fixture(
        connection: &mut VerifiedDatabaseConnection<'_>,
        registration: &SessionRegistration,
        operation_id: &str,
    ) -> (ProcessCustodian, PreparedCustody, OriginBoundFrame) {
        raw_process_fixture_with_command(
            connection,
            registration,
            operation_id,
            "echo raw-one&echo raw-two",
        )
    }

    fn raw_process_fixture_with_command(
        connection: &mut VerifiedDatabaseConnection<'_>,
        registration: &SessionRegistration,
        operation_id: &str,
        command_line: &str,
    ) -> (ProcessCustodian, PreparedCustody, OriginBoundFrame) {
        let command = PathBuf::from(
            std::env::var_os("SystemRoot").expect("SystemRoot"),
        )
        .join("System32")
        .join("cmd.exe");
        let mut launch = ProcessLaunch::new(command.clone());
        launch.arguments = vec![
            "/D".into(),
            "/C".into(),
            command_line.into(),
        ];
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        let request = PrepareRequest {
            binding: NativeBinding {
                binary_digest_sha256: content_hash(
                    &fs::read(&command).expect("cmd bytes"),
                ),
                profile_id: "profile-raw-source".into(),
                domain_id: registration.domain_id.clone(),
                generation: "1".into(),
            },
            launch,
        };
        let mut custodian = ProcessCustodian::new().expect("custodian");
        let prepared = custodian.prepare(&request).expect("prepare");
        authority::record_prepared_process(connection, operation_id, &prepared)
            .expect("durable custody");
        exec(
            connection,
            &format!(
                "INSERT INTO gogoke_v37_h_owner_binding
                 VALUES ('binding-raw-{operation_id}', 'instance-raw', '{}',
                         'SESSION', '{}', '1', 'ACTIVE');
                 INSERT INTO gogoke_v37_h_claim
                 (domain_id, session_id, instance_id, home_id, binding_id,
                  generation, state, revision, process_operation_id)
                 VALUES ('{}', '{}', 'instance-raw', 'home-raw',
                         'binding-raw-{operation_id}', '1', 'COMMITTED', 1,
                          '{operation_id}');
                  INSERT INTO gogoke_v37_h_process_episode
                  (domain_id,session_id,generation,process_operation_id,phase)
                  VALUES ('{}','{}','1','{operation_id}','ACTIVE')",
                registration.domain_id,
                registration.session_id,
                registration.domain_id,
                registration.session_id,
                registration.domain_id,
                registration.session_id,
            ),
        )
        .expect("durable H claim");
        custodian.activate(&prepared).expect("activate");
        authority::mark_process_active(connection, operation_id, &prepared)
            .expect("active custody");
        let frame = custodian
            .read_persistent_child_frame(&prepared.ticket, Duration::from_secs(5))
            .expect("exact child frame");
        (custodian, prepared, frame)
    }

    #[test]
    fn same_open_append_scope_subscription_and_restart() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
            (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
             occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let lead = session(
            "project-a",
            "lead",
            "lead-session",
            SessionPurpose::Work,
            None,
        );
        let worker = session(
            "project-a",
            "worker",
            "worker-session",
            SessionPurpose::Work,
            None,
        );
        let other = session(
            "project-b",
            "lead",
            "other-session",
            SessionPurpose::Work,
            None,
        );
        let side = session(
            "project-a",
            "owner",
            "side-session",
            SessionPurpose::SideChat,
            Some("side-a"),
        );
        let review = session(
            "project-a",
            "review",
            "review-session",
            SessionPurpose::FormalReview,
            None,
        );
        for item in [&lead, &worker, &other, &side, &review] {
            register_session(&mut connection, item).expect("register");
        }
        assert!(register_session(
            &mut connection,
            &session(
                "project-a",
                "lead",
                "lead-session",
                SessionPurpose::FormalReview,
                None
            )
        )
        .is_err());
        let lead_event = event("lead-private", &lead, Tier::Seat);
        let saved = record(&mut connection, &lead_event).expect("append");
        assert_eq!(record(&mut connection, &lead_event).expect("replay"), saved);
        let mut changed = lead_event.clone();
        changed.update_json = r#"{"sessionUpdate":"usage_update"}"#.into();
        assert!(record(&mut connection, &changed).is_err());
        record(
            &mut connection,
            &event("worker-private", &worker, Tier::Seat),
        )
        .expect("worker");
        record(
            &mut connection,
            &event("other-project", &other, Tier::Project),
        )
        .expect("other");
        record(&mut connection, &event("side-private", &side, Tier::Side)).expect("side");
        record(
            &mut connection,
            &event_at("shared", &lead, Tier::Project, "2"),
        )
        .expect("shared");
        record(
            &mut connection,
            &event_at("global", &lead, Tier::Global, "3"),
        )
        .expect("global");
        record(
            &mut connection,
            &event_at("side-latest", &side, Tier::Side, "2"),
        )
        .expect("latest side");
        let lead_reader = Reader {
            domain_id: lead.domain_id.clone(),
            seat_id: lead.seat_id.clone(),
            session_id: lead.session_id.clone(),
        };
        let worker_reader = Reader {
            domain_id: worker.domain_id.clone(),
            seat_id: worker.seat_id.clone(),
            session_id: worker.session_id.clone(),
        };
        let side_reader = Reader {
            domain_id: side.domain_id.clone(),
            seat_id: side.seat_id.clone(),
            session_id: side.session_id.clone(),
        };
        let review_reader = Reader {
            domain_id: review.domain_id.clone(),
            seat_id: review.seat_id.clone(),
            session_id: review.session_id.clone(),
        };
        assert_eq!(
            query(&connection, &lead_reader, &start, 100)
                .expect("lead query")
                .events
                .len(),
            2
        );
        assert_eq!(
            query(&connection, &worker_reader, &start, 100)
                .expect("worker query")
                .events
                .len(),
            2
        );
        assert_eq!(
            query(&connection, &side_reader, &start, 100)
                .expect("side query")
                .events
                .len(),
            5
        );
        let own_first = query_own_side(&connection, &side_reader, &start, 1)
            .expect("registered side transcript");
        assert_eq!(own_first.position.cursor, 7);
        assert_eq!(own_first.events[0].input.event_id, "side-private");
        let own_next = query_own_side(
            &connection,
            &side_reader,
            &LedgerPosition { epoch: start.epoch.clone(), cursor: own_first.events[0].cursor },
            1,
        ).expect("next own turn");
        assert_eq!(own_next.events[0].input.event_id, "side-latest");
        assert!(query_own_side(&connection, &lead_reader, &start, 1).is_err());
        assert!(query_own_side(&connection, &review_reader, &start, 1).is_err());
        assert!(query_own_side(&connection, &Reader { seat_id: "lead".into(), ..side_reader.clone() }, &start, 1).is_err());
        assert!(query_own_side(&connection, &side_reader, &LedgerPosition { epoch: "wrong".into(), cursor: 0 }, 1).is_err());
        assert!(query(&connection, &review_reader, &start, 100).is_err());
        assert_eq!(
            query_global(&connection, &start, 100)
                .expect("global query")
                .events
                .len(),
            7
        );
        let subscribed =
            subscribe(&mut connection, &lead_reader, "sub-lead", &start, 1).expect("subscribe");
        assert_eq!(subscribed.events.len(), 1);
        assert!(resume_subscription(&mut connection, &lead_reader, "sub-lead", &start, 1).is_err());
        delete_side_events(&mut connection, "project-a", "side-a").expect("delete side entries");
        assert_eq!(
            recover(&connection)
                .expect("high-water after delete")
                .cursor,
            7
        );
        assert_eq!(
            query_global(&connection, &start, 100)
                .expect("remaining")
                .events
                .len(),
            5
        );
        connection.close_checked().expect("close");
        let mut reopened = open_existing(&root, &db).expect("reopen");
        let recovered = recover(&reopened).expect("recover");
        assert_eq!(recovered.cursor, 7);
        assert_eq!(recovered.epoch, start.epoch);
        let next = resume_subscription(
            &mut reopened,
            &lead_reader,
            "sub-lead",
            &LedgerPosition {
                epoch: start.epoch.clone(),
                cursor: subscribed.subscription.cursor,
            },
            100,
        )
        .expect("resume after restart");
        assert_eq!(next.events.len(), 1);
        assert!(
            !end_subscription(
                &mut reopened,
                &lead_reader,
                "sub-lead",
                next.subscription.revision
            )
            .expect("end")
            .active
        );
        reopened.close_checked().expect("close reopened");
    }

    #[test]
    fn legacy_rows_are_read_by_reference_only_after_explicit_binding() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let lead = session(
            "project-a",
            "lead",
            "lead-session",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &lead).expect("register");
        exec(
            &mut connection,
            "INSERT INTO orchestration_events VALUES
             (1, 'old-event', 'old-thread', '2026-09-29T00:00:00Z',
              'thread.message-sent', '{\"role\":\"user\",\"text\":\"hello\"}')",
        )
        .expect("old append and trigger");
        let reader = Reader {
            domain_id: lead.domain_id.clone(),
            seat_id: lead.seat_id.clone(),
            session_id: lead.session_id.clone(),
        };
        assert!(query(&connection, &reader, &start, 100)
            .expect("unbound query")
            .events
            .is_empty());
        bind_legacy_thread(&mut connection, "old-thread", &lead).expect("bind");
        let page = query(&connection, &reader, &start, 100).expect("bound query");
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].input.event_id, "old-event");
        assert!(page.events[0].input.update_json.contains("legacyPayload"));
        assert!(bind_legacy_thread(
            &mut connection,
            "old-thread",
            &session(
                "project-a",
                "worker",
                "worker-session",
                SessionPurpose::Work,
                None
            )
        )
        .is_err());
        connection.close_checked().expect("close");
    }

    #[test]
    fn source_stream_rejects_gap_duplicate_cursor_and_accepts_epoch_rollover() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        initialize_schema(&mut connection).expect("schema");
        let registration = session(
            "project-a",
            "seat-a",
            "session-a",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        record(
            &mut connection,
            &event_at("event-1", &registration, Tier::Seat, "1"),
        )
        .expect("first");
        // The trigger is the atomicity backstop for any future direct index
        // writer: a rejected insert must not advance the durable high-water
        // row or leave a partially accepted source event.
        assert!(exec(
            &mut connection,
            "INSERT INTO v37_ledger_index
             (source_event_id, source_kind, source_cursor, source_epoch,
              domain_id, seat_id, session_id, tier, occurred_at, update_json)
             VALUES ('direct-gap', 'v37', '3', 'source-epoch',
                     'project-a', 'seat-a', 'session-a', 'SEAT',
                     '2026-09-29T00:00:00Z',
                     '{\"sessionUpdate\":\"agent_message_chunk\"}')",
        )
        .is_err());
        assert_eq!(recover(&connection).expect("rollback recovery").cursor, 1);
        let gap = event_at("event-3", &registration, Tier::Seat, "3");
        assert!(matches!(
            record(&mut connection, &gap),
            Err(AtomicError::DurabilityContractFailed(message))
                if message.contains("source stream gap")
        ));
        let duplicate_cursor = event_at("event-2", &registration, Tier::Seat, "1");
        assert!(matches!(
            record(&mut connection, &duplicate_cursor),
            Err(AtomicError::DurabilityContractFailed(message))
                if message.contains("source stream duplicate or rewind")
        ));
        record(
            &mut connection,
            &event_at("event-2", &registration, Tier::Seat, "2"),
        )
        .expect("contiguous");
        let rollover = EventInput {
            source_epoch: "source-epoch-2".into(),
            ..event_at("event-epoch-2", &registration, Tier::Seat, "1")
        };
        record(&mut connection, &rollover).expect("epoch rollover");
        assert_eq!(recover(&connection).expect("recover").cursor, 3);
        connection.close_checked().expect("close");
    }

    #[test]
    fn raw_source_capture_preserves_exact_frame_and_replay_conflict_is_atomic() {
        let _guard = route_b_test_guard();
        assert!(raw_bytes(b"missing-lf").is_err());
        let mut oversized = vec![b'x'; RAW_SOURCE_MAX_BYTES + 1];
        *oversized.last_mut().expect("oversized frame") = b'\n';
        assert!(raw_bytes(&oversized).is_err());
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        initialize_schema(&mut connection).expect("schema");
        exec(
            &mut connection,
            "CREATE TEMP TABLE v37_ledger_raw_source AS
                 SELECT * FROM main.v37_ledger_raw_source WHERE 0;
             INSERT INTO temp.v37_ledger_raw_source
                 (operation_id, process_ticket, custodian_nonce, domain_id,
                  session_id, generation, source_epoch, source_cursor, raw_bytes,
                  state, resolved_event_id, no_event_reason)
             VALUES ('operation-raw', 'temp-ticket', 'temp-nonce', 'project-raw',
                     'session-raw', '1', 'source-epoch', '1', X'74656D702D736861646F770A',
                     'NO_EVENT', NULL, 'temp_shadow')",
        )
        .expect("temp shadow row");
        let registration = session(
            "project-raw",
            "seat-raw",
            "session-raw",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        initialize_raw_h_fixture(&mut connection);
        let (mut custodian, _prepared, first_frame) =
            raw_process_fixture(&mut connection, &registration, "operation-raw");
        let second_frame = custodian
            .read_persistent_child_frame(
                &first_frame.custody().ticket,
                Duration::from_secs(5),
            )
            .expect("second exact child frame");

        let first = capture_raw_source(
            &mut connection,
            &first_frame,
            "operation-raw",
            "source-epoch",
            "1",
        )
        .expect("capture");
        assert_eq!(first.raw_bytes, first_frame.bytes());
        assert_eq!(first.state, RawSourceState::Pending);
        assert!(first.raw_bytes.ends_with(b"\n"));
        assert_eq!(
            capture_raw_source(
                &mut connection,
                &first_frame,
                "operation-raw",
                "source-epoch",
                "1",
            )
            .expect("idempotent replay"),
            first
        );
        assert!(matches!(
            capture_raw_source(
                &mut connection,
                &second_frame,
                "operation-raw",
                "source-epoch",
                "1",
            ),
            Err(AtomicError::OperationConflict)
        ));
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM main.v37_ledger_raw_source",
            )
            .expect("raw row count"),
            "1"
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM temp.v37_ledger_raw_source",
            )
            .expect("temp shadow row count"),
            "1"
        );
        drop(custodian);
        connection.close_checked().expect("close");
    }

    #[test]
    fn pending_raw_source_survives_restart_and_resolves_without_new_bytes() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let registration = session(
            "project-recovery",
            "seat-recovery",
            "session-recovery",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        initialize_raw_h_fixture(&mut connection);
        let (mut custodian, _prepared, frame) =
            raw_process_fixture(&mut connection, &registration, "operation-recovery");
        let captured = capture_raw_source(
            &mut connection,
            &frame,
            "operation-recovery",
            "source-epoch",
            "1",
        )
        .expect("capture");
        // The unit fixture cannot invoke native H admission; these writes
        // model its durable STOPPED -> RELEASED ordering and proof fields.
        exec(
            &mut connection,
            "UPDATE gogoke_coordination_process_custody
             SET state = 'STOPPED', stop_proof_hash = 'proof-recovery'
             WHERE operation_id = 'operation-recovery';
             UPDATE gogoke_v37_h_claim
             SET state = 'RELEASED', stop_fact_id = 'proof-recovery'
              WHERE process_operation_id = 'operation-recovery';
              UPDATE gogoke_v37_h_process_episode
              SET phase = 'STOPPED', stop_fact_id = 'proof-recovery'
              WHERE process_operation_id = 'operation-recovery'",
        )
        .expect("simulate durable stop followed by H release");
        exec(
            &mut connection,
            "UPDATE gogoke_v37_h_process_episode
             SET stop_fact_id = 'different-proof'
             WHERE process_operation_id = 'operation-recovery'",
        )
        .expect("fixture proof mismatch");
        assert!(matches!(
            read_pending_raw_source(
                &connection,
                "operation-recovery",
                "source-epoch",
                "1",
            ),
            Err(AtomicError::OperationConflict)
        ));
        exec(
            &mut connection,
            "UPDATE gogoke_v37_h_process_episode
             SET stop_fact_id = 'proof-recovery'
             WHERE process_operation_id = 'operation-recovery'",
        )
        .expect("fixture proof restore");
        assert!(matches!(
            capture_raw_source(
                &mut connection,
                &frame,
                "operation-recovery",
                "source-epoch",
                "1",
            ),
            Err(AtomicError::OperationConflict)
        ));
        drop(custodian);
        connection.close_checked().expect("close");

        let mut reopened = open_existing(&root, &db).expect("reopen");
        let pending = read_pending_raw_source(
            &reopened,
            "operation-recovery",
            "source-epoch",
            "1",
        )
        .expect("read pending")
        .expect("pending frame");
        assert_eq!(pending.raw_bytes, captured.raw_bytes);
        assert_eq!(pending.state, RawSourceState::Pending);

        let normalized = record(
            &mut reopened,
            &event_at("normalized-event", &registration, Tier::Session, "1"),
        )
        .expect("normalize existing raw frame");
        let resolution = resolve_raw_source(
            &mut reopened,
            "operation-recovery",
            "source-epoch",
            "1",
            &normalized.input.event_id,
        )
        .expect("resolve pending");
        assert_eq!(resolution.event_id, "normalized-event");
        assert_eq!(
            resolve_raw_source(
                &mut reopened,
                "operation-recovery",
                "source-epoch",
                "1",
                "normalized-event",
            )
            .expect("idempotent resolution"),
            resolution
        );
        assert_eq!(
            query(
                &reopened,
                &Reader {
                    domain_id: registration.domain_id.clone(),
                    seat_id: registration.seat_id.clone(),
                    session_id: registration.session_id.clone(),
                },
                &start,
                10,
            )
            .expect("ordinary ledger query")
            .events
            .len(),
            1
        );
        reopened.close_checked().expect("close reopened");
    }

    #[test]
    fn raw_source_schema_upgrade_preserves_pending_bytes() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT);
             CREATE TABLE main.v37_ledger_raw_source (
                 operation_id TEXT NOT NULL,
                 process_ticket TEXT NOT NULL,
                 custodian_nonce TEXT NOT NULL,
                 domain_id TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 generation TEXT NOT NULL,
                 source_epoch TEXT NOT NULL,
                 source_cursor TEXT NOT NULL,
                 raw_bytes BLOB NOT NULL,
                 state TEXT NOT NULL,
                 resolved_event_id TEXT,
                 PRIMARY KEY (operation_id, source_epoch, source_cursor),
                 UNIQUE (process_ticket, source_epoch, source_cursor)
             ) STRICT;
             INSERT INTO main.v37_ledger_raw_source
             VALUES ('operation-old', 'ticket-old', 'nonce-old', 'project-old',
                     'session-old', '1', 'epoch-old', '1', X'610A', 'PENDING', NULL)",
        )
        .expect("legacy raw journal");
        initialize_schema(&mut connection).expect("upgrade schema");
        let record = read_raw_source(
            &connection,
            &RawSourceKey {
                operation_id: "operation-old".into(),
                source_epoch: "epoch-old".into(),
                source_cursor: "1".into(),
            },
        )
        .expect("read upgraded raw row")
        .expect("upgraded row");
        assert_eq!(record.state, RawSourceState::Pending);
        assert_eq!(record.raw_bytes, b"a\n");
        assert_eq!(record.no_event_reason, None);
        connection.close_checked().expect("close");
    }

    #[test]
    fn secretary_purpose_migration_preserves_old_rows_and_refuses_unknown_shapes() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        let (old_ddl, current_ddl) = session_schema_definitions().expect("known DDL");
        connection.execute("CREATE TABLE orchestration_events
            (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
             occurred_at TEXT, event_type TEXT, payload_json TEXT)").expect("legacy events");
        // The historical schema script executed IF NOT EXISTS, while SQLite
        // stores the normalized statement without that clause.
        connection.execute(&old_ddl.replacen("CREATE TABLE ",
            "CREATE TABLE IF NOT EXISTS ", 1)).expect("old session table");
        assert_eq!(session_schema_state(&connection).unwrap().as_deref(), Some(old_ddl.as_str()));
        connection.execute("INSERT INTO main.v37_ledger_session
            (session_id, domain_id, seat_id, purpose, side_id) VALUES
            ('work', 'project-a', 'lead', 'WORK', NULL),
            ('handoff', 'project-a', 'lead', 'HANDOFF', NULL),
            ('side', 'project-a', 'owner', 'SIDE_CHAT', 'side-a'),
            ('review', 'project-b', 'auditor', 'FORMAL_REVIEW', NULL);
            INSERT INTO orchestration_events
            (sequence, event_id, stream_id, occurred_at, event_type, payload_json)
            VALUES (1, 'old-event', 'old-thread', '2026-09-29T00:00:00Z',
                    'old', '{\"original\":true}')").expect("old history");
        let before = scalar(&connection, "SELECT group_concat(
            quote(session_id) || ':' || quote(domain_id) || ':' || quote(seat_id)
            || ':' || quote(purpose) || ':' || quote(side_id), '|')
            FROM (SELECT * FROM v37_ledger_session ORDER BY session_id)")
            .expect("old rows");
        connection.execute("CREATE INDEX unexpected_session_index
            ON v37_ledger_session(domain_id)").expect("foreign index fixture");
        assert!(initialize_schema(&mut connection).is_err());
        assert_eq!(session_schema_state(&connection).unwrap().as_deref(), Some(old_ddl.as_str()));
        assert_eq!(scalar(&connection, "SELECT COUNT(*) FROM main.v37_ledger_session").unwrap(), "4");
        connection.execute("DROP INDEX unexpected_session_index").expect("remove fixture");
        connection.execute("CREATE TRIGGER unexpected_session_trigger
            AFTER INSERT ON v37_ledger_session BEGIN SELECT 1; END")
            .expect("foreign trigger fixture");
        assert!(initialize_schema(&mut connection).is_err());
        connection.execute("DROP TRIGGER unexpected_session_trigger").expect("remove trigger");
        connection.execute("CREATE TABLE dependent_session_fk (
            session_id TEXT REFERENCES v37_ledger_session(session_id)) STRICT")
            .expect("foreign key fixture");
        assert!(initialize_schema(&mut connection).is_err());
        connection.execute("DROP TABLE dependent_session_fk").expect("remove foreign key");

        let position = initialize_schema(&mut connection).expect("upgrade old session schema");
        assert_eq!(session_schema_state(&connection).unwrap().as_deref(), Some(current_ddl.as_str()));
        assert_eq!(before, scalar(&connection, "SELECT group_concat(
            quote(session_id) || ':' || quote(domain_id) || ':' || quote(seat_id)
            || ':' || quote(purpose) || ':' || quote(side_id), '|')
            FROM (SELECT * FROM v37_ledger_session ORDER BY session_id)").unwrap());
        assert_eq!(position.cursor, 1);
        assert_eq!(scalar(&connection, "SELECT source_event_id FROM v37_ledger_index
            WHERE source_kind = 'legacy'").unwrap(), "old-event");
        assert_eq!(scalar(&connection, "SELECT payload_json FROM orchestration_events
            WHERE event_id = 'old-event'").unwrap(), "{\"original\":true}");
        for (id, purpose) in [
            ("work", SessionPurpose::Work),
            ("handoff", SessionPurpose::Handoff),
            ("side", SessionPurpose::SideChat),
            ("review", SessionPurpose::FormalReview),
        ] {
            assert_eq!(registered(&connection, id).unwrap().unwrap().purpose, purpose);
        }
        let secretary = session("global", "secretary-seat", "secretary-session",
            SessionPurpose::Secretary, None);
        register_session(&mut connection, &secretary).expect("secretary registration");
        assert_eq!(registered(&connection, "secretary-session").unwrap(), Some(secretary.clone()));
        assert!(register_session(&mut connection, &session("global", "secretary-seat",
            "secretary-session", SessionPurpose::Work, None)).is_err());
        assert!(query(&connection, &Reader { domain_id: "global".into(),
            seat_id: "secretary-seat".into(), session_id: "secretary-session".into() },
            &position, 1).is_err());
        connection.close_checked().expect("close migrated");

        for table_name in ["v37_ledger_session", "V37_LEDGER_SESSION"] {
            let unknown_path = scratch_root();
            let unknown_root = RootLock::acquire(&unknown_path).expect("unknown root");
            let unknown_db = unknown_path.join("ledger.db");
            let mut unknown = create_new(&unknown_root, &unknown_db).expect("unknown open");
            unknown.execute(&format!("CREATE TABLE main.{table_name} (
                session_id TEXT PRIMARY KEY, domain_id TEXT NOT NULL,
                seat_id TEXT NOT NULL, purpose TEXT NOT NULL, side_id TEXT
            ) STRICT;
            INSERT INTO main.v37_ledger_session VALUES
                ('unknown-session', 'project-a', 'lead', 'WORK', NULL)"))
                .expect("unknown schema fixture");
            let unknown_ddl = session_schema_state(&unknown).unwrap().unwrap();
            assert!(initialize_schema(&mut unknown).is_err());
            assert_eq!(session_schema_state(&unknown).unwrap(), Some(unknown_ddl));
            assert_eq!(scalar(&unknown, "SELECT COUNT(*) FROM main.v37_ledger_session").unwrap(), "1");
            unknown.close_checked().expect("close unknown");
        }

        let fresh_path = scratch_root();
        let fresh_root = RootLock::acquire(&fresh_path).expect("fresh root");
        let fresh_db = fresh_path.join("ledger.db");
        let mut fresh = create_new(&fresh_root, &fresh_db).expect("fresh open");
        fresh.execute("CREATE TABLE orchestration_events
            (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
             occurred_at TEXT, event_type TEXT, payload_json TEXT)")
            .expect("fresh legacy events");
        initialize_schema(&mut fresh).expect("fresh schema script");
        assert_eq!(session_schema_state(&fresh).unwrap().as_deref(), Some(current_ddl.as_str()));
        initialize_schema(&mut fresh).expect("reopen current schema");
        fresh.close_checked().expect("close fresh");
    }

    #[test]
    fn no_event_terminal_resolution_is_bounded_and_idempotent() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let registration = session(
            "project-no-event",
            "seat-no-event",
            "session-no-event",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        initialize_raw_h_fixture(&mut connection);
        let (mut custodian, _prepared, frame) = raw_process_fixture_with_command(
            &mut connection,
            &registration,
            "operation-no-event",
            "echo frame-one&echo frame-two",
        );
        assert!(!frame.bytes().is_empty());
        assert!(frame.bytes().ends_with(b"\n"));
        capture_raw_source(
            &mut connection,
            &frame,
            "operation-no-event",
            "source-epoch",
            "1",
        )
        .expect("capture first protocol frame");

        assert!(matches!(
            resolve_raw_source_no_event(
                &mut connection,
                "operation-no-event",
                "source-epoch",
                "1",
                "",
            ),
            Err(AtomicError::InvalidRecord("noEventReason"))
        ));
        let oversized_reason = "x".repeat(129);
        assert!(matches!(
            resolve_raw_source_no_event(
                &mut connection,
                "operation-no-event",
                "source-epoch",
                "1",
                &oversized_reason,
            ),
            Err(AtomicError::InvalidRecord("noEventReason"))
        ));
        let resolved = resolve_raw_source_no_event(
            &mut connection,
            "operation-no-event",
            "source-epoch",
            "1",
            "protocol_reply_without_ledger_event",
        )
        .expect("terminal no-event resolution");
        assert_eq!(resolved.reason, "protocol_reply_without_ledger_event");
        assert_eq!(
            resolve_raw_source_no_event(
                &mut connection,
                "operation-no-event",
                "source-epoch",
                "1",
                "protocol_reply_without_ledger_event",
            )
            .expect("idempotent no-event replay"),
            resolved
        );
        assert!(matches!(
            resolve_raw_source_no_event(
                &mut connection,
                "operation-no-event",
                "source-epoch",
                "1",
                "different_reason",
            ),
            Err(AtomicError::OperationConflict)
        ));
        assert!(read_pending_raw_source(
            &connection,
            "operation-no-event",
            "source-epoch",
            "1",
        )
        .expect("terminal row read")
        .is_none());
        let second_frame = custodian
            .read_persistent_child_frame(
                &frame.custody().ticket,
                Duration::from_secs(5),
            )
            .expect("second exact child frame");
        assert!(!second_frame.bytes().is_empty());
        assert!(second_frame.bytes().ends_with(b"\n"));
        capture_raw_source(
            &mut connection,
            &second_frame,
            "operation-no-event",
            "source-epoch",
            "2",
        )
        .expect("capture second protocol frame");
        let normalized = record(
            &mut connection,
            &event_at("normalized-from-frame-two", &registration, Tier::Session, "1"),
        )
        .expect("normalized event uses its own event cursor");
        assert_eq!(normalized.input.source_cursor, "1");
        assert_eq!(
            resolve_raw_source(
                &mut connection,
                "operation-no-event",
                "source-epoch",
                "2",
                &normalized.input.event_id,
            )
            .expect("link second raw frame"),
            RawSourceResolution {
                key: RawSourceKey {
                    operation_id: "operation-no-event".into(),
                    source_epoch: "source-epoch".into(),
                    source_cursor: "2".into(),
                },
                event_id: "normalized-from-frame-two".into(),
            }
        );
        assert!(matches!(
            resolve_raw_source(
                &mut connection,
                "operation-no-event",
                "source-epoch",
                "2",
                "different-event",
            ),
            Err(AtomicError::OperationConflict)
        ));
        assert!(read_pending_raw_source(
            &connection,
            "operation-no-event",
            "source-epoch",
            "2",
        )
        .expect("resolved row read")
        .is_none());
        assert_eq!(
            scalar(
                &connection,
                "SELECT state || ':' || no_event_reason
                 FROM main.v37_ledger_raw_source
                 WHERE source_cursor = '1'",
            )
            .expect("terminal disposition"),
            "NO_EVENT:protocol_reply_without_ledger_event"
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT state || ':' || COALESCE(resolved_event_id, '')
                 FROM main.v37_ledger_raw_source
                 WHERE source_cursor = '2'",
            )
            .expect("resolved disposition"),
            "RESOLVED:normalized-from-frame-two"
        );
        assert_eq!(
            query(
                &connection,
                &Reader {
                    domain_id: registration.domain_id.clone(),
                    seat_id: registration.seat_id.clone(),
                    session_id: registration.session_id.clone(),
                },
                &start,
                10,
            )
            .expect("ordinary query excludes raw bytes")
            .events
            .len(),
            1
        );
        assert_eq!(recover(&connection).expect("ordinary recovery").cursor, start.cursor + 1);
        drop(custodian);
        connection.close_checked().expect("close");
    }

    #[test]
    fn normalized_and_raw_frame_cursors_are_independent() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        let start = initialize_schema(&mut connection).expect("schema");
        let registration = session(
            "project-cursor-separation",
            "seat-cursor-separation",
            "session-cursor-separation",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        initialize_raw_h_fixture(&mut connection);
        let (mut custodian, _prepared, frame_one) = raw_process_fixture_with_command(
            &mut connection,
            &registration,
            "operation-cursor-separation",
            "echo event-one&echo no-event&echo event-two",
        );
        let frame_two = custodian
            .read_persistent_child_frame(
                &frame_one.custody().ticket,
                Duration::from_secs(5),
            )
            .expect("second exact child frame");
        let frame_three = custodian
            .read_persistent_child_frame(
                &frame_one.custody().ticket,
                Duration::from_secs(5),
            )
            .expect("third exact child frame");
        for (frame, ordinal) in [(&frame_one, "1"), (&frame_two, "2"), (&frame_three, "3")] {
            assert!(!frame.bytes().is_empty());
            assert!(frame.bytes().ends_with(b"\n"));
            capture_raw_source(
                &mut connection,
                frame,
                "operation-cursor-separation",
                "source-epoch",
                ordinal,
            )
            .expect("capture exact frame ordinal");
        }
        let first = record(
            &mut connection,
            &event_at("normalized-event-one", &registration, Tier::Session, "1"),
        )
        .expect("first normalized event");
        resolve_raw_source(
            &mut connection,
            "operation-cursor-separation",
            "source-epoch",
            "1",
            &first.input.event_id,
        )
        .expect("link first raw frame");
        resolve_raw_source_no_event(
            &mut connection,
            "operation-cursor-separation",
            "source-epoch",
            "2",
            "protocol_notification_without_ledger_event",
        )
        .expect("terminalize second raw frame");
        let second = record(
            &mut connection,
            &event_at("normalized-event-two", &registration, Tier::Session, "2"),
        )
        .expect("second normalized event after no-event frame");
        resolve_raw_source(
            &mut connection,
            "operation-cursor-separation",
            "source-epoch",
            "3",
            &second.input.event_id,
        )
        .expect("link third raw frame to normalized cursor two");
        assert_eq!(
            scalar(
                &connection,
                "SELECT last_cursor FROM v37_ledger_source_stream
                 WHERE session_id = 'session-cursor-separation'
                   AND source_epoch = 'source-epoch'",
            )
            .expect("normalized stream high-water"),
            "2"
        );
        assert_eq!(recover(&connection).expect("recovery").cursor, start.cursor + 2);
        assert_eq!(
            query(
                &connection,
                &Reader {
                    domain_id: registration.domain_id.clone(),
                    seat_id: registration.seat_id.clone(),
                    session_id: registration.session_id.clone(),
                },
                &start,
                10,
            )
            .expect("ordinary query")
            .events
            .len(),
            2
        );
        drop(custodian);
        connection.close_checked().expect("close");
    }

    #[test]
    fn forged_raw_provenance_rejects_without_mutating_journal() {
        let _guard = route_b_test_guard();
        let path = scratch_root();
        let root = RootLock::acquire(&path).expect("root");
        let db = path.join("ledger.db");
        let mut connection = create_new(&root, &db).expect("open");
        exec(
            &mut connection,
            "CREATE TABLE orchestration_events
             (sequence INTEGER PRIMARY KEY, event_id TEXT UNIQUE, stream_id TEXT,
              occurred_at TEXT, event_type TEXT, payload_json TEXT)",
        )
        .expect("legacy table");
        initialize_schema(&mut connection).expect("schema");
        let registration = session(
            "project-forge",
            "seat-forge",
            "session-forge",
            SessionPurpose::Work,
            None,
        );
        register_session(&mut connection, &registration).expect("register");
        initialize_raw_h_fixture(&mut connection);
        let (mut custodian, _prepared, frame) =
            raw_process_fixture(&mut connection, &registration, "operation-real");
        exec(
            &mut connection,
            "INSERT INTO gogoke_coordination_process_custody
             (operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,
              binary_digest_sha256,profile_id,domain_id,generation,state)
             VALUES ('operation-forged','ticket-forged','nonce-forged','0','0',
                     'fixture','sha256:fixture','profile-forged','project-forge','1','ACTIVE');
             INSERT INTO gogoke_v37_h_owner_binding
             VALUES ('binding-forged','instance-forged','project-forge','SESSION',
                     'session-forged','1','ACTIVE');
             INSERT INTO gogoke_v37_h_claim
             (domain_id,session_id,instance_id,home_id,binding_id,generation,state,
              revision,process_operation_id)
             VALUES ('project-forge','session-forged','instance-forged','home-forged',
                     'binding-forged','1','COMMITTED',1,'operation-forged')",
        )
        .expect("forged durable rows");
        assert!(matches!(
            capture_raw_source(
                &mut connection,
                &frame,
                "operation-forged",
                "source-epoch",
                "1",
            ),
            Err(AtomicError::OperationConflict)
        ));
        assert_eq!(
            scalar(
                &connection,
                "SELECT COUNT(*) FROM main.v37_ledger_raw_source",
            )
            .expect("journal count"),
            "0"
        );
        drop(custodian);
        connection.close_checked().expect("close");
    }
}
