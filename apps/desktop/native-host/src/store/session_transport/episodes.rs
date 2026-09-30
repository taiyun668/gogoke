//! Durable process identity for every native H generation. The claim is only
//! the current admission pointer; old A and C evidence is owned by these rows.
use crate::store::atomic::{AtomicError, Statement};
use crate::store::same_open::VerifiedDatabaseConnection;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) const PROCESS_SCHEMA: &str = "CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,session_id TEXT NOT NULL,generation TEXT NOT NULL,old_generation TEXT,raw_hex TEXT NOT NULL,previous_revision INTEGER NOT NULL,result_revision INTEGER,process_operation_id TEXT UNIQUE,instance_id TEXT NOT NULL,home_id TEXT NOT NULL,binding_id TEXT NOT NULL,seat_id TEXT,seat_incarnation TEXT,stop_request_id TEXT,phase TEXT NOT NULL CHECK(phase IN ('INTENT','PREPARED','ACTIVE','UNKNOWN','STOPPED','FAILED')),stop_fact_id TEXT,PRIMARY KEY(domain_id,request_id)) STRICT";
pub(super) const GENERATION_SCHEMA: &str = "CREATE TABLE gogoke_v37_h_generation(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,generation TEXT NOT NULL,request_id TEXT NOT NULL,process_operation_id TEXT NOT NULL UNIQUE,PRIMARY KEY(domain_id,session_id,generation),UNIQUE(domain_id,request_id)) STRICT";

pub(crate) fn record_initial(connection: &VerifiedDatabaseConnection<'_>, domain: &str,
    session: &str, request_id: &str, operation_id: &str) -> Result<(), AtomicError> {
    let process = Statement::prepare(connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id)
         SELECT a.domain_id,o.request_id,a.session_id,a.generation,NULL,o.raw_hex,
                o.previous_revision,o.revision,
                a.process_operation_id,a.instance_id,a.home_id,a.binding_id,
                s.seat_id,s.seat_incarnation,'PREPARED',NULL
           FROM main.gogoke_v37_h_claim a
           JOIN main.gogoke_v37_h_operation o ON o.domain_id=a.domain_id
             AND o.session_id=a.session_id AND o.operation='open'
           JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=a.domain_id
             AND s.session_id=a.session_id AND s.generation=a.generation
          WHERE a.domain_id=?1 AND a.session_id=?2 AND o.request_id=?3
            AND a.process_operation_id=?4 AND a.state='COMMITTED'")?;
    for (index,value) in [domain,session,request_id,operation_id].iter().enumerate() {
        process.bind_text((index+1) as i32,value)?;
    }
    process.step_done()?;
    let changed = Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)? != "1" {
        return Err(AtomicError::OperationConflict);
    }
    let generation = Statement::prepare(connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id)
         SELECT domain_id,session_id,generation,request_id,process_operation_id
           FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND request_id=?2")?;
    generation.bind_text(1,domain)?;
    generation.bind_text(2,request_id)?;
    generation.step_done()?;
    Ok(())
}

pub(crate) fn mark_active(connection: &VerifiedDatabaseConnection<'_>, operation: &str)
    -> Result<(), AtomicError> {
    let statement=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_process_episode SET phase='ACTIVE',
          result_revision=(SELECT o.revision FROM main.gogoke_v37_h_operation o
            WHERE o.domain_id=gogoke_v37_h_process_episode.domain_id
              AND o.request_id=gogoke_v37_h_process_episode.request_id)
          WHERE process_operation_id=?1 AND phase='PREPARED'")?;
    statement.bind_text(1,operation)?;
    statement.step_done()?;
    let changed=Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {
        return Err(AtomicError::OperationConflict);
    }
    Ok(())
}

pub(crate) fn mark_stopped(connection: &VerifiedDatabaseConnection<'_>, operation: &str,
    proof: &str) -> Result<(), AtomicError> {
    let statement=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_process_episode SET phase='STOPPED',stop_fact_id=?2
          WHERE process_operation_id=?1 AND phase IN ('ACTIVE','UNKNOWN','PREPARED')
            AND stop_fact_id IS NULL")?;
    statement.bind_text(1,operation)?;
    statement.bind_text(2,proof)?;
    statement.step_done()?;
    let changed=Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {
        return Err(AtomicError::OperationConflict);
    }
    Ok(())
}

/// Intent exists before a candidate process is prepared. The current claim
/// remains STOPPED until the provider's original resume response is observed.
pub(crate) fn begin_resume(connection: &VerifiedDatabaseConnection<'_>,
    domain: &str, session: &str, request_id: &str, raw_bytes: &[u8],
    old_generation: &str, new_generation: &str, expected_revision: i64,
    home_id: &str, binding_id: &str) -> Result<(), AtomicError> {
    if let Some(change)=super::generation_change::active_for_session(connection,domain,session)? {
        if change.request_id!=request_id || change.stage!="OLD_STOPPED"
            || change.owner_stop_request_id.is_some() {
            return Err(AtomicError::OperationConflict);
        }
    }
    // A stopped-but-cancelled candidate has already occupied the next
    // physical generation's home. Do not launch another process into it.
    let cancelled=Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_generation_change x
           JOIN main.gogoke_v37_h_process_episode e
             ON e.domain_id=x.domain_id AND e.request_id=x.request_id
          WHERE x.domain_id=?1 AND x.session_id=?2 AND x.old_generation=?3
            AND x.stage='CANCELLED' AND e.process_operation_id IS NOT NULL LIMIT 1")?;
    cancelled.bind_text(1,domain)?;cancelled.bind_text(2,session)?;
    cancelled.bind_text(3,old_generation)?;
    if cancelled.step_row()? {return Err(AtomicError::OperationConflict);}
    let unresolved=Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_process_episode
          WHERE domain_id=?1 AND session_id=?2 AND old_generation IS NOT NULL
            AND phase IN ('INTENT','PREPARED','ACTIVE','UNKNOWN') LIMIT 1")?;
    unresolved.bind_text(1,domain)?;unresolved.bind_text(2,session)?;
    if unresolved.step_row()? { return Err(AtomicError::OperationConflict); }
    let insert=Statement::prepare(connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_h_process_episode
         (domain_id,request_id,session_id,generation,old_generation,raw_hex,
          previous_revision,result_revision,
          process_operation_id,instance_id,home_id,binding_id,seat_id,
          seat_incarnation,phase,stop_fact_id)
         SELECT a.domain_id,?3,a.session_id,?4,a.generation,?5,?9,NULL,NULL,
                a.instance_id,?6,?7,s.seat_id,s.seat_incarnation,'INTENT',NULL
           FROM main.gogoke_v37_h_claim a
           JOIN main.gogoke_v37_h_seat_binding s
             ON s.domain_id=a.domain_id AND s.session_id=a.session_id
             AND s.generation=a.generation
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
             AND c.generation=a.generation AND c.state='STOPPED'
             AND c.stop_proof_hash=a.stop_fact_id
           JOIN main.gogoke_v37_h_owner_binding b
             ON b.binding_id=?7 AND b.domain_id=a.domain_id
             AND b.instance_id=a.instance_id AND b.generation=?4
             AND b.kind='SESSION' AND b.owner_id=a.session_id AND b.state='ACTIVE'
           JOIN main.gogoke_v37_instance_homes h
             ON h.home_id=?6 AND h.instance_id=a.instance_id
             AND h.domain_id=a.domain_id AND h.generation=?4
             AND h.kind='SESSION' AND h.owner_id=a.session_id AND h.state='ACTIVE'
          WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?8
            AND a.revision=?9 AND a.state='STOPPED' AND a.stop_fact_id IS NOT NULL")?;
    let original_hex=hex(raw_bytes);
    for (index,value) in [domain,session,request_id,new_generation,
        original_hex.as_str(),home_id,binding_id,old_generation].iter().enumerate() {
        insert.bind_text((index+1) as i32,value)?;
    }
    insert.bind_i64(9,expected_revision)?;
    insert.step_done()?;
    let changed=Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {
        return Err(AtomicError::OperationConflict);
    }
    Ok(())
}

pub(crate) fn attach_resume_process(connection: &VerifiedDatabaseConnection<'_>,
    domain: &str, request_id: &str, operation_id: &str) -> Result<(), AtomicError> {
    let update=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_process_episode AS a SET process_operation_id=?3,phase='PREPARED'
          WHERE domain_id=?1 AND request_id=?2 AND phase='INTENT'
            AND process_operation_id IS NULL
            AND EXISTS(SELECT 1 FROM main.gogoke_coordination_process_custody c
                        WHERE c.operation_id=?3 AND c.domain_id=a.domain_id
                          AND c.generation=a.generation AND c.state='PREPARED')")?;
    update.bind_text(1,domain)?;update.bind_text(2,request_id)?;
    update.bind_text(3,operation_id)?;update.step_done()?;
    let changed=Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {
        return Err(AtomicError::OperationConflict);
    }
    Ok(())
}

pub(crate) fn mark_resume_unknown(connection: &VerifiedDatabaseConnection<'_>,
    domain: &str, session: &str, request_id: &str,
    operation_id: &str) -> Result<i64,AtomicError> {
    let row=Statement::prepare(connection.as_ptr(),
        "SELECT e.old_generation,e.previous_revision,a.revision
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
             AND a.session_id=e.session_id AND a.generation=e.old_generation
             AND a.state='STOPPED'
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.process_operation_id=?4 AND e.phase='PREPARED'")?;
    for (index,value) in [domain,session,request_id,operation_id].iter().enumerate() {
        row.bind_text((index+1) as i32,value)?;
    }
    if !row.step_row()? {return Err(AtomicError::OperationConflict);}
    let old=row.column_text(0)?;
    let before=row.column_text(1)?.parse::<i64>().map_err(|_|AtomicError::OperationConflict)?;
    let current=row.column_text(2)?.parse::<i64>().map_err(|_|AtomicError::OperationConflict)?;
    if current!=before || row.step_row()? {return Err(AtomicError::OperationConflict);}
    drop(row);
    let compound=super::generation_change::read(connection,domain,request_id)?;
    let next=if let Some(c)=&compound {
        if c.stage!="OLD_STOPPED" || c.old_generation!=old
            || c.owner_stop_request_id.is_some() {return Err(AtomicError::OperationConflict);}
        if let Some(already)=c.unknown_revision {
            if current!=already {return Err(AtomicError::OperationConflict);}
            current
        } else {
            current.checked_add(1).ok_or(AtomicError::OperationConflict)?
        }
    } else {current.checked_add(1).ok_or(AtomicError::OperationConflict)?};
    let claim=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_claim SET revision=?5
          WHERE domain_id=?1 AND session_id=?2 AND generation=?3
            AND state='STOPPED' AND revision=?4")?;
    claim.bind_text(1,domain)?;claim.bind_text(2,session)?;claim.bind_text(3,&old)?;
    claim.bind_i64(4,current)?;claim.bind_i64(5,next)?;claim.step_done()?;
    let episode=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_process_episode SET phase='UNKNOWN',result_revision=?3
          WHERE domain_id=?1 AND request_id=?2 AND phase='PREPARED'")?;
    episode.bind_text(1,domain)?;episode.bind_text(2,request_id)?;
    episode.bind_i64(3,next)?;episode.step_done()?;
    if compound.is_some_and(|c|c.unknown_revision.is_none()) {
        super::generation_change::note_candidate_unknown(connection,domain,request_id,next)?;
    }
    Ok(next)
}

pub(crate) fn promote_resume(connection: &VerifiedDatabaseConnection<'_>,
    domain: &str, session: &str, request_id: &str, operation_id: &str,
    expected_revision: i64) -> Result<i64, AtomicError> {
    if let Some(change)=super::generation_change::active_for_session(connection,domain,session)? {
        if change.request_id!=request_id || change.stage!="OLD_STOPPED"
            || change.owner_stop_request_id.is_some() {
            return Err(AtomicError::OperationConflict);
        }
    }
    let candidate=Statement::prepare(connection.as_ptr(),
        "SELECT e.generation,e.old_generation,e.home_id,e.binding_id,e.seat_id,
                e.seat_incarnation,e.instance_id
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
             AND a.session_id=e.session_id AND a.generation=e.old_generation
             AND a.instance_id=e.instance_id AND a.state='STOPPED'
            AND a.revision=?5 AND a.stop_fact_id IS NOT NULL
           JOIN main.gogoke_coordination_process_custody oldc
             ON oldc.operation_id=a.process_operation_id AND oldc.domain_id=a.domain_id
             AND oldc.generation=a.generation AND oldc.state='STOPPED'
             AND oldc.stop_proof_hash=a.stop_fact_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation AND c.state IN ('ACTIVE','UNKNOWN')
             AND c.binary_digest_sha256=oldc.binary_digest_sha256
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
             AND i.driver_id='codex' AND i.version='0.149.0'
             AND i.install_state='INSTALLED' AND i.login_state='LOGGED_IN'
             AND i.program_digest=c.binary_digest_sha256
           JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=e.binding_id
             AND b.instance_id=e.instance_id AND b.domain_id=e.domain_id
             AND b.owner_id=e.session_id AND b.generation=e.generation
             AND b.kind='SESSION' AND b.state='ACTIVE'
           JOIN main.gogoke_v37_instance_homes h ON h.home_id=e.home_id
             AND h.instance_id=e.instance_id AND h.domain_id=e.domain_id
             AND h.owner_id=e.session_id AND h.generation=e.generation
             AND h.kind='SESSION' AND h.state='ACTIVE'
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.process_operation_id=?4 AND e.phase IN ('PREPARED','UNKNOWN')")?;
    candidate.bind_text(1,domain)?;candidate.bind_text(2,session)?;
    candidate.bind_text(3,request_id)?;candidate.bind_text(4,operation_id)?;
    candidate.bind_i64(5,expected_revision)?;
    if !candidate.step_row()? {return Err(AtomicError::OperationConflict);}
    let mut row=Vec::new();
    for index in 0..7 {row.push(candidate.column_text(index)?);}
    if candidate.step_row()? {return Err(AtomicError::OperationConflict);}
    drop(candidate);
    let (new_generation,old_generation,home_id,binding_id,seat_id,incarnation,instance_id)=
        (&row[0],&row[1],&row[2],&row[3],&row[4],&row[5],&row[6]);
    let expected_new=old_generation.parse::<i64>().ok()
        .and_then(|value|value.checked_add(1))
        .ok_or(AtomicError::OperationConflict)?;
    if new_generation!=&expected_new.to_string() {return Err(AtomicError::OperationConflict);}
    let step_id=format!("{operation_id}-thread-resume");
    let observed=Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id
             AND r.generation=s.generation
          WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
            AND s.open_request_id=?4 AND s.generation=?5 AND s.step_id=?6
            AND s.phase='OBSERVED' AND r.state='NO_EVENT'")?;
    for (index,value) in [domain,session,operation_id,request_id,
        new_generation.as_str(),step_id.as_str()].iter().enumerate() {
        observed.bind_text((index+1) as i32,value)?;
    }
    if !observed.step_row()? || observed.step_row()? {return Err(AtomicError::OperationConflict);}
    drop(observed);
    // The original A/RPC response settles only this already-held candidate.
    // A separate process start remains forbidden while the request was UNKNOWN.
    let settle=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_coordination_process_custody SET state='ACTIVE'
          WHERE operation_id=?1 AND domain_id=?2 AND generation=?3 AND state='UNKNOWN'")?;
    settle.bind_text(1,operation_id)?;settle.bind_text(2,domain)?;
    settle.bind_text(3,new_generation)?;settle.step_done()?;
    let seat=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_seats SET generation=?1,revision=revision+1
          WHERE domain_id=?2 AND seat_id=?3 AND incarnation=?4
            AND instance_id=?5 AND CAST(generation AS TEXT)=?6 AND state='BUSY'")?;
    seat.bind_i64(1,expected_new)?;
    for (index,value) in [domain,seat_id.as_str(),incarnation.as_str(),instance_id.as_str(),
        old_generation.as_str()].iter().enumerate() {
        seat.bind_text((index+2) as i32,value)?;
    }
    seat.step_done()?;
    let changed=Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {return Err(AtomicError::OperationConflict);}
    let binding=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_seat_binding SET generation=?1
          WHERE domain_id=?2 AND session_id=?3 AND seat_id=?4
            AND seat_incarnation=?5 AND generation=?6")?;
    for (index,value) in [new_generation.as_str(),domain,session,seat_id.as_str(),
        incarnation.as_str(),old_generation.as_str()].iter().enumerate() {
        binding.bind_text((index+1) as i32,value)?;
    }
    binding.step_done()?;
    let claim=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_claim SET generation=?1,home_id=?2,binding_id=?3,
          process_operation_id=?4,stop_fact_id=NULL,state='COMMITTED',revision=revision+1
          WHERE domain_id=?5 AND session_id=?6 AND generation=?7
            AND state='STOPPED' AND revision=?8 AND stop_fact_id IS NOT NULL")?;
    for (index,value) in [new_generation.as_str(),home_id.as_str(),binding_id.as_str(),
        operation_id,domain,session,old_generation.as_str()].iter().enumerate() {
        claim.bind_text((index+1) as i32,value)?;
    }
    claim.bind_i64(8,expected_revision)?;claim.step_done()?;
    let changed=Statement::prepare(connection.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {return Err(AtomicError::OperationConflict);}
    let generation=Statement::prepare(connection.as_ptr(),
        "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id)
         VALUES(?1,?2,?3,?4,?5)")?;
    for (index,value) in [domain,session,new_generation.as_str(),request_id,
        operation_id].iter().enumerate() {
        generation.bind_text((index+1) as i32,value)?;
    }
    generation.step_done()?;
    let next=expected_revision.checked_add(1).ok_or(AtomicError::OperationConflict)?;
    let update=Statement::prepare(connection.as_ptr(),
        "UPDATE main.gogoke_v37_h_process_episode SET phase='ACTIVE',
          previous_revision=CASE WHEN phase='UNKNOWN' THEN result_revision ELSE previous_revision END,
          result_revision=?3
          WHERE domain_id=?1 AND request_id=?2 AND phase IN ('PREPARED','UNKNOWN')")?;
    update.bind_text(1,domain)?;update.bind_text(2,request_id)?;
    update.bind_i64(3,next)?;update.step_done()?;
    Ok(next)
}

pub(super) fn backfill(connection: &VerifiedDatabaseConnection<'_>) -> Result<(), AtomicError> {
    // Only a process already attached to an H claim has a historical episode.
    // All original request bytes and native custody rows stay in their tables.
    let query = Statement::prepare(connection.as_ptr(),
        "SELECT a.domain_id,a.session_id,a.generation,a.process_operation_id,a.instance_id,
                a.home_id,a.binding_id,a.state,COALESCE(a.stop_fact_id,''),
                COALESCE(s.seat_id,''),COALESCE(s.seat_incarnation,''),
                o.request_id,o.raw_hex
           FROM main.gogoke_v37_h_claim a
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
             AND c.generation=a.generation
           LEFT JOIN main.gogoke_v37_h_seat_binding s
             ON s.domain_id=a.domain_id AND s.session_id=a.session_id
           JOIN main.gogoke_v37_h_operation o
             ON o.domain_id=a.domain_id AND o.session_id=a.session_id AND o.operation='open'
          WHERE a.process_operation_id IS NOT NULL
            AND (a.state NOT IN ('STOPPED','RELEASED')
              OR (c.state='STOPPED' AND c.stop_proof_hash=a.stop_fact_id
                  AND a.stop_fact_id IS NOT NULL))")?;
    while query.step_row()? {
        let mut fields = Vec::new();
        for column in 0..13 { fields.push(query.column_text(column)?); }
        let stop = if fields[8].is_empty() { None } else { Some(fields[8].as_str()) };
        let phase = if fields[7] == "STOPPED" || fields[7] == "RELEASED" { "STOPPED" }
            else if fields[7] == "UNKNOWN" { "UNKNOWN" } else { "ACTIVE" };
        let insert = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES(?1,?2,?3,?4,NULL,?5,?14,?15,?6,?7,?8,?9,?10,?11,?12,?13)")?;
        for (index,value) in [&fields[0],&fields[11],&fields[1],&fields[2],&fields[12],
            &fields[3],&fields[4],&fields[5],&fields[6],&fields[9],&fields[10]].iter().enumerate() {
            insert.bind_text((index+1) as i32,value)?;
        }
        insert.bind_text(12,phase)?;
        if let Some(value) = stop { insert.bind_text(13,value)?; }
        let revision=Statement::prepare(connection.as_ptr(),
            "SELECT previous_revision,revision FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2")?;
        revision.bind_text(1,&fields[0])?;revision.bind_text(2,&fields[11])?;
        if !revision.step_row()? { return Err(AtomicError::OperationConflict); }
        insert.bind_i64(14,revision.column_text(0)?.parse().map_err(|_|AtomicError::OperationConflict)?)?;
        insert.bind_i64(15,revision.column_text(1)?.parse().map_err(|_|AtomicError::OperationConflict)?)?;
        insert.step_done()?;
        let generation = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES(?1,?2,?3,?4,?5)")?;
        for (index,value) in [&fields[0],&fields[1],&fields[2],&fields[11],&fields[3]].iter().enumerate() {
            generation.bind_text((index+1) as i32,value)?;
        }
        generation.step_done()?;
    }
    let ambiguous=Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_operation WHERE operation='stop'
          GROUP BY domain_id,session_id HAVING count(*)>1 LIMIT 1")?;
    if ambiguous.step_row()? {return Err(AtomicError::OperationConflict);}
    let stops=Statement::prepare(connection.as_ptr(),
      "UPDATE main.gogoke_v37_h_process_episode AS e
      SET stop_request_id=(SELECT o.request_id FROM main.gogoke_v37_h_operation o
        WHERE o.domain_id=e.domain_id AND o.session_id=e.session_id
          AND o.operation='stop' LIMIT 1)
      WHERE e.old_generation IS NULL")?;
    stops.step_done()?;
    let count=Statement::prepare(connection.as_ptr(),
        "SELECT (SELECT count(*) FROM main.gogoke_v37_h_claim
                   WHERE process_operation_id IS NOT NULL),
                (SELECT count(*) FROM main.gogoke_v37_h_process_episode),
                (SELECT count(*) FROM main.gogoke_v37_h_generation)")?;
    if !count.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    let claims=count.column_text(0)?;
    if claims!=count.column_text(1)? || claims!=count.column_text(2)?
        || count.step_row()? {return Err(AtomicError::OperationConflict);}
    Ok(())
}
