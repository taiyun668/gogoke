//! E.2 native call authority. The app server and model never mint a caller.
//! H supplies the current authenticated seat and turn after its own claim,
//! generation, process and admission checks; these rows are rechecked here.

use super::*;
use std::time::{SystemTime,UNIX_EPOCH};

#[path = "host_rule.rs"]
mod host_rule;
pub(crate) use host_rule::{HostEscalationProof,observe_host_reject_cap_in_transaction,observe_host_stalled_in_transaction,
    revalidate_host_escalation_in_transaction,read_host_escalation_intent_in_transaction,
    begin_host_escalation_in_transaction};

pub(super) const OLD_POLICY_HEAD: &str = "CREATE TABLE gogoke_v37_seat_policy_head(domain_id TEXT PRIMARY KEY,revision INTEGER NOT NULL CHECK(revision>0),current_stage TEXT NOT NULL) STRICT";
pub(super) const POLICY_HEAD: &str = "CREATE TABLE gogoke_v37_seat_policy_head(domain_id TEXT PRIMARY KEY,revision INTEGER NOT NULL CHECK(revision>0),current_stage TEXT) STRICT";
pub(super) const POLICY_GRANTS: &str = "CREATE TABLE gogoke_v37_seat_policy_grants(domain_id TEXT NOT NULL,caller_seat_id TEXT NOT NULL,target_id TEXT NOT NULL,action TEXT NOT NULL CHECK(action IN ('DISPATCH','REVIEW','MESSAGE','MERGE')),expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>=0),revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,caller_seat_id,target_id,action)) STRICT";
pub(super) const POLICY_GATES: &str = "CREATE TABLE gogoke_v37_seat_policy_gates(domain_id TEXT NOT NULL,gate_id TEXT NOT NULL,submitter_seat_id TEXT NOT NULL,reviewer_seat_id TEXT NOT NULL,from_stage TEXT NOT NULL,to_stage TEXT NOT NULL,reject_cap INTEGER NOT NULL CHECK(reject_cap>0),reject_count INTEGER NOT NULL CHECK(reject_count>=0),state TEXT NOT NULL CHECK(state IN ('READY','SUBMITTED','PASSED','REJECTED','ESCALATION_REQUIRED','ADVANCED')),reason TEXT,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,gate_id)) STRICT";
pub(super) const POLICY_ROUTES: &str = "CREATE TABLE gogoke_v37_seat_policy_routes(domain_id TEXT NOT NULL,from_seat_id TEXT NOT NULL,reason TEXT NOT NULL CHECK(reason IN ('REJECT_CAP','STALL')),to_seat_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,from_seat_id,reason)) STRICT";
pub(super) const POLICY_ESCALATIONS: &str = "CREATE TABLE gogoke_v37_seat_policy_escalations(domain_id TEXT NOT NULL,trigger_id TEXT NOT NULL,request_id TEXT NOT NULL,from_seat_id TEXT NOT NULL,to_seat_id TEXT NOT NULL,reason TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('INTENT','UNKNOWN','DELIVERED')),delivery_receipt_id TEXT,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,trigger_id),UNIQUE(domain_id,request_id)) STRICT";
pub(super) const POLICY_EVENTS: &str = "CREATE TABLE gogoke_v37_seat_policy_events(domain_id TEXT NOT NULL,event_id TEXT NOT NULL,operation TEXT NOT NULL,fingerprint TEXT NOT NULL,target_id TEXT NOT NULL,policy_revision INTEGER NOT NULL,state TEXT NOT NULL,detail TEXT NOT NULL,PRIMARY KEY(domain_id,event_id)) STRICT";
pub(super) const POLICY_TRIGGERS: &str = "CREATE TABLE gogoke_v37_seat_policy_triggers(domain_id TEXT NOT NULL,trigger_id TEXT NOT NULL,owner_seat_id TEXT NOT NULL,event_ref TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('REGISTER_INTENT','REGISTERED','CANCEL_INTENT','CANCELLED','UNKNOWN')),pending_operation TEXT NOT NULL CHECK(pending_operation IN ('REGISTER','CANCEL','NONE')),pending_request_id TEXT NOT NULL,coordinator_receipt_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),PRIMARY KEY(domain_id,trigger_id)) STRICT";

#[derive(Clone,Copy,Debug,Eq,PartialEq)]
pub(crate) enum CallAction { Dispatch, Review, Message, Merge }
impl CallAction {
    fn sql(self)->&'static str { match self {Self::Dispatch=>"DISPATCH",Self::Review=>"REVIEW",
        Self::Message=>"MESSAGE",Self::Merge=>"MERGE"} }
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct NativeSeatCall {
    domain_id:String, seat_id:String, incarnation:String, generation:i64, turn_id:String,
    proof:Option<crate::store::session_transport::model_call::ModelCallProof>,
}
impl NativeSeatCall {
    pub(crate) fn from_model_proof(
        proof:crate::store::session_transport::model_call::ModelCallProof,
    )->Result<Self,SeatError> {
        if proof.generation()<1 || !valid_id(proof.domain_id())
            || !valid_id(proof.seat_id()) || !valid_id(proof.turn_id()) {
            return Err(SeatError::Denied);
        }
        Ok(Self {domain_id:proof.domain_id().to_owned(),seat_id:proof.seat_id().to_owned(),
            incarnation:proof.incarnation().to_owned(),generation:proof.generation(),
            turn_id:proof.turn_id().to_owned(),proof:Some(proof)})
    }
    /// H alone calls this after verifying the live session and original turn.
    /// A seat snapshot or a turn string received over IPC is not that proof.
    #[cfg(test)]
    pub(crate) fn from_verified_h_turn(seat:&Seat, turn_id:&str)->Result<Self,SeatError> {
        if seat.state!=State::Busy || !valid_id(turn_id) { return Err(SeatError::Denied); }
        Ok(Self {domain_id:seat.domain_id.clone(),seat_id:seat.seat_id.clone(),
            incarnation:seat.incarnation.clone(),generation:seat.generation,turn_id:turn_id.into(),
            proof:None})
    }
    pub(crate) fn seat_id(&self)->&str { &self.seat_id }
    pub(crate) fn domain_id(&self)->&str { &self.domain_id }
    pub(crate) fn incarnation(&self)->&str {&self.incarnation}
    pub(crate) fn generation(&self)->i64 {self.generation}
    pub(crate) fn turn_id(&self)->&str { &self.turn_id }
    pub(crate) fn model_proof(&self)
        ->Option<&crate::store::session_transport::model_call::ModelCallProof> {
        self.proof.as_ref()
    }
    pub(crate) fn session_id(&self)->Option<&str> {self.proof.as_ref().map(|proof|proof.session_id())}
    pub(crate) fn thread_id(&self)->Option<&str> {self.proof.as_ref().map(|proof|proof.thread_id())}
    pub(crate) fn tool(&self)->Option<&str> {self.proof.as_ref().map(|proof|proof.tool())}
    pub(crate) fn call_id(&self)->Option<&str> {self.proof.as_ref().map(|proof|proof.call_id())}
    pub(crate) fn typed_rpc_id(&self)
        ->Option<&crate::store::session_transport::codex_rpc::RpcId> {
        self.proof.as_ref().map(|proof|proof.typed_rpc_id())
    }
    pub(crate) fn raw_sha256(&self)->Option<&str> {
        self.proof.as_ref().map(|proof|proof.raw_sha256())
    }
    pub(crate) fn host_request_id(&self)->Option<&str> {
        self.proof.as_ref().map(|proof|proof.host_request_id())
    }
    pub(crate) fn raw_request_bytes(&self)->Option<&[u8]> {
        self.proof.as_ref().map(|proof|proof.raw_request_bytes())
    }
    pub(crate) fn source_locator(&self)->Option<&crate::store::ledger::RawSourceKey> {
        self.proof.as_ref().map(|proof|proof.source())
    }
    pub(crate) fn arguments_json(&self)->Option<&str> {
        self.proof.as_ref().map(|proof|proof.arguments_json())
    }
}

pub(super) fn current_caller(db:&VerifiedDatabaseConnection<'_>, caller:&NativeSeatCall)->Result<Seat,SeatError> {
    if caller.proof.is_some() {
        return crate::store::session_transport::model_call::revalidate_model_call_in_transaction(
            db,caller).map_err(|_|SeatError::Denied);
    }
    #[cfg(not(test))]
    {return Err(SeatError::Denied);}
    #[cfg(test)]
    {
    let seat=read(db,&caller.domain_id,&caller.seat_id)?.ok_or(SeatError::Denied)?;
    if seat.state!=State::Busy || seat.incarnation!=caller.incarnation ||
        seat.generation!=caller.generation { return Err(SeatError::Denied); }
    Ok(seat)
    }
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

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) enum OwnerPolicyHead {
    Absent,
    Present {revision:i64,current_stage:Option<String>},
}

/// Owner-only head read inside the caller's already-open verified snapshot.
/// An absent row is a fact for explicit project intake, never a default grant
/// or an instruction to initialize from this read.
pub(crate) fn read_owner_policy_head_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,domain:&str,
)->Result<OwnerPolicyHead,SeatError> {
    check_current_owner(db,issuer)?;
    if !valid_id(domain) {return Err(SeatError::Invalid("policy domain"));}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT revision,current_stage,current_stage IS NULL FROM main.gogoke_v37_seat_policy_head WHERE domain_id=?1")?;
    q.bind_text(1,domain)?;
    if !q.step_row()? {return Ok(OwnerPolicyHead::Absent);}
    let revision=q.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
    let current_stage=if q.column_text(2)?=="1" {None} else {Some(q.column_text(1)?)};
    if revision<=0 || current_stage.as_deref().is_some_and(|stage|!valid_id(stage)) || q.step_row()? {
        return Err(SeatError::SchemaDrift);
    }
    Ok(OwnerPolicyHead::Present {revision,current_stage})
}

pub(crate) fn current_policy_revision(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall)->Result<i64,SeatError> {
    current_caller(db,caller)?;
    head_revision(db,caller.domain_id())
}

pub(crate) fn policy_revision_for_native_request(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall,operation:&str,request_id:&str)->Result<i64,SeatError> {
    current_caller(db,caller)?;
    let q=Statement::prepare(db.as_ptr(),
        "SELECT operation,policy_revision FROM main.gogoke_v37_seat_policy_events WHERE domain_id=?1 AND event_id=?2")?;
    q.bind_text(1,caller.domain_id())?;q.bind_text(2,request_id)?;
    if q.step_row()? {
        let recorded=q.column_text(0)?;
        let revision=q.column_text(1)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        if q.step_row()? {return Err(SeatError::SchemaDrift);}
        if recorded!=operation {return Err(SeatError::Conflict);}
        return if operation=="stage-transition" {revision.checked_sub(1).ok_or(SeatError::SchemaDrift)}
            else {Ok(revision)};
    }
    head_revision(db,caller.domain_id())
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
        let source=read(db,domain,caller)?.ok_or(SeatError::Denied)?;
        if source.state==State::Reclaimed || (source.layer==Layer::Lead && target=="OWNER") ||
            (action==CallAction::Dispatch && caller==target) {return Err(SeatError::Denied);}
        if target!="MAIN" && target!="OWNER" {
            let destination=read(db,domain,target)?.ok_or(SeatError::Denied)?;
            if destination.state==State::Reclaimed {return Err(SeatError::Denied);}
        }
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

/// The authenticated Owner's original side-open grants only its fixed pair of
/// MESSAGE edges. D and the designated E incarnation are rechecked here; a
/// replay observes the original event and current grants, never restores a
/// later Owner revocation or silently broadens a different policy action.
pub(crate) fn ensure_side_message_pair(db:&mut VerifiedDatabaseConnection<'_>,issuer:&OwnerIssuer,
    domain:&str,side_id:&str,source_seat:&str,source_incarnation:&str,
    side_seat:&str,side_incarnation:&str,open_id:&str,open_bytes:&[u8],
    create_id:&str,create_bytes:&[u8])->Result<i64,SeatError> {
    if [domain,side_id,source_seat,source_incarnation,side_seat,side_incarnation,open_id,create_id]
        .iter().any(|value|!valid_id(value))||
        (source_seat==side_seat&&source_incarnation!=side_incarnation)||
        open_bytes.is_empty()||create_bytes.is_empty() {
        return Err(SeatError::Invalid("side MESSAGE pair"));
    }
    let cause=crate::store::digest::sha256_hex(
        format!("{domain}\n{side_id}\n{open_id}\n{create_id}").as_bytes());
    let event_id=format!("sidepair-{}",&cause[..40]);
    let mut basis=format!("{domain}\n{side_id}\n{source_seat}\n{source_incarnation}\n{side_seat}\n{side_incarnation}\n{open_id}\n{create_id}\n").into_bytes();
    basis.extend_from_slice(&(open_bytes.len() as u64).to_be_bytes());basis.extend_from_slice(open_bytes);
    basis.extend_from_slice(&(create_bytes.len() as u64).to_be_bytes());basis.extend_from_slice(create_bytes);
    let fingerprint=crate::store::digest::sha256_hex(&basis);
    let detail=format!("{source_seat}/{source_incarnation}->{side_seat}/{side_incarnation}");
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        let source=read(db,domain,source_seat)?.ok_or(SeatError::Denied)?;
        let side=read(db,domain,side_seat)?.ok_or(SeatError::Denied)?;
        if source.incarnation!=source_incarnation||side.incarnation!=side_incarnation||
            source.layer!=Layer::User||source.state==State::Reclaimed||side.state==State::Reclaimed {
            return Err(SeatError::Denied);
        }
        let designated=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_project_lead WHERE domain_id=?1 AND seat_id=?2 AND incarnation=?3")?;
        designated.bind_text(1,domain)?;designated.bind_text(2,source_seat)?;
        designated.bind_text(3,source_incarnation)?;
        if !designated.step_row()?||designated.step_row()? {return Err(SeatError::Denied);}drop(designated);
        let registry=Statement::prepare(db.as_ptr(),
            "SELECT source_seat_id,source_seat_incarnation,seat_id,seat_incarnation,state,source_session_id,session_id FROM main.gogoke_v37_side_registry WHERE domain_id=?1 AND side_id=?2")?;
        registry.bind_text(1,domain)?;registry.bind_text(2,side_id)?;
        if !registry.step_row()?||registry.column_text(0)?!=source_seat||
            registry.column_text(1)?!=source_incarnation||registry.column_text(2)?!=side_seat||
            registry.column_text(3)?!=side_incarnation||registry.column_text(4)?=="DELETED"||
            registry.column_text(5)?==registry.column_text(6)?||
            registry.step_row()? {return Err(SeatError::Denied);}drop(registry);
        let previous=prior_event(db,domain,&event_id,"side-message-pair",&fingerprint)?;
        if let Some(ref event)=previous {
            if event.target_id!=side_id||event.state!="APPLIED"||event.detail!=detail {
                return Err(SeatError::Conflict);
            }
        }
        let mut missing=Vec::new();
        // A side window can use the same logical seat as its WORK source.
        // D still binds two distinct exact sessions; E stores one self edge.
        let edges=if source_seat==side_seat {vec![(source_seat,side_seat)]}
            else {vec![(source_seat,side_seat),(side_seat,source_seat)]};
        for (from,to) in edges {
            let grant=Statement::prepare(db.as_ptr(),
                "SELECT expires_at_ms FROM main.gogoke_v37_seat_policy_grants WHERE domain_id=?1 AND caller_seat_id=?2 AND target_id=?3 AND action='MESSAGE'")?;
            grant.bind_text(1,domain)?;grant.bind_text(2,from)?;grant.bind_text(3,to)?;
            if grant.step_row()? {
                let expiry=grant.column_text(0)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
                if expiry<0||(expiry>0&&now_ms()?>=expiry)||grant.step_row()? {
                    return Err(SeatError::Denied);
                }
            } else {missing.push((from,to));}
        }
        if let Some(event)=previous {
            if !missing.is_empty() {return Err(SeatError::Denied);}
            return Ok(event.policy_revision);
        }
        let head=head_revision(db,domain)?;
        let next=if missing.is_empty() {head} else {
            let next=head.checked_add(1).ok_or(SeatError::Conflict)?;
            for (from,to) in missing {
                let row=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_policy_grants(domain_id,caller_seat_id,target_id,action,expires_at_ms,revision) VALUES(?1,?2,?3,'MESSAGE',0,?4)")?;
                row.bind_text(1,domain)?;row.bind_text(2,from)?;row.bind_text(3,to)?;
                row.bind_i64(4,next)?;row.step_done()?;
            }
            let row=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_seat_policy_head SET revision=?3 WHERE domain_id=?1 AND revision=?2")?;
            row.bind_text(1,domain)?;row.bind_i64(2,head)?;row.bind_i64(3,next)?;
            row.step_done()?;
            if head_revision(db,domain)?!=next {return Err(SeatError::Conflict);}
            next
        };
        record_event(db,domain,PolicyEvent {event_id,operation:"side-message-pair".into(),
            target_id:side_id.into(),policy_revision:next,state:"APPLIED".into(),
            detail,replayed:false},&fingerprint)?;
        Ok(next)
    })
}

pub(crate) fn authorize_current_call(db:&VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    target_domain:&str,target_id:&str,action:CallAction)->Result<i64,SeatError> {
    if caller.domain_id!=target_domain || !valid_id(target_id) { return Err(SeatError::Denied); }
    let seat=current_caller(db,caller)?;
    if seat.layer==Layer::Lead && target_id=="OWNER" { return Err(SeatError::Denied); }
    if action==CallAction::Dispatch && target_id==caller.seat_id {return Err(SeatError::Denied);}
    if target_id!="MAIN" && target_id!="OWNER" {
        let target=read(db,target_domain,target_id)?.ok_or(SeatError::Denied)?;
        if target.state==State::Reclaimed {return Err(SeatError::Denied);}
    }
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

/// Called only inside first-create's transaction after the new child exists.
/// The Owner's copied parent scope authorizes this one direct DISPATCH edge;
/// it never grants MAIN, OWNER, a peer, or an arbitrary existing seat.
pub(super) fn derive_new_child_dispatch_grant(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall,child:&Seat)->Result<i64,SeatError> {
    let parent=current_caller(db,caller)?;
    if parent.layer!=Layer::User || child.layer!=Layer::Lead || child.state!=State::Idle ||
        child.domain_id!=parent.domain_id ||
        child.parent_seat_id.as_deref()!=Some(parent.seat_id.as_str()) ||
        child.seat_id==parent.seat_id || !super::continuity::takeover_ready(db,&parent)? {
        return Err(SeatError::Denied);
    }
    super::orchestration::child_within_scope(&parent,
        child.settings_json.as_deref().ok_or(SeatError::Denied)?,&child.instance_id)?;
    let revision=head_revision(db,&parent.domain_id)?;
    let next=revision.checked_add(1).ok_or(SeatError::Conflict)?;
    let q=Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_seat_policy_grants(domain_id,caller_seat_id,target_id,action,expires_at_ms,revision) VALUES(?1,?2,?3,'DISPATCH',0,?4)")?;
    q.bind_text(1,&parent.domain_id)?;q.bind_text(2,&parent.seat_id)?;
    q.bind_text(3,&child.seat_id)?;q.bind_i64(4,next)?;q.step_done()?;
    advance_head(db,&parent.domain_id,revision)
}

/// F.2 supplies the original writer's stored seat. H supplies the distinct
/// current merge caller and its original turn. The source seat is provenance,
/// never an authorization substitute for the caller's current MAIN grant.
pub(crate) fn authorize_merge_for_f2(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall,worktree_domain_id:&str,
    worktree_seat_id:&str)->Result<Option<String>,SeatError> {
    if caller.domain_id!=worktree_domain_id {
        return Ok(None);
    }
    let source=read(db,worktree_domain_id,worktree_seat_id)?.ok_or(SeatError::Denied)?;
    if source.domain_id!=worktree_domain_id || source.seat_id!=worktree_seat_id ||
        source.incarnation.is_empty() {return Err(SeatError::Denied);}
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
        let target_id=q.column_text(0)?;
        if target_id!="MAIN" && target_id!="OWNER" &&
            !matches!(read(db,&caller.domain_id,&target_id)?,Some(target) if target.state!=State::Reclaimed) {
            continue;
        }
        rows.push(CallPermissionRow {target_id,action:q.column_text(1)?,
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

/// The User pipe supplies bytes and an id; only the retained OwnerIssuer grants
/// authority. Its receipt is committed with the policy change, so an exact
/// replay cannot apply the same configuration twice after a lost response.
pub(crate) enum OwnerPolicyCommand<'a> {
    Template {template_id:&'a str,settings_json:&'a [u8]},
    Initialize {stage:&'a str},
    MetadataInitialize,
    SetInitialStage {stage:&'a str,expected_revision:i64},
    Grant {caller:&'a str,target:&'a str,action:CallAction,
        expires_at_ms:Option<i64>,expected_revision:i64},
    Gate {gate_id:&'a str,submitter:&'a str,reviewer:&'a str,
        from_stage:&'a str,to_stage:&'a str,reject_cap:i64,expected_revision:i64},
    Route {from_seat:&'a str,reason:&'a str,to_seat:&'a str,expected_revision:i64},
}

pub(crate) fn apply_owner_policy_configuration(db:&mut VerifiedDatabaseConnection<'_>,
    issuer:&OwnerIssuer,domain:&str,request_id:&str,raw:&[u8],
    command:OwnerPolicyCommand<'_>)->Result<(i64,bool),SeatError> {
    validate(domain,domain,request_id,raw)?;
    let operation=match &command {
        OwnerPolicyCommand::Template{..}=>"seat-template",
        OwnerPolicyCommand::Initialize{..}=>"policy-initialize",
        OwnerPolicyCommand::MetadataInitialize=>"policy-metadata-initialize",
        OwnerPolicyCommand::SetInitialStage{..}=>"policy-stage-set-initial",
        OwnerPolicyCommand::Grant{..}=>"policy-call-grant",
        OwnerPolicyCommand::Gate{..}=>"policy-gate",
        OwnerPolicyCommand::Route{..}=>"policy-escalation-route",
    };
    let fp=fingerprint(&["owner-policy",operation,domain],raw);
    transact(db,|db| {
        check_current_owner(db,issuer)?;
        if let Some(previous)=prior_event(db,domain,request_id,operation,&fp)? {
            return Ok((previous.policy_revision,true));
        }
        let revision=match command {
            OwnerPolicyCommand::Template{template_id,settings_json}=>{
                if !valid_id(template_id) {return Err(SeatError::Invalid("template_id"));}
                validate_template_settings(settings_json)?;
                let settings=std::str::from_utf8(settings_json)
                    .map_err(|_|SeatError::Invalid("template_settings"))?;
                let settings=super::orchestration::normalized_effort_json(settings)?;
                let q=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_templates(domain_id,template_id,settings_json,revision) VALUES(?1,?2,?3,1)")?;
                q.bind_text(1,domain)?;q.bind_text(2,template_id)?;
                q.bind_text(3,&settings)?;q.step_done()?;
                1
            }
            OwnerPolicyCommand::Initialize{stage}=>{
                if !valid_id(stage) {return Err(SeatError::Invalid("policy stage"));}
                let q=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_policy_head(domain_id,revision,current_stage) VALUES(?1,1,?2)")?;
                q.bind_text(1,domain)?;q.bind_text(2,stage)?;q.step_done()?;
                1
            }
            OwnerPolicyCommand::MetadataInitialize=>{
                if read_owner_policy_head_in_transaction(db,issuer,domain)?!=OwnerPolicyHead::Absent {
                    return Err(SeatError::Conflict);
                }
                for table in ["gogoke_v37_seat_policy_grants","gogoke_v37_seat_policy_gates",
                    "gogoke_v37_seat_policy_routes"] {
                    let sql=format!("SELECT 1 FROM main.{table} WHERE domain_id=?1 LIMIT 1");
                    let q=Statement::prepare(db.as_ptr(),&sql)?;
                    q.bind_text(1,domain)?;
                    if q.step_row()? {return Err(SeatError::SchemaDrift);}
                }
                let q=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_policy_head(domain_id,revision,current_stage) VALUES(?1,1,NULL)")?;
                q.bind_text(1,domain)?;q.step_done()?;
                1
            }
            OwnerPolicyCommand::SetInitialStage{stage,expected_revision}=>{
                if !valid_id(stage)||expected_revision<1 {return Err(SeatError::Invalid("policy stage"));}
                match read_owner_policy_head_in_transaction(db,issuer,domain)? {
                    OwnerPolicyHead::Present{revision,current_stage:None} if revision==expected_revision=>{},
                    _=>return Err(SeatError::Conflict),
                }
                let next=expected_revision.checked_add(1).ok_or(SeatError::Conflict)?;
                let q=Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_seat_policy_head SET current_stage=?1,revision=?2 WHERE domain_id=?3 AND revision=?4 AND current_stage IS NULL")?;
                q.bind_text(1,stage)?;q.bind_i64(2,next)?;q.bind_text(3,domain)?;
                q.bind_i64(4,expected_revision)?;q.step_done()?;
                if read_owner_policy_head_in_transaction(db,issuer,domain)?!=
                    (OwnerPolicyHead::Present{revision:next,current_stage:Some(stage.into())}) {
                    return Err(SeatError::Conflict);
                }
                next
            }
            OwnerPolicyCommand::Grant{caller,target,action,expires_at_ms,expected_revision}=>{
                if !valid_id(caller)||!valid_id(target)||expected_revision<1||
                    expires_at_ms.is_some_and(|expiry|expiry<=0)||
                    (action==CallAction::Merge&&target!="MAIN") {
                    return Err(SeatError::Invalid("call grant"));
                }
                if head_revision(db,domain)?!=expected_revision {return Err(SeatError::Conflict);}
                let source=read(db,domain,caller)?.ok_or(SeatError::Denied)?;
                if source.state==State::Reclaimed || (source.layer==Layer::Lead&&target=="OWNER") ||
                    (action==CallAction::Dispatch&&caller==target) {return Err(SeatError::Denied);}
                if target!="MAIN"&&target!="OWNER" {
                    let destination=read(db,domain,target)?.ok_or(SeatError::Denied)?;
                    if destination.state==State::Reclaimed {return Err(SeatError::Denied);}
                }
                let next=expected_revision.checked_add(1).ok_or(SeatError::Conflict)?;
                let q=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_policy_grants(domain_id,caller_seat_id,target_id,action,expires_at_ms,revision) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(domain_id,caller_seat_id,target_id,action) DO UPDATE SET expires_at_ms=excluded.expires_at_ms,revision=excluded.revision")?;
                q.bind_text(1,domain)?;q.bind_text(2,caller)?;q.bind_text(3,target)?;
                q.bind_text(4,action.sql())?;q.bind_i64(5,expires_at_ms.unwrap_or(0))?;
                q.bind_i64(6,next)?;q.step_done()?;
                advance_head(db,domain,expected_revision)?
            }
            OwnerPolicyCommand::Gate{gate_id,submitter,reviewer,from_stage,to_stage,
                reject_cap,expected_revision}=>{
                if [gate_id,submitter,reviewer,from_stage,to_stage].iter().any(|value|!valid_id(value))||
                    submitter==reviewer||from_stage==to_stage||reject_cap<=0||expected_revision<1 {
                    return Err(SeatError::Invalid("gate configuration"));
                }
                if head_revision(db,domain)?!=expected_revision {return Err(SeatError::Conflict);}
                let q=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_policy_gates(domain_id,gate_id,submitter_seat_id,reviewer_seat_id,from_stage,to_stage,reject_cap,reject_count,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,0,'READY',1)")?;
                for (index,value) in [domain,gate_id,submitter,reviewer,from_stage,to_stage].iter().enumerate() {
                    q.bind_text((index+1) as i32,value)?;
                }
                q.bind_i64(7,reject_cap)?;q.step_done()?;
                advance_head(db,domain,expected_revision)?
            }
            OwnerPolicyCommand::Route{from_seat,reason,to_seat,expected_revision}=>{
                if !valid_id(from_seat)||!valid_id(to_seat)||
                    !matches!(reason,"REJECT_CAP"|"STALL")||expected_revision<1||from_seat==to_seat {
                    return Err(SeatError::Invalid("escalation route"));
                }
                if head_revision(db,domain)?!=expected_revision {return Err(SeatError::Conflict);}
                let source=read(db,domain,from_seat)?.ok_or(SeatError::Denied)?;
                if source.layer==Layer::Lead&&to_seat=="OWNER" {return Err(SeatError::Denied);}
                if to_seat!="OWNER" {read(db,domain,to_seat)?.ok_or(SeatError::Denied)?;}
                let next=expected_revision.checked_add(1).ok_or(SeatError::Conflict)?;
                let q=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_seat_policy_routes(domain_id,from_seat_id,reason,to_seat_id,revision) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(domain_id,from_seat_id,reason) DO UPDATE SET to_seat_id=excluded.to_seat_id,revision=excluded.revision")?;
                q.bind_text(1,domain)?;q.bind_text(2,from_seat)?;q.bind_text(3,reason)?;
                q.bind_text(4,to_seat)?;q.bind_i64(5,next)?;q.step_done()?;
                advance_head(db,domain,expected_revision)?
            }
        };
        record_event(db,domain,PolicyEvent {event_id:request_id.into(),
            operation:operation.into(),target_id:domain.into(),policy_revision:revision,
            state:"APPLIED".into(),detail:String::new(),replayed:false},&fp)?;
        Ok((revision,false))
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
        "SELECT current_stage,current_stage IS NULL FROM main.gogoke_v37_seat_policy_head WHERE domain_id=?1")?;
    q.bind_text(1,domain)?;
    if !q.step_row()? {return Err(SeatError::Denied);}
    if q.column_text(1)?=="1" {return Err(SeatError::Denied);}
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
pub(crate) struct TriggerTransition {
    pub(crate) trigger_id:String,pub(crate) state:String,
    pub(crate) revision:i64,pub(crate) external_action_authorized:bool,
}

fn trigger_row(db:&VerifiedDatabaseConnection<'_>,domain:&str,trigger_id:&str)
    ->Result<Option<(String,String,String,String,String,String,i64)>,SeatError> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT owner_seat_id,event_ref,state,pending_operation,pending_request_id,coordinator_receipt_id,revision FROM main.gogoke_v37_seat_policy_triggers WHERE domain_id=?1 AND trigger_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,trigger_id)?;
    if !q.step_row()? {return Ok(None);}
    let row=(q.column_text(0)?,q.column_text(1)?,q.column_text(2)?,q.column_text(3)?,
        q.column_text(4)?,q.column_text(5)?,q.column_text(6)?.parse::<i64>()
            .map_err(|_|SeatError::SchemaDrift)?);
    if q.step_row()? {return Err(SeatError::SchemaDrift);}
    Ok(Some(row))
}

/// The first transaction gives the coordinator one registration authority.
/// The caller must never schedule again for a replayed or UNKNOWN result.
pub(crate) fn begin_trigger_register(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    trigger_id:&str,event_ref:&str,request_id:&str,original_raw:&[u8],
    expected_policy_revision:i64)->Result<TriggerTransition,SeatError> {
    validate(&caller.domain_id,trigger_id,request_id,original_raw)?;
    if !valid_id(event_ref) {return Err(SeatError::Invalid("trigger event"));}
    let fp=fingerprint(&["trigger-register",&caller.seat_id,trigger_id,event_ref,
        &expected_policy_revision.to_string()],original_raw);
    transact(db,|db| {
        current_caller(db,caller)?;
        if let Some(old)=prior_event(db,&caller.domain_id,request_id,"trigger-register",&fp)? {
            let row=trigger_row(db,&caller.domain_id,trigger_id)?.ok_or(SeatError::SchemaDrift)?;
            if old.target_id!=trigger_id||row.0!=caller.seat_id||row.1!=event_ref {
                return Err(SeatError::Conflict);
            }
            return Ok(TriggerTransition {trigger_id:trigger_id.into(),state:row.2,
                revision:row.6,external_action_authorized:false});
        }
        if head_revision(db,&caller.domain_id)?!=expected_policy_revision ||
            trigger_row(db,&caller.domain_id,trigger_id)?.is_some() {return Err(SeatError::Conflict);}
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_seat_policy_triggers(domain_id,trigger_id,owner_seat_id,event_ref,state,pending_operation,pending_request_id,coordinator_receipt_id,revision) VALUES(?1,?2,?3,?4,'REGISTER_INTENT','REGISTER',?5,'',1)")?;
        q.bind_text(1,&caller.domain_id)?;q.bind_text(2,trigger_id)?;
        q.bind_text(3,&caller.seat_id)?;q.bind_text(4,event_ref)?;
        q.bind_text(5,request_id)?;q.step_done()?;
        record_event(db,&caller.domain_id,PolicyEvent {event_id:request_id.into(),
            operation:"trigger-register".into(),target_id:trigger_id.into(),
            policy_revision:expected_policy_revision,state:"REGISTER_INTENT".into(),
            detail:event_ref.into(),replayed:false},&fp)?;
        Ok(TriggerTransition {trigger_id:trigger_id.into(),state:"REGISTER_INTENT".into(),
            revision:1,external_action_authorized:true})
    })
}

pub(crate) fn begin_trigger_cancel(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    trigger_id:&str,request_id:&str,original_raw:&[u8],expected_revision:i64,
    expected_policy_revision:i64)->Result<TriggerTransition,SeatError> {
    validate(&caller.domain_id,trigger_id,request_id,original_raw)?;
    let fp=fingerprint(&["trigger-cancel",&caller.seat_id,trigger_id,
        &expected_revision.to_string(),&expected_policy_revision.to_string()],original_raw);
    transact(db,|db| {
        current_caller(db,caller)?;
        if let Some(old)=prior_event(db,&caller.domain_id,request_id,"trigger-cancel",&fp)? {
            let row=trigger_row(db,&caller.domain_id,trigger_id)?.ok_or(SeatError::SchemaDrift)?;
            if old.target_id!=trigger_id||row.0!=caller.seat_id {return Err(SeatError::Conflict);}
            return Ok(TriggerTransition {trigger_id:trigger_id.into(),state:row.2,
                revision:row.6,external_action_authorized:false});
        }
        if head_revision(db,&caller.domain_id)?!=expected_policy_revision {return Err(SeatError::Conflict);}
        let row=trigger_row(db,&caller.domain_id,trigger_id)?.ok_or(SeatError::Denied)?;
        if row.0!=caller.seat_id||row.2!="REGISTERED"||row.6!=expected_revision {
            return Err(SeatError::Denied);
        }
        let next=row.6.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_triggers SET state='CANCEL_INTENT',pending_operation='CANCEL',pending_request_id=?1,revision=?2 WHERE domain_id=?3 AND trigger_id=?4 AND state='REGISTERED' AND revision=?5")?;
        q.bind_text(1,request_id)?;q.bind_i64(2,next)?;
        q.bind_text(3,&caller.domain_id)?;q.bind_text(4,trigger_id)?;
        q.bind_i64(5,row.6)?;q.step_done()?;
        if trigger_row(db,&caller.domain_id,trigger_id)?.ok_or(SeatError::SchemaDrift)?.6!=next {
            return Err(SeatError::Conflict);
        }
        record_event(db,&caller.domain_id,PolicyEvent {event_id:request_id.into(),
            operation:"trigger-cancel".into(),target_id:trigger_id.into(),
            policy_revision:expected_policy_revision,state:"CANCEL_INTENT".into(),
            detail:String::new(),replayed:false},&fp)?;
        Ok(TriggerTransition {trigger_id:trigger_id.into(),state:"CANCEL_INTENT".into(),
            revision:next,external_action_authorized:true})
    })
}

/// A coordinator observation of the original trigger operation. The shared
/// connector must verify its receipt before constructing this native value.
pub(crate) struct NativeCoordinatorTriggerEvidence {
    domain_id:String,trigger_id:String,request_id:String,receipt_id:String,
    registered:bool,
}
impl NativeCoordinatorTriggerEvidence {
    pub(crate) fn from_verified_coordinator(domain:&str,trigger:&str,request:&str,
        receipt:&str,registered:bool)->Result<Self,SeatError> {
        if [domain,trigger,request,receipt].iter().any(|value|!valid_id(value)) {
            return Err(SeatError::Invalid("trigger receipt"));
        }
        Ok(Self {domain_id:domain.into(),trigger_id:trigger.into(),request_id:request.into(),
            receipt_id:receipt.into(),registered})
    }
}

pub(crate) fn settle_trigger(db:&mut VerifiedDatabaseConnection<'_>,
    evidence:&NativeCoordinatorTriggerEvidence)->Result<TriggerTransition,SeatError> {
    transact(db,|db| {
        let row=trigger_row(db,&evidence.domain_id,&evidence.trigger_id)?
            .ok_or(SeatError::Denied)?;
        let expected=if evidence.registered {"REGISTER"}else{"CANCEL"};
        let new_state=if evidence.registered {"REGISTERED"}else{"CANCELLED"};
        if row.2==new_state && row.5==evidence.receipt_id {
            return Ok(TriggerTransition {trigger_id:evidence.trigger_id.clone(),state:row.2,
                revision:row.6,external_action_authorized:false});
        }
        if row.3!=expected||row.4!=evidence.request_id ||
            !matches!(row.2.as_str(),"REGISTER_INTENT"|"CANCEL_INTENT"|"UNKNOWN") {
            return Err(SeatError::Unknown);
        }
        let next=row.6.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_triggers SET state=?1,pending_operation='NONE',coordinator_receipt_id=?2,revision=?3 WHERE domain_id=?4 AND trigger_id=?5 AND revision=?6")?;
        q.bind_text(1,new_state)?;q.bind_text(2,&evidence.receipt_id)?;
        q.bind_i64(3,next)?;q.bind_text(4,&evidence.domain_id)?;
        q.bind_text(5,&evidence.trigger_id)?;q.bind_i64(6,row.6)?;q.step_done()?;
        if trigger_row(db,&evidence.domain_id,&evidence.trigger_id)?
            .ok_or(SeatError::SchemaDrift)?.2!=new_state {return Err(SeatError::Conflict);}
        Ok(TriggerTransition {trigger_id:evidence.trigger_id.clone(),state:new_state.into(),
            revision:next,external_action_authorized:false})
    })
}

pub(crate) fn recover_trigger(db:&mut VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
    evidence:&NativeCoordinatorTriggerEvidence,expected_revision:i64,
    event_id:&str,original_raw:&[u8])->Result<TriggerTransition,SeatError> {
    validate(&caller.domain_id,&evidence.trigger_id,event_id,original_raw)?;
    if evidence.domain_id!=caller.domain_id||!evidence.registered {
        return Err(SeatError::Denied);
    }
    let fp=fingerprint(&["trigger-recover",&caller.seat_id,&evidence.trigger_id,
        &evidence.receipt_id,&expected_revision.to_string()],original_raw);
    transact(db,|db| {
        current_caller(db,caller)?;
        if let Some(old)=prior_event(db,&caller.domain_id,event_id,"trigger-recover",&fp)? {
            let row=trigger_row(db,&caller.domain_id,&evidence.trigger_id)?
                .ok_or(SeatError::SchemaDrift)?;
            if old.target_id!=evidence.trigger_id||row.0!=caller.seat_id {
                return Err(SeatError::Conflict);
            }
            return Ok(TriggerTransition {trigger_id:evidence.trigger_id.clone(),state:row.2,
                revision:row.6,external_action_authorized:false});
        }
        let row=trigger_row(db,&caller.domain_id,&evidence.trigger_id)?
            .ok_or(SeatError::Denied)?;
        if row.0!=caller.seat_id||row.2!="REGISTERED"||row.6!=expected_revision||
            row.5!=evidence.receipt_id {return Err(SeatError::Denied);}
        let next=row.6.checked_add(1).ok_or(SeatError::Conflict)?;
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_triggers SET revision=?1 WHERE domain_id=?2 AND trigger_id=?3 AND state='REGISTERED' AND revision=?4")?;
        q.bind_i64(1,next)?;q.bind_text(2,&caller.domain_id)?;
        q.bind_text(3,&evidence.trigger_id)?;q.bind_i64(4,row.6)?;q.step_done()?;
        if trigger_row(db,&caller.domain_id,&evidence.trigger_id)?
            .ok_or(SeatError::SchemaDrift)?.6!=next {return Err(SeatError::Conflict);}
        record_event(db,&caller.domain_id,PolicyEvent {event_id:event_id.into(),
            operation:"trigger-recover".into(),target_id:evidence.trigger_id.clone(),
            policy_revision:head_revision(db,&caller.domain_id)?,state:"REGISTERED".into(),
            detail:evidence.receipt_id.clone(),replayed:false},&fp)?;
        Ok(TriggerTransition {trigger_id:evidence.trigger_id.clone(),state:"REGISTERED".into(),
            revision:next,external_action_authorized:false})
    })
}

pub(crate) fn mark_trigger_unknown(db:&mut VerifiedDatabaseConnection<'_>,domain:&str,
    trigger_id:&str)->Result<(),SeatError> {
    if !valid_id(domain)||!valid_id(trigger_id) {return Err(SeatError::Invalid("trigger"));}
    transact(db,|db| {
        let row=trigger_row(db,domain,trigger_id)?.ok_or(SeatError::Denied)?;
        if !matches!(row.2.as_str(),"REGISTER_INTENT"|"CANCEL_INTENT") {
            return Err(SeatError::Unknown);
        }
        let q=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_triggers SET state='UNKNOWN',revision=revision+1 WHERE domain_id=?1 AND trigger_id=?2 AND revision=?3")?;
        q.bind_text(1,domain)?;q.bind_text(2,trigger_id)?;q.bind_i64(3,row.6)?;q.step_done()?;
        if trigger_row(db,domain,trigger_id)?.ok_or(SeatError::SchemaDrift)?.2!="UNKNOWN" {
            return Err(SeatError::Conflict);
        }
        Ok(())
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
    pub(crate) to_seat_id:String,pub(crate) reason:String,pub(crate) state:String,
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
        let trigger=trigger_row(db,&caller.domain_id,trigger_id)?.ok_or(SeatError::Denied)?;
        if trigger.0!=caller.seat_id||trigger.1!=cause.evidence_id()||
            trigger.2!="REGISTERED" {return Err(SeatError::Denied);}
        if let Some(old)=prior_event(db,&caller.domain_id,request_id,"escalate",&fp)? {
            let q=Statement::prepare(db.as_ptr(),
                "SELECT from_seat_id,to_seat_id,reason,state,revision FROM main.gogoke_v37_seat_policy_escalations WHERE domain_id=?1 AND trigger_id=?2 AND request_id=?3")?;
            q.bind_text(1,&caller.domain_id)?;q.bind_text(2,trigger_id)?;q.bind_text(3,request_id)?;
            if !q.step_row()? || old.target_id!=trigger_id {return Err(SeatError::Conflict);}
            let intent=EscalationIntent {trigger_id:trigger_id.into(),from_seat_id:q.column_text(0)?,
                to_seat_id:q.column_text(1)?,reason:q.column_text(2)?,state:q.column_text(3)?,
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
            to_seat_id:destination,reason:cause.reason().into(),state:"INTENT".into(),
            revision:1,replayed:false})
    })
}

/// This token can only be constructed after the existing C/H delivery path
/// verifies its original receipt and destination. It does not assert a send.
pub(crate) struct NativeDeliveryEvidence {
    domain_id:String,trigger_id:String,request_id:String,to_seat_id:String,receipt_id:String,
}
impl NativeDeliveryEvidence {
    pub(crate) fn from_verified_c_delivery(domain:&str,trigger:&str,request:&str,
        destination:&str,receipt:&str)->Result<Self,SeatError> {
        if [domain,trigger,request,destination,receipt].iter().any(|value|!valid_id(value)) {
            return Err(SeatError::Invalid("delivery evidence"));
        }
        Ok(Self {domain_id:domain.into(),trigger_id:trigger.into(),request_id:request.into(),
            to_seat_id:destination.into(),receipt_id:receipt.into()})
    }
}

/// Second transaction: only the exact original pending trigger can settle.
/// A missing or ambiguous external receipt remains UNKNOWN, never redelivered.
pub(crate) fn settle_escalation(db:&mut VerifiedDatabaseConnection<'_>,
    evidence:&NativeDeliveryEvidence)->Result<EscalationIntent,SeatError> {
    transact(db,|db| {
        let q=Statement::prepare(db.as_ptr(),
            "SELECT from_seat_id,to_seat_id,reason,state,revision,COALESCE(delivery_receipt_id,''),request_id FROM main.gogoke_v37_seat_policy_escalations WHERE domain_id=?1 AND trigger_id=?2")?;
        q.bind_text(1,&evidence.domain_id)?;q.bind_text(2,&evidence.trigger_id)?;
        if !q.step_row()? {return Err(SeatError::Denied);}
        let from=q.column_text(0)?;let to=q.column_text(1)?;let reason=q.column_text(2)?;
        let state=q.column_text(3)?;
        let revision=q.column_text(4)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)?;
        let old_receipt=q.column_text(5)?;
        let original_request=q.column_text(6)?;
        if q.step_row()? || to!=evidence.to_seat_id ||
            original_request!=evidence.request_id {return Err(SeatError::Denied);}
        if state=="DELIVERED" && old_receipt==evidence.receipt_id {
            return Ok(EscalationIntent {trigger_id:evidence.trigger_id.clone(),from_seat_id:from,
                to_seat_id:to,reason,state:"DELIVERED".into(),revision,replayed:true});
        }
        if !matches!(state.as_str(),"INTENT"|"UNKNOWN") || !old_receipt.is_empty() {
            return Err(SeatError::Unknown);
        }
        let next=revision.checked_add(1).ok_or(SeatError::Conflict)?;
        let update=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_seat_policy_escalations SET state='DELIVERED',delivery_receipt_id=?1,revision=?2 WHERE domain_id=?3 AND trigger_id=?4 AND request_id=?5 AND state IN ('INTENT','UNKNOWN') AND revision=?6")?;
        update.bind_text(1,&evidence.receipt_id)?;update.bind_i64(2,next)?;
        update.bind_text(3,&evidence.domain_id)?;update.bind_text(4,&evidence.trigger_id)?;
        update.bind_text(5,&evidence.request_id)?;update.bind_i64(6,revision)?;
        update.step_done()?;
        let verify=Statement::prepare(db.as_ptr(),
            "SELECT state,delivery_receipt_id,revision FROM main.gogoke_v37_seat_policy_escalations WHERE domain_id=?1 AND trigger_id=?2 AND request_id=?3")?;
        verify.bind_text(1,&evidence.domain_id)?;verify.bind_text(2,&evidence.trigger_id)?;
        verify.bind_text(3,&evidence.request_id)?;
        if !verify.step_row()? || verify.column_text(0)?!="DELIVERED" ||
            verify.column_text(1)?!=evidence.receipt_id ||
            verify.column_text(2)?.parse::<i64>().map_err(|_|SeatError::SchemaDrift)? != next ||
            verify.step_row()? {return Err(SeatError::Conflict);}
        Ok(EscalationIntent {trigger_id:evidence.trigger_id.clone(),from_seat_id:from,
            to_seat_id:to,reason,state:"DELIVERED".into(),revision:next,replayed:false})
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
