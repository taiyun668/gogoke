//! E.1 seat identity, template copy, and instance binding on the product database connection.
//! This module receives only native capabilities. It is not an IPC dispatcher;
//! the future ingress must authenticate a live lead session before admission.

use super::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use super::authority::{check_owner_in_current_transaction, OwnerIssuer};
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};

#[cfg(all(test, windows))]
mod tests;

const LEGACY_SEATS: &str = "CREATE TABLE gogoke_v37_seats(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT NOT NULL REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const SEATS: &str = "CREATE TABLE gogoke_v37_seats(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const OPERATIONS: &str = "CREATE TABLE gogoke_v37_seat_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,layer TEXT NOT NULL,parent_seat_id TEXT,kind TEXT NOT NULL,instance_id TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,generation INTEGER NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT";
const TEMPLATES: &str = "CREATE TABLE gogoke_v37_seat_templates(domain_id TEXT NOT NULL,template_id TEXT NOT NULL,settings_json TEXT NOT NULL CHECK(length(settings_json) > 0),revision INTEGER NOT NULL CHECK(revision >= 1),PRIMARY KEY(domain_id,template_id)) STRICT";
const SETTINGS: &str = "CREATE TABLE gogoke_v37_seat_settings(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,template_id TEXT NOT NULL,settings_json TEXT NOT NULL CHECK(length(settings_json) > 0),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const OPERATION_SNAPSHOTS: &str = "CREATE TABLE gogoke_v37_seat_operation_snapshots(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,template_id TEXT NOT NULL,settings_json TEXT NOT NULL CHECK(length(settings_json) > 0),PRIMARY KEY(domain_id,request_id),FOREIGN KEY(domain_id,request_id) REFERENCES gogoke_v37_seat_operations(domain_id,request_id)) STRICT";
const PROJECT_CAPS: &str = "CREATE TABLE gogoke_v37_seat_project_caps(domain_id TEXT PRIMARY KEY,parallel_cap INTEGER NOT NULL CHECK(parallel_cap > 0)) STRICT";

#[derive(Debug)]
pub(crate) enum SeatError {
    Invalid(&'static str),
    Denied,
    Conflict,
    Busy,
    Unknown,
    SchemaDrift,
    Store(AtomicError),
    Open(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
}
impl From<AtomicError> for SeatError {
    fn from(error: AtomicError) -> Self {
        Self::Store(error)
    }
}
impl From<SameOpenError> for SeatError {
    fn from(error: SameOpenError) -> Self {
        Self::Open(error)
    }
}
impl From<SeatError> for super::orchestration::OrchestrationError {
    fn from(error: SeatError) -> Self {
        use super::orchestration::OrchestrationError;
        match error {
            SeatError::Invalid(field) => OrchestrationError::Invalid(field),
            SeatError::Denied => OrchestrationError::AccessDenied,
            SeatError::Conflict => OrchestrationError::OperationConflict,
            SeatError::Store(source) => OrchestrationError::Atomic(source),
            SeatError::CommitUnknown(source) => OrchestrationError::CommitUnknownWithCause(source),
            // The remaining variants lack an exact typed destination. Retain
            // the full native error and cause rather than collapsing them.
            other => OrchestrationError::V37StoreFailure(format!("seat: {other:?}")),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Seat {
    pub(crate) domain_id: String,
    pub(crate) seat_id: String,
    pub(crate) incarnation: String,
    pub(crate) layer: Layer,
    pub(crate) parent_seat_id: Option<String>,
    pub(crate) kind: Kind,
    /// Empty is the native view of a SQL NULL until bind_instance succeeds.
    pub(crate) instance_id: String,
    pub(crate) template_id: Option<String>,
    pub(crate) settings_json: Option<String>,
    pub(crate) state: State,
    pub(crate) generation: i64,
    pub(crate) revision: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Layer {
    User,
    Lead,
}
impl Layer {
    fn sql(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Lead => "LEAD",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Long,
    Short,
}
impl Kind {
    fn sql(self) -> &'static str {
        match self {
            Self::Long => "LONG",
            Self::Short => "SHORT",
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum State {
    Idle,
    Busy,
    Reclaimed,
}

/// A native lead admission. No string or wire token constructor is exposed.
/// H must call the constructor only after authenticating the live session and
/// matching its seat, domain and generation to its admission reservation.
pub(crate) struct NativeLeadAdmission {
    domain_id: String,
    seat_id: String,
    incarnation: String,
    generation: i64,
}
impl NativeLeadAdmission {
    pub(crate) fn from_native_runtime_snapshot(seat: &Seat) -> Result<Self, SeatError> {
        if seat.layer != Layer::User || seat.state != State::Busy {
            return Err(SeatError::Denied);
        }
        Ok(Self {
            domain_id: seat.domain_id.clone(),
            seat_id: seat.seat_id.clone(),
            incarnation: seat.incarnation.clone(),
            generation: seat.generation,
        })
    }
}

pub(crate) enum NativeOrigin<'a> {
    User(&'a OwnerIssuer),
    Lead(&'a NativeLeadAdmission),
}
impl<'a> NativeOrigin<'a> {
    pub(crate) fn user(issuer: &'a OwnerIssuer) -> Self {
        Self::User(issuer)
    }
    pub(crate) fn lead(admission: &'a NativeLeadAdmission) -> Self {
        Self::Lead(admission)
    }
}

pub(crate) struct CreateSeat<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) seat_id: &'a str,
    pub(crate) template_id: &'a str,
    /// A new seat can remain unbound until the native caller chooses an
    /// instance. Bound creation is retained for existing callers.
    pub(crate) instance_id: Option<&'a str>,
    pub(crate) kind: Kind,
    pub(crate) request_id: &'a str,
    /// Exact ingress bytes, including unknown fields and original whitespace.
    pub(crate) request_bytes: &'a [u8],
}
pub(crate) struct StoreTemplate<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) template_id: &'a str,
    /// Canonical JSON object copied into every seat created from this template.
    pub(crate) settings_json: &'a [u8],
}
pub(crate) struct SeatChange<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) seat_id: &'a str,
    pub(crate) expected_generation: i64,
    pub(crate) expected_revision: i64,
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SeatReceipt {
    pub(crate) seat: Seat,
    pub(crate) replayed: bool,
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'))
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
fn validate(domain: &str, seat: &str, request: &str, raw: &[u8]) -> Result<(), SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if !valid_id(seat) {
        return Err(SeatError::Invalid("seat_id"));
    }
    if !valid_id(request) {
        return Err(SeatError::Invalid("request_id"));
    }
    if raw.is_empty() || raw.len() > crate::ipc::MAX_FRAME_BYTES {
        return Err(SeatError::Invalid("request_bytes"));
    }
    Ok(())
}
fn fingerprint(parts: &[&str], raw: &[u8]) -> String {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    bytes.extend_from_slice(&(raw.len() as u64).to_be_bytes());
    bytes.extend_from_slice(raw);
    super::digest::content_hash(&bytes)
}
fn transact<T>(
    db: &mut VerifiedDatabaseConnection<'_>,
    f: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, SeatError>,
) -> Result<T, SeatError> {
    db.execute("BEGIN IMMEDIATE")?;
    match f(db) {
        Ok(value) => {
            db.execute("COMMIT").map_err(SeatError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => {
            db.execute("ROLLBACK").map_err(SeatError::RollbackUnknown)?;
            Err(error)
        }
    }
}
fn check_current_owner(
    db: &VerifiedDatabaseConnection<'_>,
    issuer: &OwnerIssuer,
) -> Result<(), SeatError> {
    check_owner_in_current_transaction(db, issuer).map_err(|error| match error {
        super::orchestration::OrchestrationError::Atomic(source) => SeatError::Store(source),
        _ => SeatError::Denied,
    })
}
fn schema(db: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT name,sql FROM main.sqlite_schema WHERE lower(name) LIKE 'gogoke_v37_seat%' ORDER BY name")?;
    let mut rows = Vec::new();
    while q.step_row()? {
        rows.push((q.column_text(0)?, q.column_text(1)?));
    }
    Ok(rows)
}
fn reject_shadow_or_effect(db: &VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name) LIKE 'gogoke_v37_seat%' OR lower(tbl_name) LIKE 'gogoke_v37_seat%' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name) LIKE 'gogoke_v37_seat%' LIMIT 1",
    ] {
        if Statement::prepare(db.as_ptr(), sql)?.step_row()? { return Err(SeatError::SchemaDrift); }
    }
    Ok(())
}
fn expected_schema() -> Vec<(String, String)> {
    let mut entries = previous_schema();
    entries.push(("gogoke_v37_seat_project_caps".into(), PROJECT_CAPS.into()));
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn previous_schema() -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = vec![
        (
            "gogoke_v37_seat_operation_snapshots".into(),
            OPERATION_SNAPSHOTS.into(),
        ),
        ("gogoke_v37_seat_operations".into(), OPERATIONS.into()),
        ("gogoke_v37_seat_settings".into(), SETTINGS.into()),
        ("gogoke_v37_seat_templates".into(), TEMPLATES.into()),
        ("gogoke_v37_seats".into(), SEATS.into()),
    ];
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
fn legacy_schema() -> Vec<(String, String)> {
    let mut entries: Vec<(String, String)> = vec![
        ("gogoke_v37_seat_operations".into(), OPERATIONS.into()),
        ("gogoke_v37_seats".into(), LEGACY_SEATS.into()),
    ];
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    entries
}
/// Call after F's instance schema, on the same verified product connection.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    reject_shadow_or_effect(db)?;
    let observed = schema(db)?;
    if observed == expected_schema() {
        return Ok(());
    }
    if observed == legacy_schema() {
        return migrate_legacy_schema(db);
    }
    if observed == previous_schema() {
        return migrate_previous_schema(db);
    }
    if !observed.is_empty() {
        return Err(SeatError::SchemaDrift);
    }
    transact(db, |db| {
        reject_shadow_or_effect(db)?;
        if !schema(db)?.is_empty() {
            return Err(SeatError::SchemaDrift);
        }
        db.execute(SEATS)?;
        db.execute(OPERATIONS)?;
        db.execute(TEMPLATES)?;
        db.execute(SETTINGS)?;
        db.execute(OPERATION_SNAPSHOTS)?;
        db.execute(PROJECT_CAPS)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn migrate_previous_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    transact(db, |db| {
        reject_shadow_or_effect(db)?;
        if schema(db)? != previous_schema() {
            return Err(SeatError::SchemaDrift);
        }
        db.execute(PROJECT_CAPS)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn migrate_legacy_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    transact(db, |db| {
        if schema(db)? != legacy_schema() {
            return Err(SeatError::SchemaDrift);
        }
        // SQLite cannot make a NOT NULL column nullable in place. Rebuild only
        // this table, preserving every existing row and the self-layer FK.
        db.execute("CREATE TABLE gogoke_v37_seats_v2(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats_v2(domain_id,seat_id)) STRICT")?;
        db.execute("INSERT INTO gogoke_v37_seats_v2(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) SELECT domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision FROM gogoke_v37_seats")?;
        db.execute("DROP TABLE gogoke_v37_seats")?;
        // CREATE the final table from the canonical SQL instead of renaming
        // the temporary one: SQLite records renamed identifiers with quotes,
        // which would make the exact-schema pin depend on migration history.
        db.execute(SEATS)?;
        db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) SELECT domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision FROM gogoke_v37_seats_v2")?;
        db.execute("DROP TABLE gogoke_v37_seats_v2")?;
        db.execute(TEMPLATES)?;
        db.execute(SETTINGS)?;
        db.execute(OPERATION_SNAPSHOTS)?;
        db.execute(PROJECT_CAPS)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn validate_template_settings(raw: &[u8]) -> Result<(), SeatError> {
    if raw.is_empty() || raw.len() > crate::ipc::MAX_FRAME_BYTES {
        return Err(SeatError::Invalid("template_settings"));
    }
    super::atomic::require_canonical_json(raw, "template.settings")?;
    let text = std::str::from_utf8(raw).map_err(|_| SeatError::Invalid("template_settings"))?;
    if !matches!(Parser::parse(text)?, Json::Object(_)) {
        return Err(SeatError::Invalid("template_settings"));
    }
    Ok(())
}

fn seat_template_fields(
    template_id: String,
    settings_json: String,
) -> Result<(Option<String>, Option<String>), SeatError> {
    if template_id.is_empty() != settings_json.is_empty() {
        return Err(SeatError::SchemaDrift);
    }
    if template_id.is_empty() {
        return Ok((None, None));
    }
    if !valid_id(&template_id) {
        return Err(SeatError::SchemaDrift);
    }
    validate_template_settings(settings_json.as_bytes()).map_err(|_| SeatError::SchemaDrift)?;
    Ok((Some(template_id), Some(settings_json)))
}

fn template(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    template_id: &str,
) -> Result<Option<String>, SeatError> {
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT settings_json FROM main.gogoke_v37_seat_templates WHERE domain_id=?1 AND template_id=?2",
    )?;
    q.bind_text(1, domain)?;
    q.bind_text(2, template_id)?;
    if !q.step_row()? {
        return Ok(None);
    }
    let settings = q.column_text(0)?;
    if q.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    validate_template_settings(settings.as_bytes()).map_err(|_| SeatError::SchemaDrift)?;
    Ok(Some(settings))
}

fn read(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    seat_id: &str,
) -> Result<Option<Seat>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT s.incarnation,s.layer,COALESCE(s.parent_seat_id,''),s.kind,COALESCE(s.instance_id,''),s.state,s.generation,s.revision,COALESCE(t.template_id,''),COALESCE(t.settings_json,'') FROM main.gogoke_v37_seats AS s LEFT JOIN main.gogoke_v37_seat_settings AS t ON t.domain_id=s.domain_id AND t.seat_id=s.seat_id WHERE s.domain_id=?1 AND s.seat_id=?2")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, seat_id)?;
    if !q.step_row()? {
        return Ok(None);
    }
    let layer = match q.column_text(1)?.as_str() {
        "USER" => Layer::User,
        "LEAD" => Layer::Lead,
        _ => return Err(SeatError::SchemaDrift),
    };
    let parent = q.column_text(2)?;
    let kind = match q.column_text(3)?.as_str() {
        "LONG" => Kind::Long,
        "SHORT" => Kind::Short,
        _ => return Err(SeatError::SchemaDrift),
    };
    let state = match q.column_text(5)?.as_str() {
        "IDLE" => State::Idle,
        "BUSY" => State::Busy,
        "RECLAIMED" => State::Reclaimed,
        _ => return Err(SeatError::SchemaDrift),
    };
    let generation = q
        .column_text(6)?
        .parse()
        .map_err(|_| SeatError::SchemaDrift)?;
    let revision = q
        .column_text(7)?
        .parse()
        .map_err(|_| SeatError::SchemaDrift)?;
    let (template_id, settings_json) = seat_template_fields(q.column_text(8)?, q.column_text(9)?)?;
    let seat = Seat {
        domain_id: domain.into(),
        seat_id: seat_id.into(),
        incarnation: q.column_text(0)?,
        layer,
        parent_seat_id: if parent.is_empty() {
            None
        } else {
            Some(parent)
        },
        kind,
        instance_id: q.column_text(4)?,
        template_id,
        settings_json,
        state,
        generation,
        revision,
    };
    if q.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    Ok(Some(seat))
}
pub(crate) fn get(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    seat_id: &str,
) -> Result<Option<Seat>, SeatError> {
    if !valid_id(domain) || !valid_id(seat_id) {
        return Err(SeatError::Invalid("seat address"));
    }
    read(db, domain, seat_id)
}

/// Owner's project-wide admission limit. The native issuer is checked inside
/// the same write transaction as the update; lead-layer callers have no write
/// capability. No value is installed by schema creation or migration.
pub(crate) fn set_project_parallel_cap(
    db: &mut VerifiedDatabaseConnection<'_>,
    issuer: &OwnerIssuer,
    domain: &str,
    cap: i64,
) -> Result<(), SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if cap <= 0 {
        return Err(SeatError::Invalid("project_parallel_cap"));
    }
    transact(db, |db| {
        check_current_owner(db, issuer)?;
        let write = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_project_caps(domain_id,parallel_cap) VALUES(?1,?2) ON CONFLICT(domain_id) DO UPDATE SET parallel_cap=excluded.parallel_cap",
        )?;
        write.bind_text(1, domain)?;
        write.bind_i64(2, cap)?;
        write.step_done()?;
        Ok(())
    })
}

/// H reads this required value inside its BEGIN IMMEDIATE admission transaction
/// on the same verified connection. A missing project cap denies admission.
pub(crate) fn read_project_parallel_cap(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
) -> Result<i64, SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    let query = Statement::prepare(
        db.as_ptr(),
        "SELECT parallel_cap FROM main.gogoke_v37_seat_project_caps WHERE domain_id=?1",
    )?;
    query.bind_text(1, domain)?;
    if !query.step_row()? {
        return Err(SeatError::Denied);
    }
    let cap = query.column_text(0)?.parse::<i64>().map_err(|_| SeatError::SchemaDrift)?;
    if cap <= 0 || query.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    Ok(cap)
}

/// Store an immutable template definition. Only the owner layer can publish
/// templates; lead seats may copy them but cannot change the source definition.
pub(crate) fn store_template(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: StoreTemplate<'_>,
) -> Result<(), SeatError> {
    let issuer = match origin {
        NativeOrigin::User(issuer) => issuer,
        NativeOrigin::Lead(_) => return Err(SeatError::Denied),
    };
    if !valid_id(input.domain_id) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if !valid_id(input.template_id) {
        return Err(SeatError::Invalid("template_id"));
    }
    validate_template_settings(input.settings_json)?;
    transact(db, |db| {
        check_current_owner(db, issuer)?;
        let existing = Statement::prepare(
            db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_templates WHERE domain_id=?1 AND template_id=?2",
        )?;
        existing.bind_text(1, input.domain_id)?;
        existing.bind_text(2, input.template_id)?;
        if existing.step_row()? {
            return Err(SeatError::Conflict);
        }
        let insert = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_templates(domain_id,template_id,settings_json,revision) VALUES(?1,?2,?3,1)",
        )?;
        insert.bind_text(1, input.domain_id)?;
        insert.bind_text(2, input.template_id)?;
        let settings = std::str::from_utf8(input.settings_json)
            .map_err(|_| SeatError::Invalid("template_settings"))?;
        insert.bind_text(3, settings)?;
        insert.step_done()?;
        Ok(())
    })
}

fn instance_exists(
    db: &VerifiedDatabaseConnection<'_>,
    instance_id: &str,
) -> Result<bool, SeatError> {
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instances WHERE instance_id=?1",
    )?;
    q.bind_text(1, instance_id)?;
    Ok(q.step_row()?)
}
fn check_origin(
    db: &VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    domain: &str,
    target: Option<&Seat>,
) -> Result<(Layer, Option<String>), SeatError> {
    match origin {
        NativeOrigin::User(issuer) => {
            check_current_owner(db, issuer)?;
            Ok((Layer::User, None))
        }
        NativeOrigin::Lead(admission) => {
            if admission.domain_id != domain {
                return Err(SeatError::Denied);
            }
            let actor = read(db, domain, &admission.seat_id)?.ok_or(SeatError::Denied)?;
            if actor.layer != Layer::User
                || actor.state != State::Busy
                || actor.incarnation != admission.incarnation
                || actor.generation != admission.generation
            {
                return Err(SeatError::Denied);
            }
            if let Some(target) = target {
                if target.layer != Layer::Lead
                    || target.parent_seat_id.as_deref() != Some(&admission.seat_id)
                    || target.seat_id == admission.seat_id
                {
                    return Err(SeatError::Denied);
                }
            }
            Ok((Layer::Lead, Some(admission.seat_id.clone())))
        }
    }
}
fn operation(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    request: &str,
    fp: &str,
) -> Result<Option<SeatReceipt>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT o.fingerprint,o.seat_id,o.incarnation,o.layer,COALESCE(o.parent_seat_id,''),o.kind,o.instance_id,o.state,o.revision,o.generation,COALESCE(s.template_id,''),COALESCE(s.settings_json,'') FROM main.gogoke_v37_seat_operations AS o LEFT JOIN main.gogoke_v37_seat_operation_snapshots AS s ON s.domain_id=o.domain_id AND s.request_id=o.request_id WHERE o.domain_id=?1 AND o.request_id=?2")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, request)?;
    if !q.step_row()? {
        return Ok(None);
    }
    if q.column_text(0)? != fp {
        return Err(SeatError::Conflict);
    }
    let layer = match q.column_text(3)?.as_str() {
        "USER" => Layer::User,
        "LEAD" => Layer::Lead,
        _ => return Err(SeatError::SchemaDrift),
    };
    let parent = q.column_text(4)?;
    let kind = match q.column_text(5)?.as_str() {
        "LONG" => Kind::Long,
        "SHORT" => Kind::Short,
        _ => return Err(SeatError::SchemaDrift),
    };
    let (template_id, settings_json) =
        seat_template_fields(q.column_text(10)?, q.column_text(11)?)?;
    let state = match q.column_text(7)?.as_str() {
        "IDLE" => State::Idle,
        "BUSY" => State::Busy,
        "RECLAIMED" => State::Reclaimed,
        _ => return Err(SeatError::SchemaDrift),
    };
    let seat = Seat {
        domain_id: domain.into(),
        seat_id: q.column_text(1)?,
        incarnation: q.column_text(2)?,
        layer,
        parent_seat_id: if parent.is_empty() {
            None
        } else {
            Some(parent)
        },
        kind,
        instance_id: q.column_text(6)?,
        template_id,
        settings_json,
        state,
        revision: q
            .column_text(8)?
            .parse()
            .map_err(|_| SeatError::SchemaDrift)?,
        generation: q
            .column_text(9)?
            .parse()
            .map_err(|_| SeatError::SchemaDrift)?,
    };
    Ok(Some(SeatReceipt {
        seat,
        replayed: true,
    }))
}
fn authorize_replay(
    db: &VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    receipt: &SeatReceipt,
) -> Result<(), SeatError> {
    let current =
        read(db, &receipt.seat.domain_id, &receipt.seat.seat_id)?.ok_or(SeatError::SchemaDrift)?;
    // First recheck the live actor and layer relationship. A historical
    // request fingerprint cannot keep a stopped or revoked lead authorized.
    check_origin(db, origin, &receipt.seat.domain_id, Some(&current))?;
    // Even with a live actor, an old receipt cannot represent a later target
    // generation, reclamation or binding as the current successful result.
    // The exact reclaim receipt itself may replay while that same reclaimed
    // generation remains current; it does not restore dispatch authority.
    if current != receipt.seat {
        return Err(SeatError::Conflict);
    }
    Ok(())
}
fn record_operation(
    db: &VerifiedDatabaseConnection<'_>,
    request: &str,
    fp: &str,
    seat: &Seat,
) -> Result<(), SeatError> {
    let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seat_operations(domain_id,request_id,fingerprint,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,revision,generation) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")?;
    q.bind_text(1, &seat.domain_id)?;
    q.bind_text(2, request)?;
    q.bind_text(3, fp)?;
    q.bind_text(4, &seat.seat_id)?;
    q.bind_text(5, &seat.incarnation)?;
    q.bind_text(6, seat.layer.sql())?;
    if let Some(parent) = &seat.parent_seat_id {
        q.bind_text(7, parent)?;
    }
    q.bind_text(8, seat.kind.sql())?;
    q.bind_text(9, &seat.instance_id)?;
    q.bind_text(
        10,
        match seat.state {
            State::Idle => "IDLE",
            State::Busy => "BUSY",
            State::Reclaimed => "RECLAIMED",
        },
    )?;
    q.bind_i64(11, seat.revision)?;
    q.bind_i64(12, seat.generation)?;
    q.step_done()?;
    if let (Some(template_id), Some(settings_json)) = (&seat.template_id, &seat.settings_json) {
        let snapshot = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_operation_snapshots(domain_id,request_id,template_id,settings_json) VALUES(?1,?2,?3,?4)",
        )?;
        snapshot.bind_text(1, &seat.domain_id)?;
        snapshot.bind_text(2, request)?;
        snapshot.bind_text(3, template_id)?;
        snapshot.bind_text(4, settings_json)?;
        snapshot.step_done()?;
    }
    Ok(())
}

pub(crate) fn create(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: CreateSeat<'_>,
) -> Result<SeatReceipt, SeatError> {
    validate(
        input.domain_id,
        input.seat_id,
        input.request_id,
        input.request_bytes,
    )?;
    if !valid_id(input.template_id) {
        return Err(SeatError::Invalid("template_id"));
    }
    if let Some(instance_id) = input.instance_id {
        if !valid_id(instance_id) {
            return Err(SeatError::Invalid("instance_id"));
        }
    }
    let (layer_label, parent_label, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("USER", "", "", String::new()),
        NativeOrigin::Lead(a) => (
            "LEAD",
            a.seat_id.as_str(),
            a.incarnation.as_str(),
            a.generation.to_string(),
        ),
    };
    let fp = fingerprint(
        &[
            "create",
            input.domain_id,
            input.seat_id,
            input.template_id,
            input.instance_id.unwrap_or(""),
            input.kind.sql(),
            layer_label,
            parent_label,
            origin_incarnation,
            &origin_generation,
        ],
        input.request_bytes,
    );
    transact(db, |db| {
        // Authenticate before looking up a request ID as well as before the
        // first seat write. This covers both fresh writes and replay/conflict
        // paths in the same write group.
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let (layer, parent) = check_origin(db, &origin, input.domain_id, None)?;
        if read(db, input.domain_id, input.seat_id)?.is_some() {
            return Err(SeatError::Conflict);
        }
        let settings_json =
            template(db, input.domain_id, input.template_id)?.ok_or(SeatError::Unknown)?;
        if let Some(instance_id) = input.instance_id {
            if !instance_exists(db, instance_id)? {
                return Err(SeatError::Unknown);
            }
        }
        if let Some(instance_id) = input.instance_id {
            let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES(?1,?2,lower(hex(randomblob(16))),?3,?4,?5,?6,'IDLE',1,1)")?;
            q.bind_text(1, input.domain_id)?;
            q.bind_text(2, input.seat_id)?;
            q.bind_text(3, layer.sql())?;
            if let Some(parent) = &parent {
                q.bind_text(4, parent)?;
            }
            q.bind_text(5, input.kind.sql())?;
            q.bind_text(6, instance_id)?;
            q.step_done()?;
        } else {
            let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES(?1,?2,lower(hex(randomblob(16))),?3,?4,?5,NULL,'IDLE',1,1)")?;
            q.bind_text(1, input.domain_id)?;
            q.bind_text(2, input.seat_id)?;
            q.bind_text(3, layer.sql())?;
            if let Some(parent) = &parent {
                q.bind_text(4, parent)?;
            }
            q.bind_text(5, input.kind.sql())?;
            q.step_done()?;
        }
        let settings = Statement::prepare(
            db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES(?1,?2,?3,?4)",
        )?;
        settings.bind_text(1, input.domain_id)?;
        settings.bind_text(2, input.seat_id)?;
        settings.bind_text(3, input.template_id)?;
        settings.bind_text(4, &settings_json)?;
        settings.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt {
            seat,
            replayed: false,
        })
    })
}

fn change(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    action: &'static str,
    value: &str,
) -> Result<SeatReceipt, SeatError> {
    validate(
        input.domain_id,
        input.seat_id,
        input.request_id,
        input.request_bytes,
    )?;
    if input.expected_generation < 1 {
        return Err(SeatError::Invalid("generation"));
    }
    if input.expected_revision < 1 {
        return Err(SeatError::Invalid("revision"));
    }
    let (origin_id, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("", "", String::new()),
        NativeOrigin::Lead(a) => (
            a.seat_id.as_str(),
            a.incarnation.as_str(),
            a.generation.to_string(),
        ),
    };
    let fp = fingerprint(
        &[
            action,
            input.domain_id,
            input.seat_id,
            &input.expected_generation.to_string(),
            &input.expected_revision.to_string(),
            value,
            origin_id,
            origin_incarnation,
            &origin_generation,
        ],
        input.request_bytes,
    );
    transact(db, |db| {
        // Keep request replay lookup behind the current issuer check. A
        // request-ID collision must not become an issuer oracle.
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let before = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::Unknown)?;
        check_origin(db, &origin, input.domain_id, Some(&before))?;
        if before.state == State::Reclaimed {
            return Err(SeatError::Denied);
        }
        if before.generation != input.expected_generation {
            return Err(SeatError::Conflict);
        }
        if before.revision != input.expected_revision {
            return Err(SeatError::Conflict);
        }
        if before.state == State::Busy {
            return Err(SeatError::Busy);
        }
        let next_generation = before
            .generation
            .checked_add(1)
            .ok_or(SeatError::Conflict)?;
        let next_revision = before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let sql = match action {
            "bind-instance" => {
                if !before.instance_id.is_empty() {
                    return Err(SeatError::Conflict);
                }
                if !instance_exists(db, value)? { return Err(SeatError::Unknown); }
                "UPDATE main.gogoke_v37_seats SET instance_id=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'"
            }
            "change-instance" => {
                if before.instance_id.is_empty() || before.instance_id == value {
                    return Err(SeatError::Conflict);
                }
                if !instance_exists(db, value)? { return Err(SeatError::Unknown); }
                "UPDATE main.gogoke_v37_seats SET instance_id=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'"
            }
            "promote" => {
                if before.kind != Kind::Short { return Err(SeatError::Conflict); }
                "UPDATE main.gogoke_v37_seats SET kind=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'"
            }
            "reclaim" => "UPDATE main.gogoke_v37_seats SET state=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7 AND state='IDLE'",
            _ => return Err(SeatError::Invalid("action")),
        };
        let q = Statement::prepare(db.as_ptr(), sql)?;
        q.bind_text(1, value)?;
        q.bind_i64(2, next_generation)?;
        q.bind_i64(3, next_revision)?;
        q.bind_text(4, input.domain_id)?;
        q.bind_text(5, input.seat_id)?;
        q.bind_i64(6, before.generation)?;
        q.bind_i64(7, before.revision)?;
        q.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if seat.generation != next_generation || seat.revision != next_revision {
            return Err(SeatError::Conflict);
        }
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt {
            seat,
            replayed: false,
        })
    })
}
pub(crate) fn bind_instance(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    instance_id: &str,
) -> Result<SeatReceipt, SeatError> {
    if !valid_id(instance_id) {
        return Err(SeatError::Invalid("instance_id"));
    }
    change(db, origin, input, "bind-instance", instance_id)
}
pub(crate) fn change_instance(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    instance_id: &str,
) -> Result<SeatReceipt, SeatError> {
    if !valid_id(instance_id) {
        return Err(SeatError::Invalid("instance_id"));
    }
    change(db, origin, input, "change-instance", instance_id)
}
pub(crate) fn promote(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
) -> Result<SeatReceipt, SeatError> {
    change(db, origin, input, "promote", "LONG")
}
pub(crate) fn reclaim(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
) -> Result<SeatReceipt, SeatError> {
    change(db, origin, input, "reclaim", "RECLAIMED")
}

/// Change one copied template setting on the seat. The source template is
/// immutable; the seat revision and copied settings advance in one transaction.
pub(crate) fn tune(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: SeatChange<'_>,
    setting: &str,
    value_json: &str,
) -> Result<SeatReceipt, SeatError> {
    validate(input.domain_id, input.seat_id, input.request_id, input.request_bytes)?;
    if input.expected_generation < 1 || input.expected_revision < 1 {
        return Err(SeatError::Invalid("seat revision"));
    }
    if !valid_id(setting) { return Err(SeatError::Invalid("setting")); }
    if value_json.is_empty() || value_json.len() > crate::ipc::MAX_FRAME_BYTES {
        return Err(SeatError::Invalid("value"));
    }
    super::atomic::require_canonical_json(value_json.as_bytes(), "seat.value")?;
    let value = Parser::parse(value_json)?;
    let (origin_id, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("", "", String::new()),
        NativeOrigin::Lead(admission) => (
            admission.seat_id.as_str(), admission.incarnation.as_str(),
            admission.generation.to_string(),
        ),
    };
    let fp = fingerprint(&[
        "tune", input.domain_id, input.seat_id,
        &input.expected_generation.to_string(), &input.expected_revision.to_string(),
        setting, value_json, origin_id, origin_incarnation, &origin_generation,
    ], input.request_bytes);
    transact(db, |db| {
        check_origin(db, &origin, input.domain_id, None)?;
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            authorize_replay(db, &origin, &receipt)?;
            return Ok(receipt);
        }
        let before = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::Unknown)?;
        check_origin(db, &origin, input.domain_id, Some(&before))?;
        if before.state == State::Reclaimed { return Err(SeatError::Denied); }
        if before.generation != input.expected_generation || before.revision != input.expected_revision {
            return Err(SeatError::Conflict);
        }
        if before.state == State::Busy { return Err(SeatError::Busy); }
        let Some(settings_json) = &before.settings_json else { return Err(SeatError::SchemaDrift); };
        let Json::Object(mut settings) = Parser::parse(settings_json)? else {
            return Err(SeatError::SchemaDrift);
        };
        settings.insert(JsonString::from_str(setting), value);
        let updated_settings = Json::Object(settings).canonical();
        validate_template_settings(updated_settings.as_bytes())?;
        let next_generation = before.generation.checked_add(1).ok_or(SeatError::Conflict)?;
        let next_revision = before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let update_settings = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_settings SET settings_json=?1 WHERE domain_id=?2 AND seat_id=?3")?;
        update_settings.bind_text(1, &updated_settings)?;
        update_settings.bind_text(2, input.domain_id)?;
        update_settings.bind_text(3, input.seat_id)?;
        update_settings.step_done()?;
        let update_seat = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seats SET generation=?1,revision=?2 WHERE domain_id=?3 AND seat_id=?4 AND generation=?5 AND revision=?6 AND state='IDLE'")?;
        update_seat.bind_i64(1, next_generation)?;
        update_seat.bind_i64(2, next_revision)?;
        update_seat.bind_text(3, input.domain_id)?;
        update_seat.bind_text(4, input.seat_id)?;
        update_seat.bind_i64(5, before.generation)?;
        update_seat.bind_i64(6, before.revision)?;
        update_seat.step_done()?;
        let seat = read(db, input.domain_id, input.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if seat.generation != next_generation || seat.revision != next_revision
            || seat.settings_json.as_deref() != Some(updated_settings.as_str()) {
            return Err(SeatError::Conflict);
        }
        record_operation(db, input.request_id, &fp, &seat)?;
        Ok(SeatReceipt { seat, replayed: false })
    })
}

/// H must mark BUSY before launching a seat, and mark IDLE only after its
/// durable stop fact. BUSY survives restart and blocks a second binding until
/// H reconciles the stop; an absent process is not itself a stop fact.
pub(crate) fn set_dispatch_state(
    db: &mut VerifiedDatabaseConnection<'_>,
    seat: &Seat,
    busy: bool,
) -> Result<Seat, SeatError> {
    transact(db, |db| set_dispatch_state_in_transaction(db, seat, busy))
}

/// H calls this only inside its admission or proven-stop transaction on this
/// same connection, so seat state cannot commit independently of that fact.
pub(crate) fn set_dispatch_state_in_transaction(
    db: &mut VerifiedDatabaseConnection<'_>,
    seat: &Seat,
    busy: bool,
) -> Result<Seat, SeatError> {
        let current = read(db, &seat.domain_id, &seat.seat_id)?.ok_or(SeatError::Unknown)?;
        if current.incarnation != seat.incarnation
            || current.generation != seat.generation
            || current.revision != seat.revision
        {
            return Err(SeatError::Conflict);
        }
        if current.state != if busy { State::Idle } else { State::Busy } {
            return Err(SeatError::Conflict);
        }
        // Admission and stop both revoke tokens from the preceding runtime.
        // Otherwise an old lead token could become valid on a later BUSY turn.
        let generation = current
            .generation
            .checked_add(1)
            .ok_or(SeatError::Conflict)?;
        let revision = current.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_seats SET state=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND revision=?7")?;
        q.bind_text(1, if busy { "BUSY" } else { "IDLE" })?;
        q.bind_i64(2, generation)?;
        q.bind_i64(3, revision)?;
        q.bind_text(4, &seat.domain_id)?;
        q.bind_text(5, &seat.seat_id)?;
        q.bind_i64(6, seat.generation)?;
        q.bind_i64(7, seat.revision)?;
        q.step_done()?;
        let updated = read(db, &seat.domain_id, &seat.seat_id)?.ok_or(SeatError::SchemaDrift)?;
        if updated.revision != revision || updated.generation != generation {
            return Err(SeatError::Conflict);
        }
        Ok(updated)
}
