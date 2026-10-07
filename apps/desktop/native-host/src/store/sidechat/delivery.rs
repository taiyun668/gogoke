//! D's durable, directed side-chat delivery intentions and source-backed read.
//! C owns queueing and H owns physical sends. A timeout never grants a retry.
use super::*;
use crate::store::inbox;
use crate::store::session_transport::{decode_request,read_stdin_journal,JournalState,StdinJournalKey};

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum Direction { SideToLead, LeadToSide }
impl Direction {
    fn name(self)->&'static str {match self {Self::SideToLead=>"SIDE_TO_LEAD",Self::LeadToSide=>"LEAD_TO_SIDE"}}
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DeliveryIntent {
    pub(crate) domain_id:String,
    pub(crate) request_id:String,
    pub(crate) side_id:String,
    pub(crate) direction:String,
    pub(crate) source_seat_id:String,
    pub(crate) source_seat_incarnation:String,
    pub(crate) source_session_id:String,
    pub(crate) target_seat_id:String,
    pub(crate) target_seat_incarnation:String,
    pub(crate) target_session_id:String,
    pub(crate) target_generation:String,
    pub(crate) body:String,
    pub(crate) message_id:String,
    pub(crate) enqueue_request_id:String,
    pub(crate) delivery_request_id:String,
    pub(crate) created_at:String,
    pub(crate) dispatch_error:String,
    pub(crate) confirmed_failure:String,
    /// Only the transaction inserting this intent grants one C dispatch.
    pub(crate) may_dispatch:bool,
}
impl DeliveryIntent {
    pub(crate) fn send_request_id(&self)->String {
        self.delivery_request_id.replacen("sidedeliver-","sidesend-",1)
    }
    /// The actual H input is labelled without letting model text choose its
    /// own sender. The visible D body remains the original message.
    pub(crate) fn send_body(&self)->String {
        format!("[gogoke side {} from seat {}]\n{}",self.side_id,self.source_seat_id,self.body)
    }
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum DeliveryState { Unknown, Steered, NewTurn, Failed }
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct DeliveryRecord {
    pub(crate) intent:DeliveryIntent,
    pub(crate) state:DeliveryState,
    pub(crate) native_receipt_id:String,
    pub(crate) reason:String,
}

fn valid_id(value:&str)->bool {
    value.len()<=128 && matches!(value.bytes().next(),Some(b'A'..=b'Z'|b'a'..=b'z')) &&
        value.bytes().all(|byte|byte.is_ascii_alphanumeric()||byte==b'_'||byte==b'-')
}
fn inbox_error(error:inbox::InboxError)->SideError {
    SideError::Corrupt(format!("side C source: {error:?}"))
}
fn from_hex(value:&str)->Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(SideError::Corrupt("side C request hex length".into()));}
    value.as_bytes().chunks_exact(2).map(|part| {
        let pair=std::str::from_utf8(part).map_err(|_|SideError::Corrupt("side C request hex UTF-8".into()))?;
        u8::from_str_radix(pair,16).map_err(|_|SideError::Corrupt("side C request hex digit".into()))
    }).collect()
}
fn load_intent(db:&VerifiedDatabaseConnection<'_>,domain:&str,request_id:&str)->Result<Option<DeliveryIntent>> {
    let row=Statement::prepare(db.as_ptr(),"SELECT side_id,direction,source_seat_id,source_seat_incarnation,source_session_id,target_seat_id,target_seat_incarnation,target_session_id,target_generation,body,message_id,enqueue_request_id,delivery_request_id,created_at,dispatch_error,confirmed_failure FROM main.gogoke_v37_side_delivery WHERE domain_id=?1 AND request_id=?2")?;
    row.bind_text(1,domain)?;row.bind_text(2,request_id)?;
    if !row.step_row()? {return Ok(None);}
    Ok(Some(DeliveryIntent {domain_id:domain.into(),request_id:request_id.into(),
        side_id:row.column_text(0)?,direction:row.column_text(1)?,source_seat_id:row.column_text(2)?,
        source_seat_incarnation:row.column_text(3)?,source_session_id:row.column_text(4)?,
        target_seat_id:row.column_text(5)?,target_seat_incarnation:row.column_text(6)?,
        target_session_id:row.column_text(7)?,target_generation:row.column_text(8)?,
        body:row.column_text(9)?,message_id:row.column_text(10)?,
        enqueue_request_id:row.column_text(11)?,delivery_request_id:row.column_text(12)?,
        created_at:row.column_text(13)?,dispatch_error:row.column_text(14)?,
        confirmed_failure:row.column_text(15)?,may_dispatch:false}))
}

/// C/H may consume only the exact D row that granted this one dispatch.
pub(crate) fn verify_intent(db:&VerifiedDatabaseConnection<'_>,intent:&DeliveryIntent)->Result<()> {
    let mut actual=load_intent(db,&intent.domain_id,&intent.request_id)?.ok_or(SideError::Denied)?;
    let mut supplied=intent.clone();
    actual.may_dispatch=false;supplied.may_dispatch=false;
    actual.dispatch_error.clear();supplied.dispatch_error.clear();
    actual.confirmed_failure.clear();supplied.confirmed_failure.clear();
    if actual!=supplied {return Err(SideError::Conflict);}
    Ok(())
}
pub(crate) fn intent_for_message(db:&VerifiedDatabaseConnection<'_>,domain:&str,
    message_id:&str)->Result<Option<DeliveryIntent>> {
    let row=Statement::prepare(db.as_ptr(),"SELECT request_id FROM main.gogoke_v37_side_delivery WHERE domain_id=?1 AND message_id=?2")?;
    row.bind_text(1,domain)?;row.bind_text(2,message_id)?;
    if !row.step_row()? {return Ok(None);}
    let id=row.column_text(0)?;
    if row.step_row()? {return Err(SideError::Conflict);}
    drop(row);load_intent(db,domain,&id)
}

/// The model-tool route supplies the authenticated native session. The caller
/// selects only the side and direction; D resolves both principals from its
/// original side binding and current H/E incarnations on this same connection.
/// Model callers must provide a same-transaction E MESSAGE grant and current
/// designated lead proof here. The check runs after D derives both seats and
/// before the durable one-shot permit is inserted.
pub(crate) fn prepare(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,domain:&str,
    side_id:&str,request_id:&str,caller_session:&str,direction:Direction,body:&str,
    authorized:impl FnOnce(&VerifiedDatabaseConnection<'_>,&str,&str)->Result<bool>)->Result<DeliveryIntent> {
    if !valid_id(domain)||!valid_id(side_id)||!valid_id(request_id)||body.trim().is_empty()||
        body.len()>crate::ipc::MAX_FRAME_BYTES/2 {return Err(SideError::Invalid("delivery input"));}
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        if let Some(prior)=load_intent(db,domain,request_id)? {
            if prior.side_id!=side_id||prior.direction!=direction.name()||prior.source_session_id!=caller_session||prior.body!=body {
                return Err(SideError::Conflict);
            }
            return Ok(prior); // A replay, including PREPARED/UNKNOWN, never resends.
        }
        let s=side(db,domain,side_id)?;check_history(db,&s)?;
        if s.state!="ACTIVE" {return Err(SideError::Conflict);}
        let lead=current_session(db,&s,true)?;
        let side=current_session(db,&s,false)?;
        let (source_seat,source_inc,source_session,target_seat,target_inc,target)=match direction {
            Direction::SideToLead=>(&s.seat_id,&s.seat_incarnation,&side.0,&s.source_seat_id,&s.source_seat_incarnation,&lead),
            Direction::LeadToSide=>(&s.source_seat_id,&s.source_seat_incarnation,&lead.0,&s.seat_id,&s.seat_incarnation,&side),
        };
        if source_session.as_str()!=caller_session||source_seat==target_seat||target.3.is_empty() {
            return Err(SideError::Denied);
        }
        for session in [source_session.as_str(),target.0.as_str()] {
            let claim=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND state='COMMITTED'")?;
            claim.bind_text(1,domain)?;claim.bind_text(2,session)?;
            if !claim.step_row()? || claim.step_row()? {return Err(SideError::Denied);}
        }
        if !authorized(db,source_seat,target_seat)? {return Err(SideError::Denied);}
        // C's IDs are fixed before any enqueue/H action. Recovery only reads
        // these exact identities; it cannot derive a fresh request to resend.
        let digest=crate::store::digest::sha256_hex(format!("{domain}\n{request_id}").as_bytes());
        let suffix=&digest[..32];
        let intent=DeliveryIntent {domain_id:domain.into(),request_id:request_id.into(),side_id:side_id.into(),
            direction:direction.name().into(),source_seat_id:source_seat.to_string(),
            source_seat_incarnation:source_inc.to_string(),source_session_id:source_session.to_string(),
            target_seat_id:target_seat.to_string(),target_seat_incarnation:target_inc.to_string(),
            target_session_id:target.0.clone(),target_generation:target.1.clone(),body:body.into(),
            message_id:format!("sidemsg-{suffix}"),enqueue_request_id:format!("sideenqueue-{suffix}"),
            delivery_request_id:format!("sidedeliver-{suffix}"),created_at:String::new(),
            dispatch_error:String::new(),confirmed_failure:String::new(),may_dispatch:true};
        let row=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_side_delivery(domain_id,request_id,side_id,direction,source_seat_id,source_seat_incarnation,source_session_id,target_seat_id,target_seat_incarnation,target_session_id,target_generation,body,message_id,enqueue_request_id,delivery_request_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)")?;
        for (index,value) in [&intent.domain_id,&intent.request_id,&intent.side_id,&intent.direction,
            &intent.source_seat_id,&intent.source_seat_incarnation,&intent.source_session_id,
            &intent.target_seat_id,&intent.target_seat_incarnation,&intent.target_session_id,
            &intent.target_generation,&intent.body,&intent.message_id,&intent.enqueue_request_id,
            &intent.delivery_request_id].iter().enumerate() {row.bind_text((index+1) as i32,value)?;}
        row.step_done()?;drop(row);
        let mut stored=load_intent(db,domain,request_id)?.ok_or(SideError::Unknown)?;
        stored.may_dispatch=true;
        Ok(stored)
    })
}

/// Preserve the first real C/H dispatch error. It is diagnostic only: an error
/// cannot turn an uncertain physical write into a confirmed failure.
pub(crate) fn record_error(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    domain:&str,request_id:&str,error:&str)->Result<()> {
    if error.is_empty()||error.len()>crate::ipc::MAX_FRAME_BYTES/4 {return Err(SideError::Invalid("delivery error"));}
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        let intent=load_intent(db,domain,request_id)?.ok_or(SideError::Conflict)?;
        let s=side(db,domain,&intent.side_id)?;check_history(db,&s)?;
        if intent.dispatch_error.is_empty() {
            let row=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_side_delivery SET dispatch_error=?3 WHERE domain_id=?1 AND request_id=?2 AND dispatch_error=''")?;
            row.bind_text(1,domain)?;row.bind_text(2,request_id)?;row.bind_text(3,error)?;
            row.step_done()?;
        }
        Ok(())
    })
}

/// Only C's exact old-turn conflict may close an unsent steer as FAILED.
/// A timeout, generic exception or model string remains UNKNOWN.
pub(crate) fn record_ended_turn(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    intent:&DeliveryIntent,receipt_bytes:&[u8])->Result<()> {
    let receipt=decode_receipt(receipt_bytes)
        .map_err(|error|SideError::Corrupt(format!("side original C receipt: {error:?}")))?;
    if receipt.family!="K-INBOX"||receipt.operation!="steer"||
        receipt.request_id!=intent.delivery_request_id||
        receipt.target_id!=intent.message_id||receipt.status!=V37Status::Conflict {
        return Err(SideError::Denied);
    }
    if !matches!(receipt.into_result().get(&JsonString::from_str("reason")),
        Some(Json::String(value)) if value==&JsonString::from_str("TURN_ENDED")) {
        return Err(SideError::Denied);
    }
    let original=std::str::from_utf8(receipt_bytes)
        .map_err(|_|SideError::Invalid("side C receipt UTF-8"))?;
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        verify_intent(db,intent)?;
        let s=side(db,&intent.domain_id,&intent.side_id)?;check_history(db,&s)?;
        let message=inbox::read_message(db,&intent.domain_id,&intent.message_id).map_err(inbox_error)?
            .ok_or(SideError::Conflict)?;
        if message.state!="PENDING"||message.sender_seat_id!=intent.source_seat_id||
            message.seat_id!=intent.target_seat_id||message.body!=intent.send_body()||
            message.generation!=intent.target_generation||
            inbox::read_operation(db,&intent.domain_id,&intent.delivery_request_id)
                .map_err(inbox_error)?.is_some() {return Err(SideError::Conflict);}
        let row=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_side_delivery SET confirmed_failure='TURN_ENDED',dispatch_error=?3 WHERE domain_id=?1 AND request_id=?2 AND confirmed_failure=''")?;
        row.bind_text(1,&intent.domain_id)?;row.bind_text(2,&intent.request_id)?;
        row.bind_text(3,original)?;row.step_done()?;
        Ok(())
    })
}

fn observed(db:&VerifiedDatabaseConnection<'_>,intent:DeliveryIntent)->Result<DeliveryRecord> {
    let mut record=DeliveryRecord {reason:intent.dispatch_error.clone(),intent,
        state:DeliveryState::Unknown,native_receipt_id:String::new()};
    let i=&record.intent;
    if i.confirmed_failure=="TURN_ENDED" {
        record.state=DeliveryState::Failed;
        return Ok(record);
    }
    let Some(message)=inbox::read_message(db,&i.domain_id,&i.message_id).map_err(inbox_error)? else {
        if record.reason.is_empty() {record.reason="C enqueue has no recorded result; delivery is unconfirmed".into();}
        return Ok(record);
    };
    if message.sender_seat_id!=i.source_seat_id||message.seat_id!=i.target_seat_id||
        message.generation!=i.target_generation||message.body!=i.send_body() {
        return Err(SideError::Conflict);
    }
    let Some(operation)=inbox::read_operation(db,&i.domain_id,&i.delivery_request_id).map_err(inbox_error)? else {
        if record.reason.is_empty() {record.reason="C delivery has no recorded result; delivery is unconfirmed".into();}
        return Ok(record);
    };
    if operation.message_id!=i.message_id {return Err(SideError::Conflict);}
    let request=decode_request(&from_hex(&operation.request_hex)?)
        .map_err(|error|SideError::Corrupt(format!("side C request: {error:?}")))?;
    if request.family!="K-INBOX"||request.domain_id!=i.domain_id||
        request.request_id!=i.delivery_request_id||request.target_id!=i.message_id||
        !matches!(request.operation.as_str(),"steer"|"deliver") {return Err(SideError::Conflict);}
    if !operation.reason.is_empty() {record.reason=operation.reason;}
    match (operation.phase.as_str(),message.state.as_str()) {
        ("APPLIED","DELIVERED") if !operation.native_receipt_id.is_empty()=>{
            if request.operation=="steer" {
                record.state=DeliveryState::Steered;
                record.native_receipt_id=operation.native_receipt_id.clone();
                record.reason.clear();
            } else {
                // K-INBOX/deliver is append-without-turn in the current C/H
                // adapter. An ordinary H send and its original createdTurn
                // receipt must exist before this can be called a new turn.
                let send_id=i.send_request_id();
                let ticket=Statement::prepare(db.as_ptr(),"SELECT ticket FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND request_id=?2")?;
                ticket.bind_text(1,&i.domain_id)?;ticket.bind_text(2,&send_id)?;
                if !ticket.step_row()? {
                    record.reason="C deliver is not an H new-turn receipt".into();return Ok(record);
                }
                let original=ticket.column_text(0)?;
                if ticket.step_row()? {return Err(SideError::Conflict);}
                drop(ticket);
                let Some(journal)=read_stdin_journal(db,&StdinJournalKey {domain_id:&i.domain_id,
                    request_id:&send_id,session_id:&i.target_session_id,ticket:&original,
                    generation:&i.target_generation})
                    .map_err(|error|SideError::Corrupt(format!("side original H send: {error:?}")))? else {
                        record.reason="H new turn has no original journal result".into();return Ok(record);
                    };
                let sent=decode_request(&journal.request_bytes)
                    .map_err(|error|SideError::Corrupt(format!("side original H request: {error:?}")))?;
                if sent.family!="K-SESSION"||sent.operation!="send"||sent.request_id!=send_id||
                    sent.domain_id!=i.domain_id||sent.target_id!=i.target_session_id||
                    field(&sent,"generation")?!=i.target_generation||field(&sent,"body")?!=i.send_body() {
                    return Err(SideError::Conflict);
                }
                if journal.state!=JournalState::Receipted {
                    record.reason="H new turn remains unconfirmed".into();return Ok(record);
                }
                let receipt=decode_receipt(journal.receipt_bytes.as_deref().ok_or(SideError::Unknown)?)
                    .map_err(|error|SideError::Corrupt(format!("side original H receipt: {error:?}")))?;
                if receipt.family!="K-SESSION"||receipt.operation!="send"||
                    receipt.request_id!=send_id||receipt.target_id!=i.target_session_id||
                    !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
                    return Err(SideError::Conflict);
                }
                let result=receipt.into_result();
                let native_id=match result.get(&JsonString::from_str("receiptId")) {
                    Some(Json::String(value))=>value.to_well_formed_string().filter(|value|!value.is_empty()),
                    _=>None,
                };
                if !matches!(result.get(&JsonString::from_str("createdTurn")),Some(Json::Bool(value)) if *value)||
                    !matches!(result.get(&JsonString::from_str("generation")),Some(Json::String(value))
                        if value==&JsonString::from_str(&i.target_generation))||
                    native_id.as_deref()!=Some(operation.native_receipt_id.as_str()) {
                    return Err(SideError::Conflict);
                }
                record.state=DeliveryState::NewTurn;
                record.native_receipt_id=operation.native_receipt_id.clone();
                record.reason.clear();
            }
        },
        ("FAILED","FAILED") | ("DENIED",_) | ("CONFLICT",_)=>{
            record.state=DeliveryState::Failed;
            if record.reason.is_empty() {record.reason=operation.phase;}
        },
        _=>{
            if record.reason.is_empty() {record.reason="H delivery remains unconfirmed".into();}
        },
    }
    Ok(record)
}

/// K-SIDE deletion cannot erase a side while its C/H delivery remains
/// uncertain. This reads the same original C rows as the visible projection.
pub(super) fn unresolved(db:&VerifiedDatabaseConnection<'_>,domain:&str,side_id:&str)->Result<bool> {
    let query=Statement::prepare(db.as_ptr(),"SELECT request_id FROM main.gogoke_v37_side_delivery WHERE domain_id=?1 AND side_id=?2")?;
    query.bind_text(1,domain)?;query.bind_text(2,side_id)?;
    let mut ids=Vec::new();while query.step_row()? {ids.push(query.column_text(0)?);}drop(query);
    for id in ids {
        let intent=load_intent(db,domain,&id)?.ok_or(SideError::Conflict)?;
        if observed(db,intent)?.state==DeliveryState::Unknown {return Ok(true);}
    }
    Ok(false)
}

pub(super) fn remove_side(db:&VerifiedDatabaseConnection<'_>,domain:&str,side_id:&str)->Result<()> {
    let row=Statement::prepare(db.as_ptr(),"DELETE FROM main.gogoke_v37_side_delivery WHERE domain_id=?1 AND side_id=?2")?;
    row.bind_text(1,domain)?;row.bind_text(2,side_id)?;row.step_done()?;Ok(())
}

/// Read C's original operation and H-backed native receipt. This never writes
/// or retries a request and therefore remains safe after timeout/restart.
pub(crate) fn observe(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,domain:&str,
    request_id:&str)->Result<DeliveryRecord> {
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        let intent=load_intent(db,domain,request_id)?.ok_or(SideError::Conflict)?;
        let s=side(db,domain,&intent.side_id)?;check_history(db,&s)?;
        observed(db,intent)
    })
}

pub(crate) fn lines(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,domain:&str,
    side_id:&str)->Result<Vec<DeliveryRecord>> {
    transact(db,|db| {
        authority::check_owner_in_current_transaction(db,owner)?;
        let s=side(db,domain,side_id)?;check_history(db,&s)?;
        let query=Statement::prepare(db.as_ptr(),"SELECT request_id FROM main.gogoke_v37_side_delivery WHERE domain_id=?1 AND side_id=?2 ORDER BY rowid")?;
        query.bind_text(1,domain)?;query.bind_text(2,side_id)?;
        let mut ids=Vec::new();while query.step_row()? {ids.push(query.column_text(0)?);}drop(query);
        ids.into_iter().map(|id| {
            let intent=load_intent(db,domain,&id)?.ok_or(SideError::Conflict)?;
            observed(db,intent)
        }).collect()
    })
}
