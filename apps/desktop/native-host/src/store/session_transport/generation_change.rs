//! One original K-SESSION compact/renew request owns the native transition.
//! This is a reference ledger: provider frames remain in A and process facts
//! remain in custody and the immutable H episodes.
use crate::store::atomic::{AtomicError, Statement};
use crate::store::same_open::VerifiedDatabaseConnection;

pub(super) const SCHEMA: &str = "CREATE TABLE gogoke_v37_h_generation_change(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,raw_hex TEXT NOT NULL,operation TEXT NOT NULL CHECK(operation IN ('compact','renew-session')),session_id TEXT NOT NULL,old_generation TEXT NOT NULL,old_process_operation_id TEXT NOT NULL,old_ticket TEXT NOT NULL,old_nonce TEXT NOT NULL,thread_id TEXT NOT NULL,seat_id TEXT NOT NULL,previous_revision INTEGER NOT NULL,source_watermark INTEGER NOT NULL,stage TEXT NOT NULL CHECK(stage IN ('INTENT','ACKED','ITEM_OBSERVED','OLD_STOPPED','APPLIED','CANCELLED','UNSUPPORTED')),unknown_revision INTEGER,ack_step_id TEXT,item_source_epoch TEXT,item_source_cursor TEXT,item_id TEXT,owner_stop_request_id TEXT,result_revision INTEGER,original_error TEXT,CHECK((item_source_epoch IS NULL AND item_source_cursor IS NULL AND item_id IS NULL) OR (item_source_epoch IS NOT NULL AND item_source_cursor IS NOT NULL AND item_id IS NOT NULL)),PRIMARY KEY(domain_id,request_id)) STRICT";

#[derive(Clone, Debug)]
pub(crate) struct Change {
    pub(crate) request_id: String,
    pub(crate) raw_hex: String,
    pub(crate) operation: String,
    pub(crate) session_id: String,
    pub(crate) old_generation: String,
    pub(crate) old_operation: String,
    pub(crate) old_ticket: String,
    pub(crate) old_nonce: String,
    pub(crate) thread_id: String,
    pub(crate) seat_id: String,
    pub(crate) previous_revision: i64,
    pub(crate) source_watermark: i64,
    pub(crate) stage: String,
    pub(crate) unknown_revision: Option<i64>,
    pub(crate) ack_step_id: Option<String>,
    pub(crate) item_source_epoch: Option<String>,
    pub(crate) item_source_cursor: Option<String>,
    pub(crate) item_id: Option<String>,
    pub(crate) owner_stop_request_id: Option<String>,
    pub(crate) result_revision: Option<i64>,
    pub(crate) original_error: Option<String>,
}

fn nullable(row: &Statement, index: i32) -> Result<Option<String>, AtomicError> {
    let value=row.column_text(index)?;
    Ok(if value.is_empty() {None} else {Some(value)})
}

pub(crate) fn read(db:&VerifiedDatabaseConnection<'_>,domain:&str,request_id:&str)
    ->Result<Option<Change>,AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT request_id,raw_hex,operation,session_id,old_generation,
                old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,
                previous_revision,source_watermark,stage,COALESCE(unknown_revision,''),COALESCE(ack_step_id,''),
                COALESCE(item_source_epoch,''),COALESCE(item_source_cursor,''),
                COALESCE(item_id,''),COALESCE(owner_stop_request_id,''),COALESCE(result_revision,''),
                COALESCE(original_error,'')
           FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND request_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,request_id)?;
    if !q.step_row()? {return Ok(None);}
    let unknown=nullable(&q,13)?.map(|v|v.parse::<i64>().map_err(|_|AtomicError::OperationConflict)).transpose()?;
    let result=nullable(&q,19)?.map(|v|v.parse::<i64>().map_err(|_|AtomicError::OperationConflict)).transpose()?;
    let change=Change {request_id:q.column_text(0)?,raw_hex:q.column_text(1)?,
        operation:q.column_text(2)?,session_id:q.column_text(3)?,
        old_generation:q.column_text(4)?,old_operation:q.column_text(5)?,
        old_ticket:q.column_text(6)?,old_nonce:q.column_text(7)?,
        thread_id:q.column_text(8)?,seat_id:q.column_text(9)?,
        previous_revision:q.column_text(10)?.parse().map_err(|_|AtomicError::OperationConflict)?,
        source_watermark:q.column_text(11)?.parse().map_err(|_|AtomicError::OperationConflict)?,
        stage:q.column_text(12)?,unknown_revision:unknown,
        ack_step_id:nullable(&q,14)?,item_source_epoch:nullable(&q,15)?,
        item_source_cursor:nullable(&q,16)?,item_id:nullable(&q,17)?,
        owner_stop_request_id:nullable(&q,18)?,result_revision:result,
        original_error:nullable(&q,20)?};
    if q.step_row()? {return Err(AtomicError::OperationConflict);}
    Ok(Some(change))
}

pub(crate) fn active_for_session(db:&VerifiedDatabaseConnection<'_>,domain:&str,session:&str)
    ->Result<Option<Change>,AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT request_id FROM main.gogoke_v37_h_generation_change
          WHERE domain_id=?1 AND session_id=?2 AND stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED')")?;
    q.bind_text(1,domain)?;q.bind_text(2,session)?;
    if !q.step_row()? {return Ok(None);}
    let id=q.column_text(0)?;
    if q.step_row()? {return Err(AtomicError::OperationConflict);}
    read(db,domain,&id)
}

pub(crate) fn begin(db:&VerifiedDatabaseConnection<'_>, domain:&str,request_id:&str,
    raw_bytes:&[u8],operation:&str,session:&str,generation:&str,
    process:&str,ticket:&str,nonce:&str,thread:&str,seat:&str,revision:i64,watermark:i64)
    ->Result<(),AtomicError> {
    if !matches!(operation,"compact"|"renew-session") || active_for_session(db,domain,session)?.is_some() {
        return Err(AtomicError::OperationConflict);
    }
    super::session_binding::authorization_generation(db,domain,session)
        .map_err(|error|AtomicError::DurabilityContractFailed(
            format!("generation change relationship: {error:?}")))?;
    let raw=raw_bytes.iter().map(|b|format!("{b:02x}")).collect::<String>();
    let q=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_h_generation_change(domain_id,request_id,raw_hex,
           operation,session_id,old_generation,old_process_operation_id,old_ticket,
           old_nonce,thread_id,seat_id,previous_revision,source_watermark,stage)
         SELECT ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'INTENT'
          WHERE EXISTS(SELECT 1 FROM main.gogoke_v37_h_claim a
            JOIN main.gogoke_v37_h_process_episode e
              ON e.domain_id=a.domain_id AND e.session_id=a.session_id
              AND e.generation=a.generation AND e.process_operation_id=a.process_operation_id
            JOIN main.gogoke_coordination_process_custody c
              ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
              AND c.generation=e.generation
            JOIN main.gogoke_v37_effective_seat b
              ON b.domain_id=a.domain_id AND b.session_id=a.session_id
              AND b.generation=a.generation
             JOIN main.gogoke_v37_seats s ON s.domain_id=b.domain_id
               AND s.seat_id=b.seat_id AND s.incarnation=b.seat_incarnation
               AND s.generation=b.seat_authorization_generation
               AND s.instance_id=b.selected_instance_id AND s.state='BUSY'
            WHERE a.domain_id=?1 AND a.session_id=?5 AND a.generation=?6
              AND a.process_operation_id=?7 AND a.revision=?12
              AND a.state='COMMITTED' AND e.phase='ACTIVE'
              AND c.state='ACTIVE' AND c.ticket=?8 AND c.custodian_nonce=?9
               AND b.seat_id=?11 AND b.selected_instance_id=a.instance_id)")?;
    for (index,value) in [domain,request_id,raw.as_str(),operation,session,generation,
        process,ticket,nonce,thread,seat].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    q.bind_i64(12,revision)?;q.bind_i64(13,watermark)?;q.step_done()?;
    let changed=Statement::prepare(db.as_ptr(),"SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)?!="1" {return Err(AtomicError::OperationConflict);}
    Ok(())
}

pub(crate) fn mark_ack(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,step:&str)
    ->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET stage='ACKED',ack_step_id=?3
          WHERE domain_id=?1 AND request_id=?2 AND stage='INTENT'
            AND owner_stop_request_id IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_text(3,step)?;q.step_done()?;
    changed_one(db)
}

pub(crate) fn mark_item(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,
    epoch:&str,cursor:&str,item:&str)->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET stage='ITEM_OBSERVED',
          item_source_epoch=?3,item_source_cursor=?4,item_id=?5
          WHERE domain_id=?1 AND request_id=?2 AND stage='ACKED'
            AND owner_stop_request_id IS NULL")?;
    for (index,value) in [domain,id,epoch,cursor,item].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    q.step_done()?;changed_one(db)
}

pub(crate) fn mark_old_stopped(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str)
    ->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET stage='OLD_STOPPED'
          WHERE domain_id=?1 AND request_id=?2 AND stage IN ('INTENT','ITEM_OBSERVED')
            AND owner_stop_request_id IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.step_done()?;changed_one(db)
}

pub(crate) fn mark_unknown(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str)
    ->Result<i64,AtomicError> {
    let change=read(db,domain,id)?.ok_or(AtomicError::OperationConflict)?;
    if let Some(revision)=change.unknown_revision {return Ok(revision);}
    if matches!(change.stage.as_str(),"APPLIED"|"CANCELLED"|"UNSUPPORTED") {
        return Err(AtomicError::OperationConflict);
    }
    let next=change.previous_revision.checked_add(1).ok_or(AtomicError::OperationConflict)?;
    let claim=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_claim SET revision=?4
          WHERE domain_id=?1 AND session_id=?2 AND generation=?3
            AND revision=?5 AND state IN ('COMMITTED','STOPPED')")?;
    claim.bind_text(1,domain)?;claim.bind_text(2,&change.session_id)?;
    claim.bind_text(3,&change.old_generation)?;claim.bind_i64(4,next)?;
    claim.bind_i64(5,change.previous_revision)?;claim.step_done()?;changed_one(db)?;
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET unknown_revision=?3
          WHERE domain_id=?1 AND request_id=?2 AND unknown_revision IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_i64(3,next)?;q.step_done()?;
    changed_one(db)?;Ok(next)
}

/// Called in the same transaction as a compound candidate's UNKNOWN write.
/// The candidate may have advanced the H revision already; this records that
/// one public UNKNOWN transition without a second increment.
pub(crate) fn note_candidate_unknown(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,
    revision:i64)->Result<(),AtomicError> {
    let c=read(db,domain,id)?.ok_or(AtomicError::OperationConflict)?;
    if c.stage!="OLD_STOPPED" || c.owner_stop_request_id.is_some()
        || revision!=c.previous_revision.checked_add(1).ok_or(AtomicError::OperationConflict)?
        || c.unknown_revision.is_some() {
        return Err(AtomicError::OperationConflict);
    }
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET unknown_revision=?3
          WHERE domain_id=?1 AND request_id=?2 AND unknown_revision IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_i64(3,revision)?;
    q.step_done()?;changed_one(db)
}

pub(crate) fn note_error(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,
    reason:&str)->Result<(),AtomicError> {
    if reason.is_empty() || reason.len()>4096 {return Err(AtomicError::OperationConflict);}
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET original_error=?3
          WHERE domain_id=?1 AND request_id=?2 AND original_error IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_text(3,reason)?;
    q.step_done()?;changed_one(db)
}

pub(crate) fn mark_applied(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,revision:i64)
    ->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET stage='APPLIED',result_revision=?3
          WHERE domain_id=?1 AND request_id=?2 AND stage='OLD_STOPPED'
            AND owner_stop_request_id IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_i64(3,revision)?;
    q.step_done()?;changed_one(db)
}

pub(crate) fn mark_unsupported(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,
    step_id:&str)->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET stage='UNSUPPORTED',ack_step_id=?3
          WHERE domain_id=?1 AND request_id=?2 AND stage='INTENT'
            AND unknown_revision IS NULL AND owner_stop_request_id IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_text(3,step_id)?;
    q.step_done()?;changed_one(db)
}

pub(crate) fn begin_owner_stop(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,stop_id:&str)
    ->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET owner_stop_request_id=?3
          WHERE domain_id=?1 AND request_id=?2 AND stage NOT IN ('APPLIED','CANCELLED')
            AND owner_stop_request_id IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_text(3,stop_id)?;
    q.step_done()?;changed_one(db)
}

pub(crate) fn cancel_for_owner_stop(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str,stop_id:&str)
    ->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_h_generation_change SET stage='CANCELLED'
          WHERE domain_id=?1 AND request_id=?2 AND owner_stop_request_id=?3
            AND stage NOT IN ('APPLIED','CANCELLED')")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;q.bind_text(3,stop_id)?;
    q.step_done()?;changed_one(db)
}

fn changed_one(db:&VerifiedDatabaseConnection<'_>)->Result<(),AtomicError> {
    let q=Statement::prepare(db.as_ptr(),"SELECT changes()")?;
    if !q.step_row()? || q.column_text(0)?!="1" {return Err(AtomicError::OperationConflict);}
    Ok(())
}
