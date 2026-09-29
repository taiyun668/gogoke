//! Native H admission ledger. App-server output is never an authority source.
//! All entrypoints here require the existing verified connection; product v37
//! ingress and a native per-seat caller issuer are still unwired.

use crate::store::atomic::{AtomicError, Statement};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

#[derive(Debug)]
pub(crate) enum AdmissionError {
    Invalid(&'static str),
    Denied,
    Conflict,
    Stale,
    Unknown,
    UnsupportedCapacity,
    Store(AtomicError),
    Sqlite(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
}
impl From<AtomicError> for AdmissionError {
    fn from(error: AtomicError) -> Self {
        Self::Store(error)
    }
}
impl From<SameOpenError> for AdmissionError {
    fn from(error: SameOpenError) -> Self {
        Self::Sqlite(error)
    }
}

const SCHEMA: [(&str, &str); 4] = [
    ("gogoke_v37_h_owner_binding",
     "CREATE TABLE gogoke_v37_h_owner_binding(binding_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,domain_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('SESSION','CALL')),owner_id TEXT NOT NULL,generation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('ACTIVE','REVOKED')),UNIQUE(instance_id,domain_id,kind,owner_id,generation)) STRICT"),
    ("gogoke_v37_h_claim",
     "CREATE TABLE gogoke_v37_h_claim(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,instance_id TEXT NOT NULL,home_id TEXT NOT NULL,binding_id TEXT NOT NULL REFERENCES gogoke_v37_h_owner_binding(binding_id),generation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN','RELEASED')),revision INTEGER NOT NULL CHECK(revision >= 1),process_operation_id TEXT UNIQUE,stop_fact_id TEXT,PRIMARY KEY(domain_id,session_id)) STRICT"),
    ("gogoke_v37_h_operation",
     "CREATE TABLE gogoke_v37_h_operation(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,raw_hex TEXT NOT NULL,operation TEXT NOT NULL,session_id TEXT NOT NULL,status TEXT NOT NULL CHECK(status IN ('APPLIED','UNKNOWN')),previous_revision INTEGER NOT NULL,revision INTEGER NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
    ("gogoke_v37_h_home_fence",
     "CREATE TABLE gogoke_v37_h_home_fence(home_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL,domain_id TEXT NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('SESSION','CALL')),owner_id TEXT NOT NULL,generation TEXT NOT NULL,fence_id TEXT NOT NULL UNIQUE) STRICT"),
];

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
fn require(value: &str, name: &'static str) -> Result<(), AdmissionError> {
    if valid(value) {
        Ok(())
    } else {
        Err(AdmissionError::Invalid(name))
    }
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}
fn count(
    connection: &VerifiedDatabaseConnection<'_>,
    sql: &str,
    params: &[&str],
) -> Result<i64, AdmissionError> {
    let query = Statement::prepare(connection.as_ptr(), sql)?;
    for (index, value) in params.iter().enumerate() {
        query.bind_text((index + 1) as i32, value)?;
    }
    if !query.step_row()? {
        return Err(AdmissionError::Unknown);
    }
    query
        .column_text(0)?
        .parse()
        .map_err(|_| AdmissionError::Unknown)
}
fn changes(connection: &VerifiedDatabaseConnection<'_>) -> Result<i64, AdmissionError> {
    count(connection, "SELECT changes()", &[])
}
fn in_transaction<T>(
    connection: &mut VerifiedDatabaseConnection<'_>,
    action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, AdmissionError>,
) -> Result<T, AdmissionError> {
    connection.execute("BEGIN IMMEDIATE")?;
    match action(connection) {
        Ok(value) => {
            connection
                .execute("COMMIT")
                .map_err(AdmissionError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => {
            connection
                .execute("ROLLBACK")
                .map_err(AdmissionError::RollbackUnknown)?;
            Err(error)
        }
    }
}

/// Exact schema family; partial or changed state is refused, not repaired.
pub(crate) fn initialize_admission_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<(), AdmissionError> {
    reject_shadow_or_effects(connection)?;
    let rows = observed_schema(connection)?;
    let mut expected: Vec<(String, String)> = SCHEMA
        .iter()
        .map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned()))
        .collect();
    expected.sort_by(|left, right| left.0.cmp(&right.0));
    if !rows.is_empty() {
        return if rows == expected {
            Ok(())
        } else {
            Err(AdmissionError::Denied)
        };
    }
    in_transaction(connection, |connection| {
        reject_shadow_or_effects(connection)?;
        if !observed_schema(connection)?.is_empty() {
            return Err(AdmissionError::Denied);
        }
        for (_, sql) in SCHEMA {
            connection.execute(sql)?;
        }
        if observed_schema(connection)? != expected {
            return Err(AdmissionError::Denied);
        }
        Ok(())
    })
}
fn observed_schema(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<Vec<(String, String)>, AdmissionError> {
    let observed = Statement::prepare(connection.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE substr(name,1,13)='gogoke_v37_h_' ORDER BY name")?;
    let mut rows = Vec::new();
    while observed.step_row()? {
        rows.push((observed.column_text(0)?, observed.column_text(1)?));
    }
    Ok(rows)
}
fn reject_shadow_or_effects(
    connection: &VerifiedDatabaseConnection<'_>,
) -> Result<(), AdmissionError> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE substr(name,1,13)='gogoke_v37_h_' OR substr(tbl_name,1,13)='gogoke_v37_h_' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND substr(tbl_name,1,13)='gogoke_v37_h_' LIMIT 1",
    ] {
        if Statement::prepare(connection.as_ptr(),sql)?.step_row()? { return Err(AdmissionError::Denied); }
    }
    Ok(())
}

pub(crate) struct OwnerBinding<'a> {
    pub(crate) binding_id: &'a str,
    pub(crate) instance_id: &'a str,
    pub(crate) domain_id: &'a str,
    pub(crate) kind: &'a str,
    pub(crate) owner_id: &'a str,
    pub(crate) generation: &'a str,
}

/// Called only after a native E/H seat binding is established. It does not
/// infer a caller from an app-server frame or a model-provided ticket.
pub(crate) fn bind_owner_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerBinding<'_>,
) -> Result<String, AdmissionError> {
    for (name, value) in [
        ("binding_id", owner.binding_id),
        ("instance_id", owner.instance_id),
        ("domain_id", owner.domain_id),
        ("owner_id", owner.owner_id),
        ("generation", owner.generation),
    ] {
        require(value, name)?;
    }
    if owner.kind != "SESSION" && owner.kind != "CALL" {
        return Err(AdmissionError::Invalid("kind"));
    }
    if count(
        connection,
        "SELECT COUNT(*) FROM gogoke_v37_instances WHERE instance_id=?1",
        &[owner.instance_id],
    )? != 1
    {
        return Err(AdmissionError::Denied);
    }
    let row = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES(?1,?2,?3,?4,?5,?6,'ACTIVE')")?;
    for (index, value) in [
        owner.binding_id,
        owner.instance_id,
        owner.domain_id,
        owner.kind,
        owner.owner_id,
        owner.generation,
    ]
    .iter()
    .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.step_done()?;
    Ok(owner.binding_id.to_owned())
}

pub(crate) fn verify_home_owner_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
) -> Result<String, AdmissionError> {
    binding_ref(
        connection,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
        true,
    )
}
fn binding_ref(
    connection: &VerifiedDatabaseConnection<'_>,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
    active_only: bool,
) -> Result<String, AdmissionError> {
    let row = Statement::prepare(
        connection.as_ptr(),
        if active_only {
            "SELECT binding_id FROM gogoke_v37_h_owner_binding WHERE instance_id=?1 AND domain_id=?2 AND kind=?3 AND owner_id=?4 AND generation=?5 AND state='ACTIVE'"
        } else {
            "SELECT binding_id FROM gogoke_v37_h_owner_binding WHERE instance_id=?1 AND domain_id=?2 AND kind=?3 AND owner_id=?4 AND generation=?5"
        },
    )?;
    for (index, value) in [instance_id, domain_id, kind, owner_id, generation]
        .iter()
        .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    if !row.step_row()? {
        return Err(AdmissionError::Denied);
    }
    Ok(row.column_text(0)?)
}

/// Revocation makes future home creation and admission fail, without erasing
/// already committed custody or stop facts.
pub(crate) fn revoke_owner_binding_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    binding_id: &str,
) -> Result<(), AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id=?1 AND state='ACTIVE'")?;
    row.bind_text(1, binding_id)?;
    row.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Conflict);
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) struct TrustedLimits {
    pub(crate) project_parallel: i64,
    pub(crate) instance_concurrency: i64,
}
pub(crate) struct AdmissionRequest<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) raw_bytes: &'a [u8],
    pub(crate) instance_id: &'a str,
    pub(crate) home_id: &'a str,
    pub(crate) generation: &'a str,
    pub(crate) expected_revision: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionResult {
    Applied(i64),
    Replayed(i64),
    Unknown,
    Conflict,
    Stale,
}

fn prior(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    operation: &str,
) -> Result<Option<AdmissionResult>, AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT raw_hex,operation,session_id,status,revision FROM gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2")?;
    row.bind_text(1, input.domain_id)?;
    row.bind_text(2, input.request_id)?;
    if !row.step_row()? {
        return Ok(None);
    }
    if row.column_text(0)? != hex(input.raw_bytes)
        || row.column_text(1)? != operation
        || row.column_text(2)? != input.session_id
    {
        return Ok(Some(AdmissionResult::Conflict));
    }
    if row.column_text(3)? == "UNKNOWN" {
        return Ok(Some(AdmissionResult::Unknown));
    }
    let revision = row
        .column_text(4)?
        .parse::<i64>()
        .map_err(|_| AdmissionError::Unknown)?;
    Ok(Some(AdmissionResult::Replayed(revision)))
}
fn journal(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    operation: &str,
    before: i64,
    after: i64,
) -> Result<(), AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,?4,?5,'APPLIED',?6,?7)")?;
    let raw = hex(input.raw_bytes);
    for (index, value) in [
        input.domain_id,
        input.request_id,
        &raw,
        operation,
        input.session_id,
    ]
    .iter()
    .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.bind_i64(6, before)?;
    row.bind_i64(7, after)?;
    row.step_done()?;
    Ok(())
}
fn claim(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
) -> Result<Option<(String, i64, String, String, String)>, AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT state,revision,instance_id,home_id,generation FROM gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2")?;
    row.bind_text(1, input.domain_id)?;
    row.bind_text(2, input.session_id)?;
    if !row.step_row()? {
        return Ok(None);
    }
    Ok(Some((
        row.column_text(0)?,
        row.column_text(1)?
            .parse()
            .map_err(|_| AdmissionError::Unknown)?,
        row.column_text(2)?,
        row.column_text(3)?,
        row.column_text(4)?,
    )))
}

/// The authorization closure must read the current native issuer/E/F facts on
/// this same connection, inside BEGIN IMMEDIATE. No wire capacity or caller bool.
pub(crate) fn reserve_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<TrustedLimits, AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    if input.expected_revision != 0 || input.raw_bytes.is_empty() || input.raw_bytes.len() > 65_536
    {
        return Err(AdmissionError::Invalid("reserve"));
    }
    for (name, value) in [
        ("domain_id", input.domain_id),
        ("session_id", input.session_id),
        ("request_id", input.request_id),
        ("instance_id", input.instance_id),
        ("home_id", input.home_id),
        ("generation", input.generation),
    ] {
        require(value, name)?;
    }
    in_transaction(connection, |connection| {
        let limits = authorize(connection)?;
        if let Some(result) = prior(connection, input, "admission-reserve")? {
            return Ok(result);
        }
        if limits.project_parallel <= 0 || limits.instance_concurrency <= 0 {
            return Err(AdmissionError::UnsupportedCapacity);
        }
        if claim(connection, input)?.is_some() {
            return Ok(AdmissionResult::Conflict);
        }
        let binding = verify_home_owner_in_transaction(
            connection,
            input.instance_id,
            input.domain_id,
            "SESSION",
            input.session_id,
            input.generation,
        )?;
        if count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
            &[input.home_id,input.instance_id,input.domain_id,input.session_id,input.generation])? != 1 {
            return Err(AdmissionError::Denied);
        }
        if count(
            connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_home_fence WHERE home_id=?1",
            &[input.home_id],
        )? != 0
        {
            return Err(AdmissionError::Denied);
        }
        let project_active = count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE domain_id=?1 AND state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')",
            &[input.domain_id])?;
        let instance_active = count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE instance_id=?1 AND state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')",
            &[input.instance_id])?;
        if project_active >= limits.project_parallel
            || instance_active >= limits.instance_concurrency
        {
            return Err(AdmissionError::Denied);
        }
        let row = Statement::prepare(connection.as_ptr(),
            "INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES(?1,?2,?3,?4,?5,?6,'RESERVED',1)")?;
        for (index, value) in [
            input.domain_id,
            input.session_id,
            input.instance_id,
            input.home_id,
            &binding,
            input.generation,
        ]
        .iter()
        .enumerate()
        {
            row.bind_text((index + 1) as i32, value)?;
        }
        row.step_done()?;
        journal(connection, input, "admission-reserve", 0, 1)?;
        Ok(AdmissionResult::Applied(1))
    })
}

fn transition(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    operation: &str,
    from: &str,
    to: &str,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    in_transaction(connection, |connection| {
        authorize(connection)?;
        if let Some(result) = prior(connection, input, operation)? {
            return Ok(result);
        }
        let Some((state, revision, instance_id, home_id, generation)) = claim(connection, input)?
        else {
            return Ok(AdmissionResult::Conflict);
        };
        if instance_id != input.instance_id
            || home_id != input.home_id
            || generation != input.generation
        {
            return Ok(AdmissionResult::Conflict);
        }
        if count(connection,
            "SELECT COUNT(*) FROM gogoke_v37_h_claim AS a JOIN gogoke_v37_h_owner_binding AS b ON b.binding_id=a.binding_id WHERE a.domain_id=?1 AND a.session_id=?2 AND b.state='ACTIVE'",
            &[input.domain_id,input.session_id])? != 1 { return Err(AdmissionError::Denied); }
        if input.expected_revision != revision {
            return Ok(AdmissionResult::Stale);
        }
        if state != from {
            return Ok(AdmissionResult::Conflict);
        }
        let row = Statement::prepare(connection.as_ptr(),
            "UPDATE gogoke_v37_h_claim SET state=?1,revision=revision+1 WHERE domain_id=?2 AND session_id=?3 AND state=?4 AND revision=?5")?;
        for (index, value) in [to, input.domain_id, input.session_id, from]
            .iter()
            .enumerate()
        {
            row.bind_text((index + 1) as i32, value)?;
        }
        row.bind_i64(5, revision)?;
        row.step_done()?;
        if changes(connection)? != 1 {
            return Err(AdmissionError::Conflict);
        }
        journal(connection, input, operation, revision, revision + 1)?;
        Ok(AdmissionResult::Applied(revision + 1))
    })
}
pub(crate) fn commit_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    transition(
        connection,
        input,
        "admission-commit",
        "RESERVED",
        "COMMITTED",
        authorize,
    )
}
pub(crate) fn release_admission(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    authorize: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<(), AdmissionError>,
) -> Result<AdmissionResult, AdmissionError> {
    in_transaction(connection, |connection| {
        authorize(connection)?;
        if let Some(result) = prior(connection, input, "admission-release")? {
            return Ok(result);
        }
        let Some((state, revision, instance_id, home_id, generation)) = claim(connection, input)?
        else {
            return Ok(AdmissionResult::Conflict);
        };
        if instance_id != input.instance_id
            || home_id != input.home_id
            || generation != input.generation
        {
            return Ok(AdmissionResult::Conflict);
        }
        if input.expected_revision != revision {
            return Ok(AdmissionResult::Stale);
        }
        if state != "RESERVED" && state != "STOPPED" {
            return Ok(AdmissionResult::Conflict);
        }
        let row = Statement::prepare(connection.as_ptr(),
            "UPDATE gogoke_v37_h_claim SET state='RELEASED',revision=revision+1 WHERE domain_id=?1 AND session_id=?2 AND state=?3 AND revision=?4")?;
        row.bind_text(1, input.domain_id)?;
        row.bind_text(2, input.session_id)?;
        row.bind_text(3, &state)?;
        row.bind_i64(4, revision)?;
        row.step_done()?;
        if changes(connection)? != 1 {
            return Err(AdmissionError::Conflict);
        }
        journal(
            connection,
            input,
            "admission-release",
            revision,
            revision + 1,
        )?;
        Ok(AdmissionResult::Applied(revision + 1))
    })
}

/// The native launcher binds its already-persisted custody operation before
/// the process is exposed. No wire field can create this relationship.
pub(crate) fn bind_process_operation_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    process_operation_id: &str,
) -> Result<(), AdmissionError> {
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_coordination_process_custody AS c JOIN gogoke_v37_h_claim AS a ON a.domain_id=c.domain_id AND a.generation=c.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND c.operation_id=?3 AND c.state IN ('PREPARED','ACTIVE') AND a.state='COMMITTED' AND a.process_operation_id IS NULL",
        &[domain_id,session_id,process_operation_id])? != 1 { return Err(AdmissionError::Denied); }
    let row = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_claim SET process_operation_id=?1 WHERE domain_id=?2 AND session_id=?3 AND state='COMMITTED' AND process_operation_id IS NULL")?;
    row.bind_text(1, process_operation_id)?;
    row.bind_text(2, domain_id)?;
    row.bind_text(3, session_id)?;
    row.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Conflict);
    }
    Ok(())
}

/// Native launch uncertainty is durable under the original request ID. A
/// second request ID cannot start this claim while its outcome is unresolved.
pub(crate) fn mark_start_unknown_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AdmissionRequest<'_>,
    process_operation_id: &str,
) -> Result<(), AdmissionError> {
    let next_revision = input
        .expected_revision
        .checked_add(1)
        .ok_or(AdmissionError::Invalid("revision overflow"))?;
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_v37_h_claim AS a JOIN gogoke_coordination_process_custody AS c ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id AND c.generation=a.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND a.instance_id=?3 AND a.home_id=?4 AND a.generation=?5 AND a.process_operation_id=?6 AND a.state='COMMITTED' AND c.state='UNKNOWN'",
        &[input.domain_id,input.session_id,input.instance_id,input.home_id,input.generation,
          process_operation_id])? != 1 { return Err(AdmissionError::Denied); }
    let row = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_claim SET state='UNKNOWN',revision=revision+1 WHERE domain_id=?1 AND session_id=?2 AND state='COMMITTED' AND revision=?3")?;
    row.bind_text(1, input.domain_id)?;
    row.bind_text(2, input.session_id)?;
    row.bind_i64(3, input.expected_revision)?;
    row.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Stale);
    }
    let operation = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,'open',?4,'UNKNOWN',?5,?6)")?;
    let raw = hex(input.raw_bytes);
    operation.bind_text(1, input.domain_id)?;
    operation.bind_text(2, input.request_id)?;
    operation.bind_text(3, &raw)?;
    operation.bind_text(4, input.session_id)?;
    operation.bind_i64(5, input.expected_revision)?;
    operation.bind_i64(6, next_revision)?;
    operation.step_done()?;
    Ok(())
}

/// Accepts only an already persisted native custody STOPPED fact. This cannot
/// be made true by a wire `stopped` field or a Node-side callback.
pub(crate) fn record_session_stop_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    process_operation_id: &str,
) -> Result<String, AdmissionError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT c.stop_proof_hash FROM gogoke_coordination_process_custody AS c JOIN gogoke_v37_h_claim AS a ON a.process_operation_id=c.operation_id AND a.domain_id=c.domain_id AND a.generation=c.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND c.operation_id=?3 AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL AND a.state IN ('COMMITTED','UNKNOWN')")?;
    row.bind_text(1, domain_id)?;
    row.bind_text(2, session_id)?;
    row.bind_text(3, process_operation_id)?;
    if !row.step_row()? {
        return Err(AdmissionError::Denied);
    }
    let proof = row.column_text(0)?;
    drop(row);
    let update = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_claim SET state='STOPPED',revision=revision+1,stop_fact_id=?1 WHERE domain_id=?2 AND session_id=?3 AND state IN ('COMMITTED','UNKNOWN') AND process_operation_id=?4")?;
    update.bind_text(1, &proof)?;
    update.bind_text(2, domain_id)?;
    update.bind_text(3, session_id)?;
    update.bind_text(4, process_operation_id)?;
    update.step_done()?;
    if changes(connection)? != 1 {
        return Err(AdmissionError::Conflict);
    }
    let resolve = Statement::prepare(connection.as_ptr(),
        "UPDATE gogoke_v37_h_operation SET status='APPLIED',revision=(SELECT revision FROM gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2) WHERE domain_id=?1 AND session_id=?2 AND operation='open' AND status='UNKNOWN'")?;
    resolve.bind_text(1, domain_id)?;
    resolve.bind_text(2, session_id)?;
    resolve.step_done()?;
    Ok(proof)
}

pub(crate) fn verify_home_stop_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
) -> Result<String, AdmissionError> {
    binding_ref(
        connection,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
        false,
    )?;
    if kind == "CALL" {
        return Err(AdmissionError::Unknown);
    }
    if kind != "SESSION" {
        return Err(AdmissionError::Invalid("kind"));
    }
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT a.stop_fact_id FROM gogoke_v37_h_claim AS a JOIN gogoke_coordination_process_custody AS c ON c.operation_id=a.process_operation_id WHERE a.instance_id=?1 AND a.domain_id=?2 AND a.session_id=?3 AND a.generation=?4 AND a.state IN ('STOPPED','RELEASED') AND c.state='STOPPED' AND c.stop_proof_hash=a.stop_fact_id AND a.stop_fact_id IS NOT NULL")?;
    for (index, value) in [instance_id, domain_id, owner_id, generation]
        .iter()
        .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    if !row.step_row()? {
        return Err(AdmissionError::Denied);
    }
    Ok(row.column_text(0)?)
}

/// Called by F inside its cleanup transaction. The inserted fence survives
/// restart and makes every later reserve for this exact home fail closed.
pub(crate) fn fence_home_admission_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>,
    home_id: &str,
    instance_id: &str,
    domain_id: &str,
    kind: &str,
    owner_id: &str,
    generation: &str,
) -> Result<String, AdmissionError> {
    verify_home_stop_in_transaction(
        connection,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
    )?;
    let existing = Statement::prepare(connection.as_ptr(),
        "SELECT instance_id,domain_id,kind,owner_id,generation,fence_id FROM gogoke_v37_h_home_fence WHERE home_id=?1")?;
    existing.bind_text(1, home_id)?;
    if existing.step_row()? {
        let matches = [instance_id, domain_id, kind, owner_id, generation]
            .iter()
            .enumerate()
            .try_fold(
                true,
                |same, (index, value)| -> Result<bool, AdmissionError> {
                    Ok(same && existing.column_text(index as i32)? == *value)
                },
            )?;
        return if matches {
            Ok(existing.column_text(5)?)
        } else {
            Err(AdmissionError::Conflict)
        };
    }
    drop(existing);
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind=?4 AND owner_id=?5 AND generation=?6 AND state IN ('CLOSED','CLEANUP_UNKNOWN')",
        &[home_id,instance_id,domain_id,kind,owner_id,generation])? != 1 {
        return Err(AdmissionError::Denied);
    }
    if count(connection,
        "SELECT COUNT(*) FROM gogoke_v37_h_claim WHERE home_id=?1 AND state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')",
        &[home_id])? != 0 { return Err(AdmissionError::Denied); }
    let fence_id = format!("h-fence-{home_id}");
    let row = Statement::prepare(connection.as_ptr(),
        "INSERT INTO gogoke_v37_h_home_fence(home_id,instance_id,domain_id,kind,owner_id,generation,fence_id) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
    for (index, value) in [
        home_id,
        instance_id,
        domain_id,
        kind,
        owner_id,
        generation,
        &fence_id,
    ]
    .iter()
    .enumerate()
    {
        row.bind_text((index + 1) as i32, value)?;
    }
    row.step_done()?;
    Ok(fence_id)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn one_boundary_survives_reopen_unknown_replay_and_home_fence() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder =
            std::env::temp_dir().join(format!("gogoke-v37-h-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        db.execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY) STRICT")
            .unwrap();
        db.execute("CREATE TABLE gogoke_v37_instance_homes(home_id TEXT PRIMARY KEY,instance_id TEXT,domain_id TEXT,kind TEXT,owner_id TEXT,generation TEXT,state TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT PRIMARY KEY,domain_id TEXT,generation TEXT,state TEXT,stop_proof_hash TEXT) STRICT").unwrap();
        db.execute("INSERT INTO gogoke_v37_instances VALUES('instanceA')")
            .unwrap();
        db.execute("INSERT INTO gogoke_v37_instance_homes VALUES('homeA','instanceA','projectA','SESSION','sessionA','1','ACTIVE'),('homeB','instanceA','projectA','SESSION','sessionB','1','ACTIVE')").unwrap();
        initialize_admission_schema(&mut db).unwrap();
        in_transaction(&mut db, |connection| {
            for (binding_id, session_id) in [("bindingA", "sessionA"), ("bindingB", "sessionB")] {
                bind_owner_in_transaction(
                    connection,
                    &OwnerBinding {
                        binding_id,
                        instance_id: "instanceA",
                        domain_id: "projectA",
                        kind: "SESSION",
                        owner_id: session_id,
                        generation: "1",
                    },
                )?;
            }
            Ok(())
        })
        .unwrap();
        let first = AdmissionRequest {
            domain_id: "projectA",
            session_id: "sessionA",
            request_id: "reserveA",
            raw_bytes: b"reserve bytes A",
            instance_id: "instanceA",
            home_id: "homeA",
            generation: "1",
            expected_revision: 0,
        };
        let second = AdmissionRequest {
            domain_id: "projectA",
            session_id: "sessionB",
            request_id: "reserveB",
            raw_bytes: b"reserve bytes B",
            instance_id: "instanceA",
            home_id: "homeB",
            generation: "1",
            expected_revision: 0,
        };
        let limits = |_| {
            Ok(TrustedLimits {
                project_parallel: 1,
                instance_concurrency: 1,
            })
        };
        assert_eq!(
            reserve_admission(&mut db, &first, limits).unwrap(),
            AdmissionResult::Applied(1)
        );
        assert_eq!(
            reserve_admission(&mut db, &first, limits).unwrap(),
            AdmissionResult::Replayed(1)
        );
        let changed = AdmissionRequest {
            raw_bytes: b"changed bytes",
            ..first
        };
        assert_eq!(
            reserve_admission(&mut db, &changed, limits).unwrap(),
            AdmissionResult::Conflict
        );
        assert!(matches!(
            reserve_admission(&mut db, &second, limits),
            Err(AdmissionError::Denied)
        ));
        let commit = AdmissionRequest {
            request_id: "commitA",
            raw_bytes: b"commit bytes",
            expected_revision: 1,
            ..first
        };
        assert_eq!(
            commit_admission(&mut db, &commit, |_| Ok(())).unwrap(),
            AdmissionResult::Applied(2)
        );
        db.execute("INSERT INTO gogoke_coordination_process_custody VALUES('processA','projectA','1','PREPARED',NULL)").unwrap();
        in_transaction(&mut db, |connection| {
            bind_process_operation_in_transaction(connection, "projectA", "sessionA", "processA")
        })
        .unwrap();
        db.execute("UPDATE gogoke_coordination_process_custody SET state='UNKNOWN' WHERE operation_id='processA'").unwrap();
        let opened = AdmissionRequest {
            request_id: "openA",
            raw_bytes: b"open bytes",
            expected_revision: 2,
            ..first
        };
        in_transaction(&mut db, |connection| {
            mark_start_unknown_in_transaction(connection, &opened, "processA")
        })
        .unwrap();
        db.close_checked().unwrap();
        let mut db = open_existing(&root, &path).unwrap();
        initialize_admission_schema(&mut db).unwrap();
        assert_eq!(
            prior(&db, &opened, "open").unwrap(),
            Some(AdmissionResult::Unknown)
        );
        assert!(matches!(
            reserve_admission(&mut db, &second, limits),
            Err(AdmissionError::Denied)
        ));
        db.execute("UPDATE gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='proofA' WHERE operation_id='processA'").unwrap();
        in_transaction(&mut db, |connection| {
            assert_eq!(
                record_session_stop_in_transaction(connection, "projectA", "sessionA", "processA")?,
                "proofA"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            prior(&db, &opened, "open").unwrap(),
            Some(AdmissionResult::Replayed(4))
        );
        let release = AdmissionRequest {
            request_id: "releaseA",
            raw_bytes: b"release bytes",
            expected_revision: 4,
            ..first
        };
        assert_eq!(
            release_admission(&mut db, &release, |_| Ok(())).unwrap(),
            AdmissionResult::Applied(5)
        );
        db.execute("UPDATE gogoke_v37_instance_homes SET state='CLOSED' WHERE home_id='homeA'")
            .unwrap();
        in_transaction(&mut db, |connection| {
            assert_eq!(
                fence_home_admission_in_transaction(
                    connection,
                    "homeA",
                    "instanceA",
                    "projectA",
                    "SESSION",
                    "sessionA",
                    "1"
                )?,
                "h-fence-homeA"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            reserve_admission(&mut db, &second, limits).unwrap(),
            AdmissionResult::Applied(1)
        );
        in_transaction(&mut db, |connection| {
            revoke_owner_binding_in_transaction(connection, "bindingB")
        })
        .unwrap();
        assert!(matches!(
            verify_home_owner_in_transaction(
                &mut db,
                "instanceA",
                "projectA",
                "SESSION",
                "sessionB",
                "1"
            ),
            Err(AdmissionError::Denied)
        ));
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&folder).unwrap();
    }
}
