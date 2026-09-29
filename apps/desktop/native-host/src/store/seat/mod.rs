//! E.1 seat identity and instance binding on the product database connection.
//! This module receives only native capabilities. It is not an IPC dispatcher;
//! the future ingress must authenticate a live lead session before admission.

use super::atomic::{AtomicError, Statement};
use super::authority::OwnerIssuer;
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};

#[cfg(all(test, windows))]
mod tests;

const SEATS: &str = "CREATE TABLE gogoke_v37_seats(domain_id TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL UNIQUE,layer TEXT NOT NULL CHECK(layer IN ('USER','LEAD')),parent_seat_id TEXT,kind TEXT NOT NULL CHECK(kind IN ('LONG','SHORT')),instance_id TEXT NOT NULL REFERENCES gogoke_v37_instances(instance_id),state TEXT NOT NULL CHECK(state IN ('IDLE','BUSY','RECLAIMED')),generation INTEGER NOT NULL CHECK(generation >= 1),revision INTEGER NOT NULL CHECK(revision >= 1),CHECK((layer='USER' AND parent_seat_id IS NULL) OR (layer='LEAD' AND parent_seat_id IS NOT NULL)),PRIMARY KEY(domain_id,seat_id),FOREIGN KEY(domain_id,parent_seat_id) REFERENCES gogoke_v37_seats(domain_id,seat_id)) STRICT";
const OPERATIONS: &str = "CREATE TABLE gogoke_v37_seat_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,fingerprint TEXT NOT NULL,seat_id TEXT NOT NULL,incarnation TEXT NOT NULL,layer TEXT NOT NULL,parent_seat_id TEXT,kind TEXT NOT NULL,instance_id TEXT NOT NULL,state TEXT NOT NULL,revision INTEGER NOT NULL,generation INTEGER NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT";

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
    pub(crate) instance_id: String,
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
    pub(crate) instance_id: &'a str,
    pub(crate) kind: Kind,
    pub(crate) request_id: &'a str,
}
pub(crate) struct SeatChange<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) seat_id: &'a str,
    pub(crate) expected_generation: i64,
    pub(crate) request_id: &'a str,
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
fn validate(domain: &str, seat: &str, request: &str) -> Result<(), SeatError> {
    if !valid_id(domain) {
        return Err(SeatError::Invalid("domain_id"));
    }
    if !valid_id(seat) {
        return Err(SeatError::Invalid("seat_id"));
    }
    if !valid_id(request) {
        return Err(SeatError::Invalid("request_id"));
    }
    Ok(())
}
fn fingerprint(parts: &[&str]) -> String {
    let mut bytes = Vec::new();
    for part in parts {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
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
    vec![
        ("gogoke_v37_seat_operations".into(), OPERATIONS.into()),
        ("gogoke_v37_seats".into(), SEATS.into()),
    ]
}
/// Call after F's instance schema, on the same verified product connection.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), SeatError> {
    reject_shadow_or_effect(db)?;
    let observed = schema(db)?;
    if !observed.is_empty() {
        return if observed == expected_schema() {
            Ok(())
        } else {
            Err(SeatError::SchemaDrift)
        };
    }
    transact(db, |db| {
        reject_shadow_or_effect(db)?;
        if !schema(db)?.is_empty() {
            return Err(SeatError::SchemaDrift);
        }
        db.execute(SEATS)?;
        db.execute(OPERATIONS)?;
        if schema(db)? != expected_schema() {
            return Err(SeatError::SchemaDrift);
        }
        Ok(())
    })
}

fn read(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    seat_id: &str,
) -> Result<Option<Seat>, SeatError> {
    let q = Statement::prepare(db.as_ptr(), "SELECT incarnation,layer,COALESCE(parent_seat_id,''),kind,instance_id,state,generation,revision FROM main.gogoke_v37_seats WHERE domain_id=?1 AND seat_id=?2")?;
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
        NativeOrigin::User(_issuer) => Ok((Layer::User, None)),
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
    let q = Statement::prepare(db.as_ptr(), "SELECT fingerprint,seat_id,incarnation,layer,COALESCE(parent_seat_id,''),kind,instance_id,state,revision,generation FROM main.gogoke_v37_seat_operations WHERE domain_id=?1 AND request_id=?2")?;
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
    let current = read(db, domain, &seat.seat_id)?.ok_or(SeatError::SchemaDrift)?;
    if current.incarnation != seat.incarnation {
        return Err(SeatError::SchemaDrift);
    }
    Ok(Some(SeatReceipt {
        seat,
        replayed: true,
    }))
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
    Ok(())
}

pub(crate) fn create(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: NativeOrigin<'_>,
    input: CreateSeat<'_>,
) -> Result<SeatReceipt, SeatError> {
    validate(input.domain_id, input.seat_id, input.request_id)?;
    if !valid_id(input.instance_id) {
        return Err(SeatError::Invalid("instance_id"));
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
    let fp = fingerprint(&[
        "create",
        input.domain_id,
        input.seat_id,
        input.instance_id,
        input.kind.sql(),
        layer_label,
        parent_label,
        origin_incarnation,
        &origin_generation,
    ]);
    transact(db, |db| {
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
            return Ok(receipt);
        }
        let (layer, parent) = check_origin(db, &origin, input.domain_id, None)?;
        if read(db, input.domain_id, input.seat_id)?.is_some() {
            return Err(SeatError::Conflict);
        }
        if !instance_exists(db, input.instance_id)? {
            return Err(SeatError::Unknown);
        }
        let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES(?1,?2,lower(hex(randomblob(16))),?3,?4,?5,?6,'IDLE',1,1)")?;
        q.bind_text(1, input.domain_id)?;
        q.bind_text(2, input.seat_id)?;
        q.bind_text(3, layer.sql())?;
        if let Some(parent) = &parent {
            q.bind_text(4, parent)?;
        }
        q.bind_text(5, input.kind.sql())?;
        q.bind_text(6, input.instance_id)?;
        q.step_done()?;
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
    validate(input.domain_id, input.seat_id, input.request_id)?;
    if input.expected_generation < 1 {
        return Err(SeatError::Invalid("generation"));
    }
    let (origin_id, origin_incarnation, origin_generation) = match &origin {
        NativeOrigin::User(_) => ("", "", String::new()),
        NativeOrigin::Lead(a) => (
            a.seat_id.as_str(),
            a.incarnation.as_str(),
            a.generation.to_string(),
        ),
    };
    let fp = fingerprint(&[
        action,
        input.domain_id,
        input.seat_id,
        &input.expected_generation.to_string(),
        value,
        origin_id,
        origin_incarnation,
        &origin_generation,
    ]);
    transact(db, |db| {
        if let Some(receipt) = operation(db, input.domain_id, input.request_id, &fp)? {
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
        if before.state == State::Busy {
            return Err(SeatError::Busy);
        }
        let next_generation = before
            .generation
            .checked_add(1)
            .ok_or(SeatError::Conflict)?;
        let next_revision = before.revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let sql = match action {
            "bind" => {
                if before.instance_id == value { return Err(SeatError::Conflict); }
                if !instance_exists(db, value)? { return Err(SeatError::Unknown); }
                "UPDATE main.gogoke_v37_seats SET instance_id=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND state='IDLE'"
            }
            "promote" => {
                if before.kind != Kind::Short { return Err(SeatError::Conflict); }
                "UPDATE main.gogoke_v37_seats SET kind=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND state='IDLE'"
            }
            "reclaim" => "UPDATE main.gogoke_v37_seats SET state=?1,generation=?2,revision=?3 WHERE domain_id=?4 AND seat_id=?5 AND generation=?6 AND state='IDLE'",
            _ => return Err(SeatError::Invalid("action")),
        };
        let q = Statement::prepare(db.as_ptr(), sql)?;
        q.bind_text(1, value)?;
        q.bind_i64(2, next_generation)?;
        q.bind_i64(3, next_revision)?;
        q.bind_text(4, input.domain_id)?;
        q.bind_text(5, input.seat_id)?;
        q.bind_i64(6, before.generation)?;
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
    change(db, origin, input, "bind", instance_id)
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

/// H must mark BUSY before launching a seat, and mark IDLE only after its
/// durable stop fact. BUSY survives restart and blocks a second binding until
/// H reconciles the stop; an absent process is not itself a stop fact.
pub(crate) fn set_dispatch_state(
    db: &mut VerifiedDatabaseConnection<'_>,
    seat: &Seat,
    busy: bool,
) -> Result<Seat, SeatError> {
    transact(db, |db| {
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
    })
}
