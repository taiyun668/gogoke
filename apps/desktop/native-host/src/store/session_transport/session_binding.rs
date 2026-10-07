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

/// Independent of admission's historical 5/6/8/9-table upgrade chain.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<(), BindingError> {
    if schema_state(db)?.is_some() { return Ok(()); }
    db.execute("BEGIN IMMEDIATE")?;
    let outcome = (|| {
        if schema_state(db)?.is_some() { return Err(BindingError::Drift); }
        db.execute(SCHEMA)?;
        if schema_state(db)?.is_none() { return Err(BindingError::Drift); }
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

/// Requires the caller's open product transaction and original native E seat
/// authorization plus User/native request proof. No authority is minted here.
/// Exact same-session replay is idempotent; a changed field is a conflict.
pub(crate) fn insert_native_in_transaction(db: &VerifiedDatabaseConnection<'_>, binding: &SessionBinding)
    -> Result<(), BindingError> {
    if binding.provenance != Provenance::NativeV2 { return Err(BindingError::Invalid("provenance")); }
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

    fn legacy(db: &VerifiedDatabaseConnection<'_>, session: &str, state: &str,
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
        if state == "STOPPED" || state == "UNKNOWN" {
            let operation_id = format!("open{session}");
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA',?1,'00ff','open',?2,'APPLIED',6,7)").unwrap();
            insert.bind_text(1,&operation_id).unwrap(); insert.bind_text(2,session).unwrap();
            insert.step_done().unwrap(); drop(insert);
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,instance_id,home_id,binding_id,phase,stop_fact_id) VALUES('projectA',?1,?2,?3,'cafebabe',6,'instanceA',?4,?5,?6,?7)").unwrap();
            let stop = if state == "STOPPED" { "stopFactA" } else { "" };
            for (i,v) in [operation_id.as_str(),session,generation,home_id.as_str(),
                binding_id.as_str(),state,stop].iter().enumerate() {
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
