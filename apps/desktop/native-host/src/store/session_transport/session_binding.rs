//! H-owned, immutable session-to-seat relationship. This table contains no A
//! purpose, H claim generation/state, home, process, stop, or custody fact.
//! Callers must hold the original User/native proof and the product transaction
//! when inserting a native binding. This module only records a relationship;
//! it never authorizes execution. Consumers must reread A purpose and the
//! current H/E/native authority before acting.

use crate::store::atomic::{AtomicError, Statement};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

pub(crate) const TABLE: &str = "gogoke_v37_session_binding_v2";
const SCHEMA: &str = "CREATE TABLE gogoke_v37_session_binding_v2(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,seat_authorization_generation INTEGER NOT NULL CHECK(seat_authorization_generation>=0),selected_instance_id TEXT NOT NULL,provenance TEXT NOT NULL CHECK(provenance IN ('LEGACY_V1','NATIVE_V2')),PRIMARY KEY(domain_id,session_id)) STRICT";
const PENDING: &str = "gogoke_v37_native_selection";
const PENDING_SCHEMA: &str = "CREATE TABLE gogoke_v37_native_selection(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,seat_authorization_generation INTEGER NOT NULL CHECK(seat_authorization_generation>=0),selected_instance_id TEXT NOT NULL,PRIMARY KEY(domain_id,session_id)) STRICT";
pub(crate) const EFFECTIVE_SEAT: &str = "gogoke_v37_effective_seat";
const EFFECTIVE_SEAT_SCHEMA: &str = "CREATE VIEW gogoke_v37_effective_seat AS SELECT p.domain_id,p.session_id,p.seat_id,p.seat_incarnation,c.generation,p.seat_authorization_generation,p.selected_instance_id,'NATIVE_V2' AS provenance FROM main.gogoke_v37_native_selection p JOIN main.gogoke_v37_h_claim c ON c.domain_id=p.domain_id AND c.session_id=p.session_id UNION ALL SELECT b.domain_id,b.session_id,b.seat_id,b.seat_incarnation,b.generation,CAST(b.generation AS INTEGER),c.instance_id,'LEGACY_V1' FROM main.gogoke_v37_h_seat_binding b JOIN main.gogoke_v37_h_claim c ON c.domain_id=b.domain_id AND c.session_id=b.session_id WHERE b.generation=CAST(CAST(b.generation AS INTEGER) AS TEXT) AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_native_selection p WHERE p.domain_id=b.domain_id AND p.session_id=b.session_id)";

#[derive(Debug)]
pub(crate) enum BindingError {
    Invalid(&'static str),
    Drift,
    Conflict,
    Store(AtomicError),
    Sqlite(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
}
impl From<AtomicError> for BindingError {
    fn from(error: AtomicError) -> Self { Self::Store(error) }
}
impl From<SameOpenError> for BindingError {
    fn from(error: SameOpenError) -> Self { Self::Sqlite(error) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Provenance { LegacyV1, NativeV2 }
impl Provenance {
    fn text(self) -> &'static str {
        match self { Self::LegacyV1 => "LEGACY_V1", Self::NativeV2 => "NATIVE_V2" }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionBinding {
    pub(crate) domain_id: String,
    pub(crate) session_id: String,
    pub(crate) seat_id: String,
    pub(crate) seat_incarnation: String,
    pub(crate) seat_authorization_generation: i64,
    pub(crate) selected_instance_id: String,
    pub(crate) provenance: Provenance,
}

/// Current E/H metadata, not a launch or model-call permission. In particular
/// a RESERVED claim may legitimately have no A registration. Execution callers
/// must check the original A purpose, native process and current grant as well.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CurrentRelationship {
    pub(crate) seat_id: String,
    pub(crate) seat_incarnation: String,
    pub(crate) seat_authorization_generation: i64,
    pub(crate) instance_id: String,
    pub(crate) session_generation: String,
    pub(crate) native_v2: bool,
}

/// Native V2 separates the current E authorization snapshot from the H process
/// generation and selected instance. Legacy execution keeps its existing exact
/// E/H equality rules; a historical LEGACY_V1 projection cannot replace them.
pub(crate) fn current_relationship(db: &VerifiedDatabaseConnection<'_>,
    domain: &str, session: &str) -> Result<Option<CurrentRelationship>, BindingError> {
    schema_state(db)?.ok_or(BindingError::Drift)?;
    let q = Statement::prepare(db.as_ptr(),
        "SELECT s.seat_id,s.incarnation,s.generation,h.instance_id,h.generation,CASE WHEN v.provenance='NATIVE_V2' THEN '1' ELSE '0' END
           FROM main.gogoke_v37_h_claim h
           JOIN main.gogoke_v37_h_owner_binding o ON o.binding_id=h.binding_id
             AND o.instance_id=h.instance_id AND o.domain_id=h.domain_id
             AND o.kind='SESSION' AND o.owner_id=h.session_id
             AND o.generation=h.generation AND o.state='ACTIVE'
           JOIN main.gogoke_v37_instance_homes home ON home.home_id=h.home_id
             AND home.instance_id=h.instance_id AND home.domain_id=h.domain_id
             AND home.kind='SESSION' AND home.owner_id=h.session_id
             AND home.generation=h.generation AND home.state='ACTIVE'
           JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id
           LEFT JOIN main.gogoke_v37_session_binding_v2 v
             ON v.domain_id=h.domain_id AND v.session_id=h.session_id
           LEFT JOIN main.gogoke_v37_h_seat_binding old
             ON old.domain_id=h.domain_id AND old.session_id=h.session_id
           JOIN main.gogoke_v37_seats s ON s.domain_id=h.domain_id AND s.state='BUSY'
             AND ((v.provenance='NATIVE_V2' AND s.seat_id=v.seat_id
               AND s.incarnation=v.seat_incarnation
               AND s.generation=v.seat_authorization_generation
                AND h.instance_id=v.selected_instance_id
                AND s.instance_id=v.selected_instance_id)
             OR ((v.provenance IS NULL OR v.provenance='LEGACY_V1')
               AND s.seat_id=old.seat_id AND s.incarnation=old.seat_incarnation
               AND CAST(s.generation AS TEXT)=old.generation
               AND old.generation=h.generation AND s.instance_id=h.instance_id))
          WHERE h.domain_id=?1 AND h.session_id=?2 AND h.state<>'RELEASED'")?;
    q.bind_text(1,domain)?; q.bind_text(2,session)?;
    if !q.step_row()? { return Ok(None); }
    let relationship = CurrentRelationship {
        seat_id:q.column_text(0)?, seat_incarnation:q.column_text(1)?,
        seat_authorization_generation:q.column_text(2)?.parse().map_err(|_|BindingError::Drift)?,
        instance_id:q.column_text(3)?, session_generation:q.column_text(4)?,
        native_v2:q.column_text(5)?=="1",
    };
    if q.step_row()? { return Err(BindingError::Conflict); }
    if relationship.native_v2
        && authorization_generation(db,domain,session)?
            !=relationship.seat_authorization_generation {return Err(BindingError::Conflict);}
    Ok(Some(relationship))
}
impl SessionBinding {
    fn validate(&self) -> Result<(), BindingError> {
        for (name, value) in [
            ("domain_id", self.domain_id.as_str()),
            ("session_id", self.session_id.as_str()),
            ("seat_id", self.seat_id.as_str()),
            ("seat_incarnation", self.seat_incarnation.as_str()),
            ("selected_instance_id", self.selected_instance_id.as_str()),
        ] {
            if value.is_empty() || value.len() > 128 || !value.bytes().all(|byte|
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')) {
                return Err(BindingError::Invalid(name));
            }
        }
        if self.seat_authorization_generation < 0 {
            return Err(BindingError::Invalid("seat_authorization_generation"));
        }
        Ok(())
    }
}

fn schema_state(db: &VerifiedDatabaseConnection<'_>) -> Result<Option<String>, BindingError> {
    // SQLite's implicit PK index has NULL sql and is allowed. All user SQL
    // effects, including arbitrary-name TEMP triggers on this table, are not.
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name) LIKE ?1 || '%' OR lower(tbl_name)=?1 LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name)=?1 LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE lower(name) LIKE ?1 || '%' AND lower(name)<>?1 LIMIT 1",
    ] {
        let row = Statement::prepare(db.as_ptr(), sql)?;
        row.bind_text(1, TABLE)?;
        if row.step_row()? { return Err(BindingError::Drift); }
    }
    let row = Statement::prepare(db.as_ptr(),
        "SELECT type,sql FROM main.sqlite_schema WHERE lower(name)=?1")?;
    row.bind_text(1, TABLE)?;
    if !row.step_row()? { return Ok(None); }
    let kind = row.column_text(0)?;
    let sql = row.column_text(1)?;
    if row.step_row()? || kind != "table" || sql != SCHEMA {
        return Err(BindingError::Drift);
    }
    Ok(Some(sql))
}

fn auxiliary_state(db: &VerifiedDatabaseConnection<'_>, name: &str,
    kind: &str, expected: &str) -> Result<bool, BindingError> {
    let temp = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name)=?1 OR lower(tbl_name)=?1 LIMIT 1")?;
    temp.bind_text(1,name)?;
    if temp.step_row()? { return Err(BindingError::Drift); }
    let effects = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name)=?1 LIMIT 1")?;
    effects.bind_text(1,name)?;
    if effects.step_row()? { return Err(BindingError::Drift); }
    let row = Statement::prepare(db.as_ptr(),
        "SELECT type,sql FROM main.sqlite_schema WHERE lower(name)=?1")?;
    row.bind_text(1,name)?;
    if !row.step_row()? { return Ok(false); }
    if row.column_text(0)? != kind || row.column_text(1)? != expected || row.step_row()? {
        return Err(BindingError::Drift);
    }
    Ok(true)
}

/// Independent of admission's historical 5/6/8/9-table upgrade chain.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), BindingError> {
    if schema_state(db)?.is_some()
        && auxiliary_state(db,PENDING,"table",PENDING_SCHEMA)?
        && auxiliary_state(db,EFFECTIVE_SEAT,"view",EFFECTIVE_SEAT_SCHEMA)? { return Ok(()); }
    db.execute("BEGIN IMMEDIATE")?;
    let outcome = (|| {
        if schema_state(db)?.is_none() { db.execute(SCHEMA)?; }
        if !auxiliary_state(db,PENDING,"table",PENDING_SCHEMA)? { db.execute(PENDING_SCHEMA)?; }
        if !auxiliary_state(db,EFFECTIVE_SEAT,"view",EFFECTIVE_SEAT_SCHEMA)? {
            db.execute(EFFECTIVE_SEAT_SCHEMA)?;
        }
        if schema_state(db)?.is_none() { return Err(BindingError::Drift); }
        if !auxiliary_state(db,PENDING,"table",PENDING_SCHEMA)?
            || !auxiliary_state(db,EFFECTIVE_SEAT,"view",EFFECTIVE_SEAT_SCHEMA)? {
            return Err(BindingError::Drift);
        }
        Ok(())
    })();
    match outcome {
        Ok(()) => db.execute("COMMIT").map_err(BindingError::CommitUnknown),
        Err(error) => {
            db.execute("ROLLBACK").map_err(BindingError::RollbackUnknown)?;
            Err(error)
        }
    }
}

/// Raw data read, not an authority decision. The caller must verify A purpose,
/// H lifecycle, E authorization, and native proof in its own transaction.
pub(crate) fn read(db: &VerifiedDatabaseConnection<'_>, domain: &str, session: &str)
    -> Result<Option<SessionBinding>, BindingError> {
    schema_state(db)?.ok_or(BindingError::Drift)?;
    let row = Statement::prepare(db.as_ptr(),
        "SELECT domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id,provenance FROM main.gogoke_v37_session_binding_v2 WHERE domain_id=?1 AND session_id=?2")?;
    row.bind_text(1, domain)?; row.bind_text(2, session)?;
    if !row.step_row()? { return Ok(None); }
    let provenance = match row.column_text(6)?.as_str() {
        "LEGACY_V1" => Provenance::LegacyV1,
        "NATIVE_V2" => Provenance::NativeV2,
        _ => return Err(BindingError::Drift),
    };
    let binding = SessionBinding {
        domain_id: row.column_text(0)?, session_id: row.column_text(1)?,
        seat_id: row.column_text(2)?, seat_incarnation: row.column_text(3)?,
        seat_authorization_generation: row.column_text(4)?.parse()
            .map_err(|_| BindingError::Drift)?,
        selected_instance_id: row.column_text(5)?, provenance,
    };
    if row.step_row()? { return Err(BindingError::Drift); }
    binding.validate()?;
    Ok(Some(binding))
}

pub(crate) fn read_pending(db: &VerifiedDatabaseConnection<'_>, domain: &str, session: &str)
    -> Result<Option<SessionBinding>, BindingError> {
    if !auxiliary_state(db,PENDING,"table",PENDING_SCHEMA)? { return Err(BindingError::Drift); }
    let row=Statement::prepare(db.as_ptr(),
        "SELECT domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id FROM main.gogoke_v37_native_selection WHERE domain_id=?1 AND session_id=?2")?;
    row.bind_text(1,domain)?; row.bind_text(2,session)?;
    if !row.step_row()? { return Ok(None); }
    let binding=SessionBinding {domain_id:row.column_text(0)?,session_id:row.column_text(1)?,
        seat_id:row.column_text(2)?,seat_incarnation:row.column_text(3)?,
        seat_authorization_generation:row.column_text(4)?.parse()
            .map_err(|_|BindingError::Drift)?,selected_instance_id:row.column_text(5)?,
        provenance:Provenance::NativeV2};
    if row.step_row()? { return Err(BindingError::Drift); }
    binding.validate()?;
    Ok(Some(binding))
}

/// Use only after a caller has independently proved the current H operation,
/// physical custody and A source. This value is the E authorization epoch,
/// never a substitute for that physical proof.
pub(crate) fn authorization_generation(db:&VerifiedDatabaseConnection<'_>,
    domain:&str,session:&str)->Result<i64,BindingError> {
    if let Some(pending)=read_pending(db,domain,session)? {
        if read(db,domain,session)?.as_ref()!=Some(&pending) {return Err(BindingError::Conflict);}
        return Ok(pending.seat_authorization_generation);
    }
    let old=Statement::prepare(db.as_ptr(),
        "SELECT generation FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2")?;
    old.bind_text(1,domain)?;old.bind_text(2,session)?;
    if !old.step_row()? {return Err(BindingError::Conflict);}
    let raw=old.column_text(0)?;
    let generation=raw.parse::<i64>().map_err(|_|BindingError::Drift)?;
    if generation<0 || generation.to_string()!=raw {return Err(BindingError::Drift);}
    if old.step_row()? {return Err(BindingError::Conflict);}
    Ok(generation)
}

/// Read only, inside the caller's product transaction after it settles one
/// claim. Every non-RELEASED H claim for this exact seat incarnation still
/// holds E BUSY, including STOPPED and pre-open native reservations.
pub(crate) fn has_unreleased_seat_claim(db:&VerifiedDatabaseConnection<'_>,
    domain:&str,seat:&str,incarnation:&str)->Result<bool,BindingError> {
    if !auxiliary_state(db,EFFECTIVE_SEAT,"view",EFFECTIVE_SEAT_SCHEMA)? {
        return Err(BindingError::Drift);
    }
    let unbound=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_claim c
           LEFT JOIN main.gogoke_v37_effective_seat b
             ON b.domain_id=c.domain_id AND b.session_id=c.session_id
          WHERE c.domain_id=?1 AND c.state<>'RELEASED'
            AND b.session_id IS NULL LIMIT 1")?;
    unbound.bind_text(1,domain)?;
    if unbound.step_row()? {return Err(BindingError::Conflict);}
    let rows=Statement::prepare(db.as_ptr(),
        "SELECT c.session_id,COALESCE(c.process_operation_id,''),b.provenance
           FROM main.gogoke_v37_h_claim c
           JOIN main.gogoke_v37_effective_seat b
             ON b.domain_id=c.domain_id AND b.session_id=c.session_id
          WHERE c.domain_id=?1 AND b.seat_id=?2 AND b.seat_incarnation=?3
            AND c.state<>'RELEASED'")?;
    rows.bind_text(1,domain)?;rows.bind_text(2,seat)?;rows.bind_text(3,incarnation)?;
    let mut found=false;
    while rows.step_row()? {
        let session=rows.column_text(0)?;
        let operation=rows.column_text(1)?;
        let provenance=rows.column_text(2)?;
        if provenance=="NATIVE_V2" {
            let pending=read_pending(db,domain,&session)?.ok_or(BindingError::Conflict)?;
            if pending.seat_id!=seat || pending.seat_incarnation!=incarnation {
                return Err(BindingError::Conflict);
            }
            if !operation.is_empty() && read(db,domain,&session)?.as_ref()!=Some(&pending) {
                return Err(BindingError::Conflict);
            }
        } else if provenance!="LEGACY_V1" {return Err(BindingError::Drift);}
        found=true;
    }
    Ok(found)
}

/// H's original admission transaction records this selection before a process
/// exists. It is not an execution grant and never infers provenance from an
/// absent legacy row. Initial native open must match it exactly.
pub(crate) fn select_native_in_transaction(db: &VerifiedDatabaseConnection<'_>,
    binding:&SessionBinding) -> Result<(),BindingError> {
    if binding.provenance!=Provenance::NativeV2 { return Err(BindingError::Invalid("provenance")); }
    binding.validate()?;
    if let Some(existing)=read_pending(db,&binding.domain_id,&binding.session_id)? {
        return if existing==*binding {Ok(())} else {Err(BindingError::Conflict)};
    }
    let row=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_native_selection(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id) VALUES(?1,?2,?3,?4,?5,?6)")?;
    for (index,value) in [binding.domain_id.as_str(),binding.session_id.as_str(),
        binding.seat_id.as_str(),binding.seat_incarnation.as_str()].iter().enumerate() {
        row.bind_text((index+1) as i32,value)?;
    }
    row.bind_i64(5,binding.seat_authorization_generation)?;
    row.bind_text(6,&binding.selected_instance_id)?;
    row.step_done()?;
    Ok(())
}

/// Requires the caller's open product transaction and original native E seat
/// authorization plus User/native request proof. No authority is minted here.
/// Exact same-session replay is idempotent; a changed field is a conflict.
pub(crate) fn insert_native_in_transaction(db: &VerifiedDatabaseConnection<'_>, binding: &SessionBinding)
    -> Result<(), BindingError> {
    if binding.provenance != Provenance::NativeV2 { return Err(BindingError::Invalid("provenance")); }
    if read_pending(db,&binding.domain_id,&binding.session_id)?.as_ref()!=Some(binding) {
        return Err(BindingError::Conflict);
    }
    insert_exact(db, binding)
}

fn insert_exact(db: &VerifiedDatabaseConnection<'_>, binding: &SessionBinding)
    -> Result<(), BindingError> {
    binding.validate()?;
    if let Some(existing) = read(db, &binding.domain_id, &binding.session_id)? {
        return if existing == *binding { Ok(()) } else { Err(BindingError::Conflict) };
    }
    let row = Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_session_binding_v2(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id,provenance) VALUES(?1,?2,?3,?4,?5,?6,?7)")?;
    for (index,value) in [binding.domain_id.as_str(),binding.session_id.as_str(),
        binding.seat_id.as_str(),binding.seat_incarnation.as_str()].iter().enumerate() {
        row.bind_text((index+1) as i32,value)?;
    }
    row.bind_i64(5,binding.seat_authorization_generation)?;
    row.bind_text(6,&binding.selected_instance_id)?;
    row.bind_text(7,binding.provenance.text())?;
    row.step_done()?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnprojectedReason {
    MissingClaim, MissingSeatBinding, InconsistentLegacyFacts,
    AuthorizationGenerationAdvanced,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Unprojected {
    pub(crate) domain_id: String,
    pub(crate) session_id: String,
    pub(crate) reason: UnprojectedReason,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProjectionReport {
    pub(crate) projected: usize,
    pub(crate) already_projected: usize,
    pub(crate) unprojected: Vec<Unprojected>,
}

/// Run after the old H schema is initialized, under the same verified product
/// DB. Only the new table is written. A need not exist yet: A purpose is never
/// inferred here. Claim rows without a complete old relationship remain legal
/// and continue to count under H's original caps.
pub(crate) fn project_legacy(db: &mut VerifiedDatabaseConnection<'_>)
    -> Result<ProjectionReport, BindingError> {
    schema_state(db)?.ok_or(BindingError::Drift)?;
    db.execute("BEGIN IMMEDIATE")?;
    let outcome = project_legacy_in_transaction(db);
    match outcome {
        Ok(report) => {
            db.execute("COMMIT").map_err(BindingError::CommitUnknown)?;
            Ok(report)
        }
        Err(error) => {
            db.execute("ROLLBACK").map_err(BindingError::RollbackUnknown)?;
            Err(error)
        }
    }
}

fn project_legacy_in_transaction(db: &VerifiedDatabaseConnection<'_>)
    -> Result<ProjectionReport, BindingError> {
    let keys = Statement::prepare(db.as_ptr(),
        "SELECT domain_id,session_id FROM main.gogoke_v37_h_seat_binding UNION SELECT domain_id,session_id FROM main.gogoke_v37_h_claim ORDER BY domain_id,session_id")?;
    let mut report = ProjectionReport::default();
    while keys.step_row()? {
        let domain = keys.column_text(0)?;
        let session = keys.column_text(1)?;
        let existing = read(db,&domain,&session)?;
        // The native path may retain old H rows while replacing their seat
        // relationship. It is never a legacy projection candidate.
        if matches!(existing.as_ref().map(|row| row.provenance), Some(Provenance::NativeV2)) {
            continue;
        }
        if existing.is_none() {
            // A reservation can survive a cold reopen before any native open.
            // Its old H seat-binding is an admission fact, not a legacy session
            // relationship. H backfills actual old process episodes first.
            let opened = Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_h_process_episode
                  WHERE domain_id=?1 AND session_id=?2 AND old_generation IS NULL
                    AND process_operation_id IS NOT NULL LIMIT 1")?;
            opened.bind_text(1,&domain)?;
            opened.bind_text(2,&session)?;
            if !opened.step_row()? { continue; }
        }
        let source = Statement::prepare(db.as_ptr(),
            "SELECT COALESCE(b.seat_id,''),COALESCE(b.seat_incarnation,''),COALESCE(b.generation,''),COALESCE(c.instance_id,''),COALESCE(c.home_id,''),COALESCE(c.binding_id,''),COALESCE(c.generation,''),COALESCE(o.instance_id,''),COALESCE(o.domain_id,''),COALESCE(o.kind,''),COALESCE(o.owner_id,''),COALESCE(o.generation,''),COALESCE(h.instance_id,''),COALESCE(h.domain_id,''),COALESCE(h.kind,''),COALESCE(h.owner_id,''),COALESCE(h.generation,'') FROM (SELECT ?1 AS domain_id,?2 AS session_id) k LEFT JOIN main.gogoke_v37_h_seat_binding b ON b.domain_id=k.domain_id AND b.session_id=k.session_id LEFT JOIN main.gogoke_v37_h_claim c ON c.domain_id=k.domain_id AND c.session_id=k.session_id LEFT JOIN main.gogoke_v37_h_owner_binding o ON o.binding_id=c.binding_id LEFT JOIN main.gogoke_v37_instance_homes h ON h.home_id=c.home_id")?;
        source.bind_text(1,&domain)?; source.bind_text(2,&session)?;
        if !source.step_row()? { return Err(BindingError::Drift); }
        let fields = (0..17).map(|index| source.column_text(index)).collect::<Result<Vec<_>,_>>()?;
        if source.step_row()? { return Err(BindingError::Drift); }
        let reason = if fields[0].is_empty() { Some(UnprojectedReason::MissingSeatBinding) }
            else if fields[3].is_empty() { Some(UnprojectedReason::MissingClaim) }
            else if (existing.is_none() && fields[2] != fields[6]) ||
                fields[3] != fields[7] ||
                domain != fields[8] || fields[9] != "SESSION" || fields[10] != session ||
                fields[6] != fields[11] || fields[3] != fields[12] ||
                domain != fields[13] || fields[14] != "SESSION" || fields[15] != session ||
                fields[6] != fields[16] {
                Some(UnprojectedReason::InconsistentLegacyFacts)
            } else { None };
        if let Some(reason) = reason {
            if existing.is_some() { return Err(BindingError::Conflict); }
            report.unprojected.push(Unprojected { domain_id:domain, session_id:session, reason });
            continue;
        }
        let generation = match fields[2].parse::<i64>() {
            Ok(number) if number >= 0 && number.to_string() == fields[2] => number,
            _ => {
                if existing.is_some() { return Err(BindingError::Conflict); }
                report.unprojected.push(Unprojected { domain_id:domain, session_id:session,
                    reason:UnprojectedReason::InconsistentLegacyFacts });
                continue;
            }
        };
        let binding = SessionBinding { domain_id:domain, session_id:session,
            seat_id:fields[0].clone(), seat_incarnation:fields[1].clone(),
            seat_authorization_generation:generation,
            selected_instance_id:fields[3].clone(), provenance:Provenance::LegacyV1 };
        if let Some(existing) = existing {
            // The old resume path legitimately advances both the E seat and
            // old H seat-binding generation. V2 must not be silently rewritten.
            // The old H generation row proves this was a real prior process;
            // consumers must decide whether the immutable V2 is still usable.
            if existing != binding {
                if existing.domain_id != binding.domain_id ||
                    existing.session_id != binding.session_id ||
                    existing.seat_id != binding.seat_id ||
                    existing.seat_incarnation != binding.seat_incarnation ||
                    existing.selected_instance_id != binding.selected_instance_id ||
                    existing.provenance != Provenance::LegacyV1 ||
                    existing.seat_authorization_generation >= binding.seat_authorization_generation {
                    return Err(BindingError::Conflict);
                }
                let previous = Statement::prepare(db.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_h_generation WHERE domain_id=?1 AND session_id=?2 AND generation=?3 LIMIT 1")?;
                previous.bind_text(1,&binding.domain_id)?;
                previous.bind_text(2,&binding.session_id)?;
                previous.bind_text(3,&existing.seat_authorization_generation.to_string())?;
                if !previous.step_row()? { return Err(BindingError::Conflict); }
                report.unprojected.push(Unprojected { domain_id:binding.domain_id,
                    session_id:binding.session_id,
                    reason:UnprojectedReason::AuthorizationGenerationAdvanced });
                continue;
            }
            report.already_projected += 1;
        } else {
            insert_exact(db,&binding)?;
            report.projected += 1;
        }
    }
    Ok(report)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::route_b_test_guard;
    use std::time::{SystemTime,UNIX_EPOCH};

    fn with_product_db(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!(
            "gogoke-h-binding-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = crate::store::session::open_product_database(&root,&path).unwrap();
        initialize_schema(&mut db).unwrap();
        run(&mut db);
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_dir_all(folder).unwrap();
    }

    fn legacy(db: &mut VerifiedDatabaseConnection<'_>, session: &str, state: &str,
        generation: &str, with_binding: bool) {
        db.execute("INSERT OR IGNORE INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','refA','identityA','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
        db.execute("INSERT OR IGNORE INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','incA','USER',NULL,'LONG','instanceA','BUSY',1,1)").unwrap();
        let binding_id = format!("binding{session}");
        let home_id = format!("home{session}");
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES(?1,'instanceA','projectA','SESSION',?2,?3,'ACTIVE')").unwrap();
        insert.bind_text(1,&binding_id).unwrap(); insert.bind_text(2,session).unwrap();
        insert.bind_text(3,generation).unwrap(); insert.step_done().unwrap(); drop(insert);
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_ref,directory_identity,state,revision) VALUES(?1,'instanceA','projectA','SESSION',?2,?3,NULL,NULL,'ACTIVE',1)").unwrap();
        insert.bind_text(1,&home_id).unwrap(); insert.bind_text(2,session).unwrap();
        insert.bind_text(3,generation).unwrap(); insert.step_done().unwrap(); drop(insert);
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('projectA',?1,'instanceA',?2,?3,?4,?5,7,NULL,NULL)").unwrap();
        for (i,v) in [session,home_id.as_str(),binding_id.as_str(),generation,state].iter().enumerate() {
            insert.bind_text((i+1) as i32,v).unwrap();
        }
        insert.step_done().unwrap(); drop(insert);
        if state == "STOPPED" || state == "UNKNOWN" || (state == "COMMITTED" && !with_binding) {
            let operation_id = format!("open{session}");
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA',?1,'00ff','open',?2,'APPLIED',6,7)").unwrap();
            insert.bind_text(1,&operation_id).unwrap(); insert.bind_text(2,session).unwrap();
            insert.step_done().unwrap(); drop(insert);
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,process_operation_id,instance_id,home_id,binding_id,phase,stop_fact_id) VALUES('projectA',?1,?2,?3,'cafebabe',6,?1,'instanceA',?4,?5,?6,?7)").unwrap();
            let stop = if state == "STOPPED" { "stopFactA" } else { "" };
            let episode_phase = if state == "COMMITTED" { "PREPARED" } else { state };
            for (i,v) in [operation_id.as_str(),session,generation,home_id.as_str(),
                binding_id.as_str(),episode_phase,stop].iter().enumerate() {
                insert.bind_text((i+1) as i32,v).unwrap();
            }
            insert.step_done().unwrap();
        }
        if with_binding {
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_seat_binding(domain_id,session_id,seat_id,seat_incarnation,generation) VALUES('projectA',?1,'seatA','incA',?2)").unwrap();
            insert.bind_text(1,session).unwrap(); insert.bind_text(2,generation).unwrap();
            insert.step_done().unwrap();
        }
    }

    fn scalar(db: &VerifiedDatabaseConnection<'_>, sql: &str) -> String {
        let row = Statement::prepare(db.as_ptr(),sql).unwrap();
        assert!(row.step_row().unwrap()); row.column_text(0).unwrap()
    }

    #[test]
    fn projects_complete_legacy_without_a_and_preserves_old_raw_facts() {
        with_product_db(|db| {
            legacy(db,"sessionStopped","STOPPED","0",true);
            legacy(db,"sessionUnknown","UNKNOWN","1",true);
            legacy(db,"sessionIncomplete","COMMITTED","2",false);
            let before = scalar(db,"SELECT group_concat(session_id||':'||state||':'||generation||':'||revision||':'||coalesce(stop_fact_id,'NULL'),'|') FROM main.gogoke_v37_h_claim ORDER BY session_id");
            let owner_before = scalar(db,"SELECT group_concat(binding_id||':'||state||':'||generation,'|') FROM main.gogoke_v37_h_owner_binding ORDER BY binding_id");
            let home_before = scalar(db,"SELECT group_concat(home_id||':'||state||':'||generation,'|') FROM main.gogoke_v37_instance_homes ORDER BY home_id");
            let raw_before = scalar(db,"SELECT group_concat(request_id||':'||raw_hex||':'||status,'|') FROM main.gogoke_v37_h_operation ORDER BY request_id");
            let episode_before = scalar(db,"SELECT group_concat(request_id||':'||raw_hex||':'||phase||':'||coalesce(stop_fact_id,'NULL'),'|') FROM main.gogoke_v37_h_process_episode ORDER BY request_id");
            let report = project_legacy(db).unwrap();
            assert_eq!(report.projected,2);
            assert_eq!(report.unprojected.len(),1);
            assert_eq!(report.unprojected[0].reason,UnprojectedReason::MissingSeatBinding);
            assert_eq!(read(db,"projectA","sessionStopped").unwrap().unwrap().seat_authorization_generation,0);
            assert_eq!(read(db,"projectA","sessionUnknown").unwrap().unwrap().selected_instance_id,"instanceA");
            assert!(read(db,"projectA","sessionIncomplete").unwrap().is_none());
            assert_eq!(project_legacy(db).unwrap().already_projected,2);
            assert_eq!(scalar(db,"SELECT group_concat(session_id||':'||state||':'||generation||':'||revision||':'||coalesce(stop_fact_id,'NULL'),'|') FROM main.gogoke_v37_h_claim ORDER BY session_id"),before);
            assert_eq!(scalar(db,"SELECT group_concat(binding_id||':'||state||':'||generation,'|') FROM main.gogoke_v37_h_owner_binding ORDER BY binding_id"),owner_before);
            assert_eq!(scalar(db,"SELECT group_concat(home_id||':'||state||':'||generation,'|') FROM main.gogoke_v37_instance_homes ORDER BY home_id"),home_before);
            assert_eq!(scalar(db,"SELECT group_concat(request_id||':'||raw_hex||':'||status,'|') FROM main.gogoke_v37_h_operation ORDER BY request_id"),raw_before);
            assert_eq!(scalar(db,"SELECT group_concat(request_id||':'||raw_hex||':'||phase||':'||coalesce(stop_fact_id,'NULL'),'|') FROM main.gogoke_v37_h_process_episode ORDER BY request_id"),episode_before);
            assert_eq!(scalar(db,"SELECT count(*) FROM main.gogoke_v37_h_claim WHERE state IN ('RESERVED','COMMITTED','STOPPED','UNKNOWN')"),"3");
        });
    }

    #[test]
    fn native_exact_reopen_conflict_and_legacy_tamper_are_distinct() {
        with_product_db(|db| {
            let original = SessionBinding { domain_id:"projectA".into(),session_id:"nativeA".into(),
                seat_id:"seatA".into(),seat_incarnation:"incA".into(),
                seat_authorization_generation:0,selected_instance_id:"instanceA".into(),
                provenance:Provenance::NativeV2 };
            db.execute("BEGIN IMMEDIATE").unwrap();
            select_native_in_transaction(db,&original).unwrap();
            insert_native_in_transaction(db,&original).unwrap();
            insert_native_in_transaction(db,&original).unwrap();
            let mut changed = original.clone(); changed.selected_instance_id="instanceB".into();
            assert!(matches!(insert_native_in_transaction(db,&changed),Err(BindingError::Conflict)));
            db.execute("COMMIT").unwrap();
            legacy(db,"legacyA","UNKNOWN","1",true);
            assert_eq!(project_legacy(db).unwrap().projected,1);
            db.execute("UPDATE main.gogoke_v37_session_binding_v2 SET selected_instance_id='wrong' WHERE session_id='legacyA'").unwrap();
            assert!(matches!(project_legacy(db),Err(BindingError::Conflict)));
            assert_eq!(scalar(db,"SELECT state FROM main.gogoke_v37_h_claim WHERE session_id='legacyA'"),"UNKNOWN");
        });
    }

    #[test]
    fn cold_preopen_admission_does_not_become_legacy_binding() {
        with_product_db(|db| {
            legacy(db,"reservedA","COMMITTED","1",true);
            let report=project_legacy(db).unwrap();
            assert_eq!(report.projected,0);
            assert!(read(db,"projectA","reservedA").unwrap().is_none());
            let binding=SessionBinding {domain_id:"projectA".into(),session_id:"reservedA".into(),
                seat_id:"seatA".into(),seat_incarnation:"incA".into(),
                seat_authorization_generation:1,selected_instance_id:"instanceA".into(),
                provenance:Provenance::NativeV2};
            db.execute("BEGIN IMMEDIATE").unwrap();
            select_native_in_transaction(db,&binding).unwrap();
            insert_native_in_transaction(db,&binding).unwrap();
            db.execute("COMMIT").unwrap();
            assert_eq!(read(db,"projectA","reservedA").unwrap(),Some(binding));
        });
    }

    #[test]
    fn two_native_reservations_share_e_without_minting_open_authority() {
        with_product_db(|db| {
            legacy(db,"nativeA","RESERVED","1",false);
            legacy(db,"nativeB","RESERVED","1",false);
            let selection=|session:&str| SessionBinding {
                domain_id:"projectA".into(),session_id:session.into(),
                seat_id:"seatA".into(),seat_incarnation:"incA".into(),
                seat_authorization_generation:1,selected_instance_id:"instanceA".into(),
                provenance:Provenance::NativeV2};
            db.execute("BEGIN IMMEDIATE").unwrap();
            select_native_in_transaction(db,&selection("nativeA")).unwrap();
            select_native_in_transaction(db,&selection("nativeB")).unwrap();
            db.execute("COMMIT").unwrap();
            assert!(read(db,"projectA","nativeA").unwrap().is_none());
            assert!(read(db,"projectA","nativeB").unwrap().is_none());
            assert!(current_relationship(db,"projectA","nativeA").unwrap().is_none());
            assert!(has_unreleased_seat_claim(db,"projectA","seatA","incA").unwrap());
            db.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED' WHERE session_id='nativeA'").unwrap();
            assert!(has_unreleased_seat_claim(db,"projectA","seatA","incA").unwrap());
            db.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED' WHERE session_id='nativeB'").unwrap();
            assert!(!has_unreleased_seat_claim(db,"projectA","seatA","incA").unwrap());
        });
    }

    #[test]
    fn current_relation_separates_selected_instance_and_session_generation() {
        with_product_db(|db| {
            legacy(db,"sideA","COMMITTED","4",true);
            assert!(current_relationship(db,"projectA","sideA").unwrap().is_none());
            db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceB','codex','refB','identityB','sha256:fixture','fixture','INSTALLED','LOGGED_OUT',1)").unwrap();
            let binding=SessionBinding {domain_id:"projectA".into(),session_id:"sideA".into(),
                seat_id:"seatA".into(),seat_incarnation:"incA".into(),
                seat_authorization_generation:1,selected_instance_id:"instanceA".into(),
                provenance:Provenance::NativeV2};
            db.execute("BEGIN IMMEDIATE").unwrap();
            select_native_in_transaction(db,&binding).unwrap();
            insert_native_in_transaction(db,&binding).unwrap();
            db.execute("COMMIT").unwrap();
            let current=current_relationship(db,"projectA","sideA").unwrap().unwrap();
            assert!(current.native_v2);
            assert_eq!(current.seat_authorization_generation,1);
            assert_eq!(current.session_generation,"4");
            assert_eq!(current.instance_id,"instanceA");
            // Neither metadata reading nor the V2 record creates A purpose.
            assert!(crate::store::ledger::read_registered_session(db,"sideA").unwrap().is_none());
            db.execute("UPDATE main.gogoke_v37_seats SET instance_id='instanceB' WHERE seat_id='seatA'").unwrap();
            assert!(current_relationship(db,"projectA","sideA").unwrap().is_none());
            db.execute("UPDATE main.gogoke_v37_seats SET instance_id='instanceA' WHERE seat_id='seatA'").unwrap();
            db.execute("UPDATE main.gogoke_v37_seats SET generation=2 WHERE seat_id='seatA'").unwrap();
            assert!(current_relationship(db,"projectA","sideA").unwrap().is_none());
            db.execute("UPDATE main.gogoke_v37_seats SET generation=1 WHERE seat_id='seatA'").unwrap();
            db.execute("UPDATE main.gogoke_v37_h_owner_binding SET state='REVOKED' WHERE owner_id='sideA'").unwrap();
            assert!(current_relationship(db,"projectA","sideA").unwrap().is_none());
            assert_eq!(read(db,"projectA","sideA").unwrap(),Some(binding));
        });
    }

    #[test]
    fn seat_action_predicate_counts_unreleased_old_and_unprojected_claims() {
        with_product_db(|db| {
            legacy(db,"oldA","STOPPED","1",true);
            db.execute("UPDATE main.gogoke_v37_seats SET state='IDLE',generation=99 WHERE seat_id='seatA'").unwrap();
            db.execute("INSERT INTO main.gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES('projectA','seatA','fixture','{}')").unwrap();
            let facts=crate::store::seat::list_page_facts(db,"projectA").unwrap();
            assert!(!facts.seats[0].allowed.tune);
            assert_eq!(facts.seats[0].allowed.locked_reason,Some("H_PENDING_OR_UNRESOLVED"));
            db.execute("DELETE FROM main.gogoke_v37_h_seat_binding WHERE session_id='oldA'").unwrap();
            assert!(!crate::store::seat::list_page_facts(db,"projectA").unwrap().seats[0].allowed.change_instance);
            // This is a metadata predicate fixture, not a StopFact/release
            // lifecycle test. Actual release still requires original custody.
            db.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED' WHERE session_id='oldA'").unwrap();
            assert!(crate::store::seat::list_page_facts(db,"projectA").unwrap().seats[0].allowed.tune);
        });
    }

    #[test]
    fn rejects_own_sql_drift_shadow_trigger_and_index() {
        with_product_db(|db| {
            db.execute("CREATE TEMP TABLE gogoke_v37_session_binding_v2_shadow(x TEXT)").unwrap();
            assert!(matches!(initialize_schema(db),Err(BindingError::Drift)));
            db.execute("DROP TABLE temp.gogoke_v37_session_binding_v2_shadow").unwrap();
            db.execute("CREATE TEMP TRIGGER foreign_name AFTER INSERT ON main.gogoke_v37_session_binding_v2 BEGIN SELECT 1; END").unwrap();
            assert!(matches!(initialize_schema(db),Err(BindingError::Drift)));
            db.execute("DROP TRIGGER temp.foreign_name").unwrap();
            db.execute("CREATE INDEX extra_binding_index ON gogoke_v37_session_binding_v2(seat_id)").unwrap();
            assert!(matches!(initialize_schema(db),Err(BindingError::Drift)));
            db.execute("DROP INDEX extra_binding_index").unwrap();
            db.execute("ALTER TABLE gogoke_v37_session_binding_v2 ADD COLUMN drift TEXT").unwrap();
            assert!(matches!(initialize_schema(db),Err(BindingError::Drift)));
        });
    }
}
