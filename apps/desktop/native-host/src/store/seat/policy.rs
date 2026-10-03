//! E.2 native call authority. The app server and model never mint a caller.
//! H supplies the current authenticated seat and turn after its own claim,
//! generation, process and admission checks; these rows are rechecked here.

use super::*;
use std::time::{SystemTime,UNIX_EPOCH};

pub(super) const POLICY_HEAD: &str = "CREATE TABLE gogoke_v37_seat_policy_head(domain_id TEXT PRIMARY KEY,revision INTEGER NOT NULL CHECK(revision>0),current_stage TEXT NOT NULL) STRICT";
pub(super) const POLICY_GRANTS: &str = "CREATE TABLE gogoke_v37_seat_policy_grants(domain_id TEXT NOT NULL,caller_seat_id TEXT NOT NULL,target_id TEXT NOT NULL,action TEXT NOT NULL CHECK(action IN ('DISPATCH','REVIEW','MESSAGE','MERGE')),expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>=0),revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,caller_seat_id,target_id,action)) STRICT";
pub(super) const POLICY_GATES: &str = "CREATE TABLE gogoke_v37_seat_policy_gates(domain_id TEXT NOT NULL,gate_id TEXT NOT NULL,submitter_seat_id TEXT NOT NULL,reviewer_seat_id TEXT NOT NULL,from_stage TEXT NOT NULL,to_stage TEXT NOT NULL,reject_cap INTEGER NOT NULL CHECK(reject_cap>0),reject_count INTEGER NOT NULL CHECK(reject_count>=0),state TEXT NOT NULL CHECK(state IN ('READY','SUBMITTED','PASSED','REJECTED','ESCALATION_REQUIRED','ADVANCED')),reason TEXT,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,gate_id)) STRICT";
pub(super) const POLICY_ROUTES: &str = "CREATE TABLE gogoke_v37_seat_policy_routes(domain_id TEXT NOT NULL,from_seat_id TEXT NOT NULL,reason TEXT NOT NULL CHECK(reason IN ('REJECT_CAP','STALL')),to_seat_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,from_seat_id,reason)) STRICT";
pub(super) const POLICY_ESCALATIONS: &str = "CREATE TABLE gogoke_v37_seat_policy_escalations(domain_id TEXT NOT NULL,trigger_id TEXT NOT NULL,request_id TEXT NOT NULL,from_seat_id TEXT NOT NULL,to_seat_id TEXT NOT NULL,reason TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('INTENT','UNKNOWN','DELIVERED')),delivery_receipt_id TEXT,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,trigger_id),UNIQUE(domain_id,request_id)) STRICT";
pub(super) const POLICY_EVENTS: &str = "CREATE TABLE gogoke_v37_seat_policy_events(domain_id TEXT NOT NULL,event_id TEXT NOT NULL,operation TEXT NOT NULL,fingerprint TEXT NOT NULL,target_id TEXT NOT NULL,policy_revision INTEGER NOT NULL,state TEXT NOT NULL,detail TEXT NOT NULL,PRIMARY KEY(domain_id,event_id)) STRICT";

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum CallAction { Dispatch, Review, Message, Merge }
impl CallAction {
    fn sql(self)->&'static str { match self {Self::Dispatch=>"DISPATCH",Self::Review=>"REVIEW",
        Self::Message=>"MESSAGE",Self::Merge=>"MERGE"} }
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct NativeSeatCall {
    domain_id:String, seat_id:String, incarnation:String, generation:i64, turn_id:String,
}
impl NativeSeatCall {
    /// H alone calls this after verifying the live session and original turn.
    /// A seat snapshot or a turn string received over IPC is not that proof.
    pub(crate) fn from_verified_h_turn(seat:&Seat, turn_id:&str)->Result<Self,SeatError> {
        if seat.state!=State::Busy || !valid_id(turn_id) { return Err(SeatError::Denied); }
        Ok(Self {domain_id:seat.domain_id.clone(),seat_id:seat.seat_id.clone(),
            incarnation:seat.incarnation.clone(),generation:seat.generation,turn_id:turn_id.into()})
    }
    pub(crate) fn seat_id(&self)->&str { &self.seat_id }
    pub(crate) fn domain_id(&self)->&str { &self.domain_id }
}

pub(super) fn current_caller(db:&VerifiedDatabaseConnection<'_>, caller:&NativeSeatCall)->Result<Seat,SeatError> {
    let seat=read(db,&caller.domain_id,&caller.seat_id)?.ok_or(SeatError::Denied)?;
    if seat.state!=State::Busy || seat.incarnation!=caller.incarnation ||
        seat.generation!=caller.generation { return Err(SeatError::Denied); }
    Ok(seat)
}

fn now_ms()->Result<i64,SeatError> {
    i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)
        .map_err(|_|SeatError::Denied)?.as_millis()).map_err(|_|SeatError::Denied)
}

fn head_revision(db:&VerifiedDatabaseConnection<'_>,domain:&str)->Result<i64,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT revision FROM main.gogoke_v37_seat_policy_head WHERE domain_id=?1")?;
    q.bind_text(1,domain)?;
    if !q.step_row()? { return Err(SeatError::Denied); }
    let rev=q.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    if rev<=0 || q.step_row()? { return Err(SeatError::SchemaDrift); }
    Ok(rev)
}

pub(crate) fn initialize_policy(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    domain:&str,initial_stage:&str)->Result<i64,SeatError> {
    if !valid_id(domain)||!valid_id(initial_stage) { return Err(SeatError::Invalid("policy stage")); }
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_policy_head(domain_id,revision,current_stage) VALUES(?1,1,?2)")?;
        q.bind_text(1,domain)?;q.bind_text(2,initial_stage)?;q.step_done()?;
        Ok(1)
    })
}

/// Owner configuration changes the call table by CAS. There is no grant
/// default; an absent row is a denial. An expiry is measured at every use.
pub(crate) fn configure_call_grant(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    domain:&str,caller:&str,target:&str,action:CallAction,expires_at_ms:Option<i64>,
    expected_revision:i64)->Result<i64,SeatError> {
    if !valid_id(domain)||!valid_id(caller)||!valid_id(target)||expected_revision<1 ||
        expires_at_ms.is_some_and(|expiry|expiry<=0) { return Err(SeatError::Invalid("call grant")); }
    if action==CallAction::Merge && target!="MAIN" { return Err(SeatError::Invalid("merge target")); }
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        if head_revision(db,domain)?!=expected_revision { return Err(SeatError::Conflict); }
        let next=expected_revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_policy_grants(domain_id,caller_seat_id,target_id,action,expires_at_ms,revision) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(domain_id,caller_seat_id,target_id,action) DO UPDATE SET expires_at_ms=excluded.expires_at_ms,revision=excluded.revision")?;
        q.bind_text(1,domain)?;q.bind_text(2,caller)?;q.bind_text(3,target)?;
        q.bind_text(4,action.sql())?;
        q.bind_i64(5,expires_at_ms.unwrap_or(0))?;
        q.bind_i64(6,next)?;q.step_done()?;
        let update=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_head SET revision=?1 WHERE domain_id=?2 AND revision=?3")?;
        update.bind_i64(1,next)?;update.bind_text(2,domain)?;
        update.bind_i64(3,expected_revision)?;update.step_done()?;
        if head_revision(db,domain)?!=next { return Err(SeatError::Conflict); }
        Ok(next)
    })
}

pub(crate) fn authorize_current_call(db:&VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    target_domain:&str,target_id:&str,action:CallAction)->Result<i64,SeatError> {
    if caller.domain_id!=target_domain || !valid_id(target_id) { return Err(SeatError::Denied); }
    let seat=current_caller(db,caller)?;
    if seat.layer==Layer::Lead && target_id=="OWNER" { return Err(SeatError::Denied); }
    if action==CallAction::Merge && (target_id!="MAIN" || seat.instance_id.is_empty()) {
        return Err(SeatError::Denied);
    }
    let revision=head_revision(db,target_domain)?;
    let q=Statement::prepare(db.as_ptr(),
        "SELECT expires_at_ms FROM main.gogoke_v37_seat_policy_grants WHERE domain_id=?1 AND caller_seat_id=?2 AND target_id=?3 AND action=?4")?;
    q.bind_text(1,target_domain)?;q.bind_text(2,&caller.seat_id)?;
    q.bind_text(3,target_id)?;q.bind_text(4,action.sql())?;
    if !q.step_row()? { return Err(SeatError::Denied); }
    let expiry=q.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    if q.step_row()? { return Err(SeatError::SchemaDrift); }
    if expiry<0 || (expiry>0 && now_ms()? >= expiry) { return Err(SeatError::Denied); }
    Ok(revision)
}

/// F.2 supplies the exact seat bound to its registered worktree. H supplies
/// this caller only for the active original turn. A different seat is denied.
pub(crate) fn authorize_merge_for_f2(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall,worktree_seat_id:&str)->Result<Option<String>,SeatError> {
    if caller.seat_id!=worktree_seat_id { return Ok(None); }
    authorize_current_call(db,caller,&caller.domain_id,"MAIN",CallAction::Merge)?;
    Ok(Some(caller.turn_id.clone()))
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct CallPermissionRow {
    pub(crate) target_id:String,pub(crate) action:String,
    pub(crate) expires_at_ms:Option<i64>,
}
pub(crate) fn current_call_permission_table(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall)->Result<(i64,Vec<CallPermissionRow>),SeatError> {
    current_caller(db,caller)?;
    let revision=head_revision(db,&caller.domain_id)?;
    let q=Statement::prepare(db.as_ptr(),
        "SELECT target_id,action,expires_at_ms FROM main.gogoke_v37_seat_policy_grants WHERE domain_id=?1 AND caller_seat_id=?2 ORDER BY target_id,action")?;
    q.bind_text(1,&caller.domain_id)?;q.bind_text(2,&caller.seat_id)?;
    let mut rows=Vec::new();let now=now_ms()?;
    while q.step_row()? {
        let expiry=q.column_text(2)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        if expiry<0 { return Err(SeatError::SchemaDrift); }
        if expiry>0 && expiry<=now { continue; }
        rows.push(CallPermissionRow {target_id:q.column_text(0)?,action:q.column_text(1)?,
            expires_at_ms:if expiry==0 {None}else{Some(expiry)}});
    }
    Ok((revision,rows))
}

fn advance_head(db:&VerifiedDatabaseConnection<'_>,domain:&str,expected:i64)
    ->Result<i64,SeatError> {
    let next=expected.checked_add(1).ok_or(SeatError::Conflict)?;
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_seat_policy_head SET revision=?1 WHERE domain_id=?2 AND revision=?3")?;
    q.bind_i64(1,next)?;q.bind_text(2,domain)?;q.bind_i64(3,expected)?;q.step_done()?;
    if head_revision(db,domain)?!=next { return Err(SeatError::Conflict); }
    Ok(next)
}

pub(crate) fn configure_gate(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    domain:&str,gate_id:&str,submitter:&str,reviewer:&str,from_stage:&str,to_stage:&str,
    reject_cap:i64,expected_policy_revision:i64)->Result<i64,SeatError> {
    if [domain,gate_id,submitter,reviewer,from_stage,to_stage].iter().any(|value|!valid_id(value))
        || submitter==reviewer || from_stage==to_stage || reject_cap<=0 || expected_policy_revision<1 {
        return Err(SeatError::Invalid("gate configuration"));
    }
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        if head_revision(db,domain)?!=expected_policy_revision { return Err(SeatError::Conflict); }
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_policy_gates(domain_id,gate_id,submitter_seat_id,reviewer_seat_id,from_stage,to_stage,reject_cap,reject_count,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,0,'READY',1)")?;
        for (index,value) in [domain,gate_id,submitter,reviewer,from_stage,to_stage].iter().enumerate() {
            q.bind_text((index+1) as i32,value)?;
        }
        q.bind_i64(7,reject_cap)?;q.step_done()?;
        advance_head(db,domain,expected_policy_revision)
    })
}

pub(crate) fn configure_escalation_route(db:&mut VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,domain:&str,from_seat:&str,reason:&str,to_seat:&str,
    expected_policy_revision:i64)->Result<i64,SeatError> {
    if !valid_id(domain)||!valid_id(from_seat)||!valid_id(to_seat)||
        !matches!(reason,"REJECT_CAP"|"STALL")||expected_policy_revision<1 ||
        from_seat==to_seat {
        return Err(SeatError::Invalid("escalation route"));
    }
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        if head_revision(db,domain)?!=expected_policy_revision { return Err(SeatError::Conflict); }
        let source=read(db,domain,from_seat)?.ok_or(SeatError::Denied)?;
        if source.layer==Layer::Lead && to_seat=="OWNER" { return Err(SeatError::Denied); }
        if to_seat!="OWNER" { read(db,domain,to_seat)?.ok_or(SeatError::Denied)?; }
        let next=expected_policy_revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_policy_routes(domain_id,from_seat_id,reason,to_seat_id,revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(domain_id,from_seat_id,reason) DO UPDATE SET to_seat_id=excluded.to_seat_id,revision=excluded.revision")?;
        q.bind_text(1,domain)?;q.bind_text(2,from_seat)?;q.bind_text(3,reason)?;
        q.bind_text(4,to_seat)?;q.bind_i64(5,next)?;q.step_done()?;
        advance_head(db,domain,expected_policy_revision)
    })
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct PolicyEvent {
    pub(crate) event_id:String,pub(crate) operation:String,pub(crate) target_id:String,
    pub(crate) policy_revision:i64,pub(crate) state:String,pub(crate) detail:String,
    pub(crate) replayed:bool,
}

fn prior_event(db:&VerifiedDatabaseConnection<'_>,domain:&str,event_id:&str,
    operation:&str,fingerprint:&str)->Result<Option<PolicyEvent>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT operation,fingerprint,target_id,policy_revision,state,detail FROM main.gogoke_v37_seat_policy_events WHERE domain_id=?1 AND event_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,event_id)?;
    if !q.step_row()? {return Ok(None);}
    if q.column_text(0)?!=operation || q.column_text(1)?!=fingerprint {
        return Err(SeatError::Conflict);
    }
    let event=PolicyEvent {event_id:event_id.into(),operation:operation.into(),
        target_id:q.column_text(2)?,policy_revision:q.column_text(3)?.parse()
            .map_err(|_|SeatError::SchemaDrift)?,state:q.column_text(4)?,
        detail:q.column_text(5)?,replayed:true};
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(Some(event))
}

fn record_event(db:&VerifiedDatabaseConnection<'_>,domain:&str,mut event:PolicyEvent,
    fingerprint:&str)->Result<PolicyEvent,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_seat_policy_events(domain_id,event_id,operation,fingerprint,target_id,policy_revision,state,detail) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")?;
    for (index,value) in [domain,event.event_id.as_str(),event.operation.as_str(),fingerprint,
        event.target_id.as_str()].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    q.bind_i64(6,event.policy_revision)?;q.bind_text(7,&event.state)?;
    q.bind_text(8,&event.detail)?;q.step_done()?;
    event.replayed=false;
    Ok(event)
}

fn gate_row(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str)
    ->Result<(String,String,String,String,i64,i64,String,i64),SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT submitter_seat_id,reviewer_seat_id,from_stage,to_stage,reject_cap,reject_count,state,revision FROM main.gogoke_v37_seat_policy_gates WHERE domain_id=?1 AND gate_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;
    if !q.step_row()? {return Err(SeatError::Denied);}
    let parse=|i|q.column_text(i)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift);
    let row=(q.column_text(0)?,q.column_text(1)?,q.column_text(2)?,q.column_text(3)?,
        parse(4)?,parse(5)?,q.column_text(6)?,parse(7)?);
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(row)
}

fn stage(db:&VerifiedDatabaseConnection<'_>,domain:&str)->Result<String,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT current_stage FROM main.gogoke_v37_seat_policy_head WHERE domain_id=?1")?;
    q.bind_text(1,domain)?;
    if !q.step_row()? {return Err(SeatError::Denied);}
    let value=q.column_text(0)?;
    if !valid_id(&value)||q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(value)
}

pub(crate) fn gate_submit(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    gate_id:&str,expected_policy_revision:i64,expected_gate_revision:i64,
    event_id:&str,original_raw:&[u8])->Result<PolicyEvent,SeatError> {
    validate(&caller.domain_id,gate_id,event_id,original_raw)?;
    let fp=fingerprint(&["gate-submit",&caller.seat_id,gate_id,
        &expected_policy_revision.to_string(),&expected_gate_revision.to_string()],original_raw);
    transact(db,|db| {
        current_caller(db,caller)?;
        if let Some(old)=prior_event(db,&caller.domain_id,event_id,"gate-submit",&fp)? {return Ok(old);}
        if head_revision(db,&caller.domain_id)?!=expected_policy_revision {return Err(SeatError::Conflict);}
        let row=gate_row(db,&caller.domain_id,gate_id)?;
        if row.0!=caller.seat_id || row.7!=expected_gate_revision ||
            !matches!(row.6.as_str(),"READY"|"REJECTED") || row.5>=row.4 ||
            stage(db,&caller.domain_id)?!=row.2 {return Err(SeatError::Denied);}
        authorize_current_call(db,caller,&caller.domain_id,&row.1,CallAction::Review)?;
        let next=row.7.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_gates SET state='SUBMITTED',reason=NULL,revision=?1 WHERE domain_id=?2 AND gate_id=?3 AND revision=?4")?;
        q.bind_i64(1,next)?;q.bind_text(2,&caller.domain_id)?;
        q.bind_text(3,gate_id)?;q.bind_i64(4,row.7)?;q.step_done()?;
        if gate_row(db,&caller.domain_id,gate_id)?.7!=next {return Err(SeatError::Conflict);}
        record_event(db,&caller.domain_id,PolicyEvent {event_id:event_id.into(),
            operation:"gate-submit".into(),target_id:gate_id.into(),policy_revision:expected_policy_revision,
            state:"SUBMITTED".into(),detail:String::new(),replayed:false},&fp)
    })
}

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum GateDecision {Pass,Reject}
pub(crate) fn gate_decide(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    gate_id:&str,decision:GateDecision,reason:&str,expected_policy_revision:i64,
    expected_gate_revision:i64,event_id:&str,original_raw:&[u8])->Result<PolicyEvent,SeatError> {
    validate(&caller.domain_id,gate_id,event_id,original_raw)?;
    if (decision==GateDecision::Reject && (reason.is_empty()||reason.len()>4096)) ||
        (decision==GateDecision::Pass && !reason.is_empty()) {return Err(SeatError::Invalid("gate reason"));}
    let label=if decision==GateDecision::Pass {"PASS"}else{"REJECT"};
    let fp=fingerprint(&["gate-decide",&caller.seat_id,gate_id,label,reason,
        &expected_policy_revision.to_string(),&expected_gate_revision.to_string()],original_raw);
    transact(db,|db| {
        current_caller(db,caller)?;
        if let Some(old)=prior_event(db,&caller.domain_id,event_id,"gate-decide",&fp)? {return Ok(old);}
        if head_revision(db,&caller.domain_id)?!=expected_policy_revision {return Err(SeatError::Conflict);}
        let row=gate_row(db,&caller.domain_id,gate_id)?;
        if row.1!=caller.seat_id || row.7!=expected_gate_revision || row.6!="SUBMITTED" {
            return Err(SeatError::Denied);
        }
        let rejects=if decision==GateDecision::Reject {row.5.checked_add(1).ok_or(SeatError::Conflict)?}else{row.5};
        let state=if decision==GateDecision::Pass {"PASSED"} else if rejects>=row.4 {
            "ESCALATION_REQUIRED"} else {"REJECTED"};
        let next=row.7.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_gates SET state=?1,reason=?2,reject_count=?3,revision=?4 WHERE domain_id=?5 AND gate_id=?6 AND revision=?7")?;
        q.bind_text(1,state)?;q.bind_text(2,reason)?;q.bind_i64(3,rejects)?;
        q.bind_i64(4,next)?;q.bind_text(5,&caller.domain_id)?;
        q.bind_text(6,gate_id)?;q.bind_i64(7,row.7)?;q.step_done()?;
        if gate_row(db,&caller.domain_id,gate_id)?.7!=next {return Err(SeatError::Conflict);}
        record_event(db,&caller.domain_id,PolicyEvent {event_id:event_id.into(),
            operation:"gate-decide".into(),target_id:gate_id.into(),policy_revision:expected_policy_revision,
            state:state.into(),detail:reason.into(),replayed:false},&fp)
    })
}

pub(crate) fn stage_transition(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    gate_id:&str,expected_policy_revision:i64,expected_gate_revision:i64,
    event_id:&str,original_raw:&[u8])->Result<PolicyEvent,SeatError> {
    validate(&caller.domain_id,gate_id,event_id,original_raw)?;
    let fp=fingerprint(&["stage-transition",&caller.seat_id,gate_id,
        &expected_policy_revision.to_string(),&expected_gate_revision.to_string()],original_raw);
    transact(db,|db| {
        current_caller(db,caller)?;
        if let Some(old)=prior_event(db,&caller.domain_id,event_id,"stage-transition",&fp)? {return Ok(old);}
        if head_revision(db,&caller.domain_id)?!=expected_policy_revision {return Err(SeatError::Conflict);}
        let row=gate_row(db,&caller.domain_id,gate_id)?;
        if row.0!=caller.seat_id || row.7!=expected_gate_revision || row.6!="PASSED" ||
            stage(db,&caller.domain_id)?!=row.2 {return Err(SeatError::Denied);}
        let next=expected_policy_revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_head SET current_stage=?1,revision=?2 WHERE domain_id=?3 AND revision=?4 AND current_stage=?5")?;
        q.bind_text(1,&row.3)?;q.bind_i64(2,next)?;q.bind_text(3,&caller.domain_id)?;
        q.bind_i64(4,expected_policy_revision)?;q.bind_text(5,&row.2)?;q.step_done()?;
        if head_revision(db,&caller.domain_id)?!=next || stage(db,&caller.domain_id)?!=row.3 {
            return Err(SeatError::Conflict);
        }
        let gate_next=row.7.checked_add(1).ok_or(SeatError::Conflict)?;
        let update=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_gates SET state='ADVANCED',revision=?1 WHERE domain_id=?2 AND gate_id=?3 AND revision=?4")?;
        update.bind_i64(1,gate_next)?;update.bind_text(2,&caller.domain_id)?;
        update.bind_text(3,gate_id)?;update.bind_i64(4,row.7)?;update.step_done()?;
        if gate_row(db,&caller.domain_id,gate_id)?.7!=gate_next {return Err(SeatError::Conflict);}
        record_event(db,&caller.domain_id,PolicyEvent {event_id:event_id.into(),
            operation:"stage-transition".into(),target_id:gate_id.into(),policy_revision:next,
            state:"ADVANCED".into(),detail:row.3,replayed:false},&fp)
    })
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum EscalationCause {
    RejectCap {gate_id:String},
    Stall {health_event_id:String},
}
impl EscalationCause {
    fn reason(&self)->&'static str {match self {Self::RejectCap{..}=>"REJECT_CAP",Self::Stall{..}=>"STALL"}}
    fn evidence_id(&self)->&str {match self {Self::RejectCap{gate_id}=>gate_id,
        Self::Stall{health_event_id}=>health_event_id}}
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct EscalationIntent {
    pub(crate) trigger_id:String,pub(crate) from_seat_id:String,
    pub(crate) to_seat_id:String,pub(crate) reason:String,
    pub(crate) revision:i64,pub(crate) replayed:bool,
}

/// Stable trigger IDs are supplied by the existing coordinator. This first
/// transaction reserves one send authority; C/H must deliver outside SQLite.
pub(crate) fn begin_escalation(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    cause:EscalationCause,trigger_id:&str,request_id:&str,original_raw:&[u8],
    expected_policy_revision:i64)->Result<EscalationIntent,SeatError> {
    validate(&caller.domain_id,trigger_id,request_id,original_raw)?;
    if !valid_id(cause.evidence_id()) || expected_policy_revision<1 {
        return Err(SeatError::Invalid("escalation cause"));
    }
    let fp=fingerprint(&["escalate",&caller.seat_id,trigger_id,request_id,cause.reason(),
        cause.evidence_id(),&expected_policy_revision.to_string()],original_raw);
    transact(db,|db| {
        let actor=current_caller(db,caller)?;
        if head_revision(db,&caller.domain_id)?!=expected_policy_revision {return Err(SeatError::Conflict);}
        if let Some(old)=prior_event(db,&caller.domain_id,request_id,"escalate",&fp)? {
            let q=Statement::prepare(db.as_ptr(),
                "SELECT from_seat_id,to_seat_id,reason,state,revision FROM main.gogoke_v37_seat_policy_escalations WHERE domain_id=?1 AND trigger_id=?2 AND request_id=?3")?;
            q.bind_text(1,&caller.domain_id)?;q.bind_text(2,trigger_id)?;q.bind_text(3,request_id)?;
            if !q.step_row()? || old.target_id!=trigger_id {return Err(SeatError::Conflict);}
            let intent=EscalationIntent {trigger_id:trigger_id.into(),from_seat_id:q.column_text(0)?,
                to_seat_id:q.column_text(1)?,reason:q.column_text(2)?,
                revision:q.column_text(4)?.parse().map_err(|_|SeatError::SchemaDrift)?,replayed:true};
            if q.step_row()? {return Err(SeatError::SchemaDrift);}
            return Ok(intent);
        }
        match &cause {
            EscalationCause::RejectCap{gate_id}=>{
                let row=gate_row(db,&caller.domain_id,gate_id)?;
                if row.0!=caller.seat_id || row.6!="ESCALATION_REQUIRED" || row.5<row.4 {
                    return Err(SeatError::Denied);
                }
            }
            EscalationCause::Stall{health_event_id}=>{
                super::continuity::require_stalled_health(db,&caller.domain_id,&caller.seat_id,
                    health_event_id)?;
            }
        }
        let route=Statement::prepare(db.as_ptr(),
            "SELECT to_seat_id FROM main.gogoke_v37_seat_policy_routes WHERE domain_id=?1 AND from_seat_id=?2 AND reason=?3")?;
        route.bind_text(1,&caller.domain_id)?;route.bind_text(2,&caller.seat_id)?;
        route.bind_text(3,cause.reason())?;
        if !route.step_row()? {return Err(SeatError::Denied);}
        let destination=route.column_text(0)?;
        if route.step_row()? || (actor.layer==Layer::Lead && destination=="OWNER") {
            return Err(SeatError::Denied);
        }
        if destination!="OWNER" {read(db,&caller.domain_id,&destination)?.ok_or(SeatError::Denied)?;}
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_policy_escalations(domain_id,trigger_id,request_id,from_seat_id,to_seat_id,reason,state,revision) VALUES(?1,?2,?3,?4,?5,?6,'INTENT',1)")?;
        for (index,value) in [caller.domain_id.as_str(),trigger_id,request_id,
            caller.seat_id.as_str(),destination.as_str(),cause.reason()].iter().enumerate() {
            q.bind_text((index+1) as i32,value)?;
        }
        q.step_done()?;
        record_event(db,&caller.domain_id,PolicyEvent {event_id:request_id.into(),
            operation:"escalate".into(),target_id:trigger_id.into(),policy_revision:expected_policy_revision,
            state:"INTENT".into(),detail:cause.evidence_id().into(),replayed:false},&fp)?;
        Ok(EscalationIntent {trigger_id:trigger_id.into(),from_seat_id:caller.seat_id.clone(),
            to_seat_id:destination,reason:cause.reason().into(),revision:1,replayed:false})
    })
}

/// This token can only be constructed after the existing C/H delivery path
/// verifies its original receipt and destination. It does not assert a send.
pub(crate) struct NativeDeliveryEvidence {
    domain_id:String,trigger_id:String,to_seat_id:String,receipt_id:String,
}
impl NativeDeliveryEvidence {
    pub(crate) fn from_verified_c_delivery(domain:&str,trigger:&str,destination:&str,
        receipt:&str)->Result<Self,SeatError> {
        if [domain,trigger,destination,receipt].iter().any(|value|!valid_id(value)) {
            return Err(SeatError::Invalid("delivery evidence"));
        }
        Ok(Self {domain_id:domain.into(),trigger_id:trigger.into(),
            to_seat_id:destination.into(),receipt_id:receipt.into()})
    }
}

/// Second transaction: only the exact original pending trigger can settle.
/// A missing or ambiguous external receipt remains UNKNOWN, never redelivered.
pub(crate) fn settle_escalation(db:&mut VerifiedDatabaseConnection<'_>,
    evidence:&NativeDeliveryEvidence)->Result<EscalationIntent,SeatError> {
    transact(db,|db| {
        let q=Statement::prepare(db.as_ptr(),
            "SELECT from_seat_id,to_seat_id,reason,state,revision,COALESCE(delivery_receipt_id,'') FROM main.gogoke_v37_seat_policy_escalations WHERE domain_id=?1 AND trigger_id=?2")?;
        q.bind_text(1,&evidence.domain_id)?;q.bind_text(2,&evidence.trigger_id)?;
        if !q.step_row()? {return Err(SeatError::Denied);}
        let from=q.column_text(0)?;let to=q.column_text(1)?;let reason=q.column_text(2)?;
        let state=q.column_text(3)?;
        let revision=q.column_text(4)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        let old_receipt=q.column_text(5)?;
        if q.step_row()? || to!=evidence.to_seat_id {return Err(SeatError::Denied);}
        if state=="DELIVERED" && old_receipt==evidence.receipt_id {
            return Ok(EscalationIntent {trigger_id:evidence.trigger_id.clone(),from_seat_id:from,
                to_seat_id:to,reason,revision,replayed:true});
        }
        if state!="INTENT" || !old_receipt.is_empty() {return Err(SeatError::Unknown);}
        let next=revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let update=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_escalations SET state='DELIVERED',delivery_receipt_id=?1,revision=?2 WHERE domain_id=?3 AND trigger_id=?4 AND state='INTENT' AND revision=?5")?;
        update.bind_text(1,&evidence.receipt_id)?;update.bind_i64(2,next)?;
        update.bind_text(3,&evidence.domain_id)?;update.bind_text(4,&evidence.trigger_id)?;
        update.bind_i64(5,revision)?;update.step_done()?;
        Ok(EscalationIntent {trigger_id:evidence.trigger_id.clone(),from_seat_id:from,
            to_seat_id:to,reason,revision:next,replayed:false})
    })
}

pub(crate) fn mark_escalation_unknown(db:&mut VerifiedDatabaseConnection<'_>,
    domain:&str,trigger_id:&str)->Result<(),SeatError> {
    if !valid_id(domain)||!valid_id(trigger_id) {return Err(SeatError::Invalid("trigger"));}
    transact(db,|db| {
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_escalations SET state='UNKNOWN',revision=revision+1 WHERE domain_id=?1 AND trigger_id=?2 AND state='INTENT'")?;
        q.bind_text(1,domain)?;q.bind_text(2,trigger_id)?;q.step_done()?;
        let verify=Statement::prepare(db.as_ptr(),
            "SELECT state FROM main.gogoke_v37_seat_policy_escalations WHERE domain_id=?1 AND trigger_id=?2")?;
        verify.bind_text(1,domain)?;verify.bind_text(2,trigger_id)?;
        if !verify.step_row()?||verify.column_text(0)?!="UNKNOWN"||verify.step_row()? {
            return Err(SeatError::Unknown);
        }
        Ok(())
    })
}
