//! An H model call is a captured Codex server request from the current
//! physical User seat. The provider's arguments are selections, never a
//! principal, grant, source identity, or native operation receipt.

use super::{codex_rpc, decode_receipt, decode_request, rpc_journal, V37Status};
use crate::process::{OriginBoundFrame, PreparedCustody};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use crate::store::digest::sha256_hex;
use crate::store::ledger::{self, RawSourceKey, RawSourceRecord};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, NativeSeatCall, Seat, State};

#[derive(Debug)]
pub(crate) enum ModelCallError {
    Denied,
    Conflict,
    Store(AtomicError),
    Rpc(rpc_journal::RpcJournalError),
    Codec(codex_rpc::RpcError),
    Seat(seat::SeatError),
}
impl From<AtomicError> for ModelCallError { fn from(error:AtomicError)->Self {Self::Store(error)} }
impl From<rpc_journal::RpcJournalError> for ModelCallError {
    fn from(error:rpc_journal::RpcJournalError)->Self {Self::Rpc(error)}
}
impl From<codex_rpc::RpcError> for ModelCallError {
    fn from(error:codex_rpc::RpcError)->Self {Self::Codec(error)}
}
impl From<seat::SeatError> for ModelCallError {fn from(error:seat::SeatError)->Self {Self::Seat(error)}}
type Result<T> = std::result::Result<T,ModelCallError>;

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct ModelCallProof {
    custody:PreparedCustody,
    source:RawSourceKey,
    raw:Vec<u8>,
    raw_sha256:String,
    domain:String,
    session:String,
    operation:String,
    open_request_id:String,
    seat:String,
    incarnation:String,
    thread:String,
    turn:String,
    rpc_id:codex_rpc::RpcId,
    call_id:String,
    tool:String,
    host_request_id:String,
    arguments_json:String,
}
impl ModelCallProof {
    pub(crate) fn custody(&self)->&PreparedCustody {&self.custody}
    pub(crate) fn domain_id(&self)->&str {&self.domain}
    pub(crate) fn seat_id(&self)->&str {&self.seat}
    pub(crate) fn incarnation(&self)->&str {&self.incarnation}
    pub(crate) fn generation(&self)->i64 {self.custody.binding.generation.parse().unwrap_or(-1)}
    pub(crate) fn session_id(&self)->&str {&self.session}
    pub(crate) fn thread_id(&self)->&str {&self.thread}
    pub(crate) fn turn_id(&self)->&str {&self.turn}
    pub(crate) fn tool(&self)->&str {&self.tool}
    pub(crate) fn call_id(&self)->&str {&self.call_id}
    pub(crate) fn typed_rpc_id(&self)->&codex_rpc::RpcId {&self.rpc_id}
    pub(crate) fn raw_sha256(&self)->&str {&self.raw_sha256}
    pub(crate) fn host_request_id(&self)->&str {&self.host_request_id}
    pub(crate) fn raw_request_bytes(&self)->&[u8] {&self.raw}
    pub(crate) fn source(&self)->&RawSourceKey {&self.source}
    pub(crate) fn arguments_json(&self)->&str {&self.arguments_json}
}

fn unhex(value:&str)->Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(ModelCallError::Denied);}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|_|ModelCallError::Denied)?;
        u8::from_str_radix(text,16).map_err(|_|ModelCallError::Denied)
    }).collect()
}
fn object(raw:&[u8])->Result<std::collections::BTreeMap<JsonString,Json>> {
    let text=std::str::from_utf8(raw).map_err(|_|ModelCallError::Denied)?;
    let Json::Object(fields)=Parser::parse(text.trim_end_matches('\n'))?
        else {return Err(ModelCallError::Denied)};
    Ok(fields)
}
fn string(fields:&std::collections::BTreeMap<JsonString,Json>,name:&str)->Result<String> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::String(value))=>value.to_well_formed_string().ok_or(ModelCallError::Denied),
        _=>Err(ModelCallError::Denied),
    }
}
fn fields<'a>(fields:&'a std::collections::BTreeMap<JsonString,Json>,name:&str)
    ->Result<&'a std::collections::BTreeMap<JsonString,Json>> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::Object(value))=>Ok(value),_=>Err(ModelCallError::Denied),
    }
}
fn id(fields:&std::collections::BTreeMap<JsonString,Json>)->Result<codex_rpc::RpcId> {
    match fields.get(&JsonString::from_str("id")) {
        Some(Json::String(value))=>Ok(codex_rpc::RpcId::String(
            value.to_well_formed_string().ok_or(ModelCallError::Denied)?)),
        Some(Json::Number(value))=>Ok(codex_rpc::RpcId::Number(
            value.parse().map_err(|_|ModelCallError::Denied)?)),
        _=>Err(ModelCallError::Denied),
    }
}
fn original_user_turn(db:&VerifiedDatabaseConnection<'_>,proof:&ModelCallProof)->Result<()> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT request_id,request_hex,receipt_hex FROM main.gogoke_v37_h_stdin_journal
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND ticket=?4 AND custodian_nonce=?5 AND generation=?6
            AND operation='send' AND phase='RECEIPTED' AND receipt_status='APPLIED'")?;
    for (index,value) in [proof.domain.as_str(),proof.session.as_str(),
        proof.operation.as_str(),proof.custody.ticket.opaque(),
        proof.custody.custodian_nonce.as_str(),
        proof.custody.binding.generation.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    let mut found=false;
    while q.step_row()? {
        let request_id=q.column_text(0)?;
        let original=unhex(&q.column_text(1)?)?;
        let receipt=unhex(&q.column_text(2)?)?;
        let request=decode_request(&original).map_err(|_|ModelCallError::Denied)?;
        let receipt=decode_receipt(&receipt).map_err(|_|ModelCallError::Denied)?;
        if request.family!="K-SESSION" || request.operation!="send"
            || request.payload.len()!=2
            || request.domain_id!=proof.domain || request.target_id!=proof.session
            || request.request_id!=request_id || receipt.status!=V37Status::Applied
            || receipt.family!="K-SESSION" || receipt.operation!="send"
            || receipt.request_id!=request_id || receipt.target_id!=proof.session
            || receipt.previous_revision!=request.expected_revision
            || receipt.revision!=request.expected_revision.checked_add(1)
                .ok_or(ModelCallError::Denied)? {continue;}
        let result=receipt.into_result();
        if !matches!(result.get(&JsonString::from_str("turnId")),
            Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some(proof.turn.as_str()))
            || !matches!(result.get(&JsonString::from_str("createdTurn")),Some(Json::Bool(true))) {
            continue;
        }
        let text=string(&request.payload,"body")?;
        let step_id=format!("send-{}",&sha256_hex(&original)[..40]);
        let source=Statement::prepare(db.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes) FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id
                 AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
                AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.generation=?6
                AND s.open_request_id=?7 AND s.step_id=?8 AND s.phase='OBSERVED'
                AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
        for (index,value) in [proof.domain.as_str(),proof.session.as_str(),
            proof.operation.as_str(),proof.custody.ticket.opaque(),
            proof.custody.custodian_nonce.as_str(),proof.custody.binding.generation.as_str(),
            proof.open_request_id.as_str(),step_id.as_str()].iter().enumerate() {
            source.bind_text((index+1) as i32,value)?;
        }
        if !source.step_row()? {continue;}
        let encoded_command=unhex(&source.column_text(0)?)?;
        let response=unhex(&source.column_text(1)?)?;
        if source.step_row()? {return Err(ModelCallError::Conflict);}
        let command=object(&encoded_command)?;
        if string(&command,"method")?!="turn/start" {return Err(ModelCallError::Denied);}
        let command_id=id(&command)?;
        let params=fields(&command,"params")?;
        if string(params,"threadId")?!=proof.thread {return Err(ModelCallError::Denied);}
        let Some(Json::Array(input))=params.get(&JsonString::from_str("input"))
            else {return Err(ModelCallError::Denied)};
        let [Json::Object(message)]=input.as_slice() else {return Err(ModelCallError::Denied)};
        if string(message,"type")?!="text" || string(message,"text")?!=text {
            return Err(ModelCallError::Denied);
        }
        let response=object(&response)?;
        if id(&response)?!=command_id {return Err(ModelCallError::Denied);}
        let turn=fields(fields(&response,"result")?,"turn")?;
        if string(turn,"id")?!=proof.turn || string(turn,"status")?!="inProgress" {
            return Err(ModelCallError::Denied);
        }
        let receipt_identity=format!("{}\n{}\n{}\n{}",
            sha256_hex(&original),sha256_hex(&encoded_command),
            proof.operation,proof.custody.custodian_nonce);
        let expected=format!("rpc-{}",&sha256_hex(receipt_identity.as_bytes())[..40]);
        if !matches!(result.get(&JsonString::from_str("receiptId")),
            Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some(expected.as_str())) {
            return Err(ModelCallError::Denied);
        }
        if found {return Err(ModelCallError::Conflict);}
        found=true;
    }
    if found {Ok(())} else {Err(ModelCallError::Denied)}
}

fn current_turn_stream(db:&VerifiedDatabaseConnection<'_>,proof:&ModelCallProof)->Result<()> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT source_epoch,source_cursor,hex(raw_bytes) FROM main.v37_ledger_raw_source
          WHERE operation_id=?1 AND process_ticket=?2 AND custodian_nonce=?3
            AND domain_id=?4 AND session_id=?5 AND generation=?6
          ORDER BY source_epoch,CAST(source_cursor AS INTEGER)")?;
    for (index,value) in [proof.operation.as_str(),proof.custody.ticket.opaque(),
        proof.custody.custodian_nonce.as_str(),proof.domain.as_str(),
        proof.session.as_str(),proof.custody.binding.generation.as_str()]
        .iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    let call_cursor=proof.source.source_cursor.parse::<u64>().map_err(|_|ModelCallError::Denied)?;
    let mut started=false;
    while q.step_row()? {
        let epoch=q.column_text(0)?;
        let cursor=q.column_text(1)?.parse::<u64>().map_err(|_|ModelCallError::Denied)?;
        let raw=unhex(&q.column_text(2)?)?;
        let root=match object(&raw) {Ok(root)=>root,Err(_)=>continue};
        let method=string(&root,"method").unwrap_or_default();
        if method!="turn/started" && method!="turn/completed" {continue;}
        let reply=codex_rpc::decode(&raw,None).map_err(|_|ModelCallError::Denied)?;
        if let codex_rpc::Reply::TurnNotification {thread_id,turn_id,status,..}=reply {
            if thread_id==proof.thread && turn_id==proof.turn {
                if method=="turn/completed" {return Err(ModelCallError::Denied);}
                if status==codex_rpc::TurnStatus::InProgress
                    && epoch==proof.source.source_epoch && cursor<call_cursor {
                    started=true;
                }
            }
        }
    }
    if started {Ok(())} else {Err(ModelCallError::Denied)}
}

fn original_source(db:&VerifiedDatabaseConnection<'_>,proof:&ModelCallProof)->Result<()> {
    let (operation,open_id,seat_id,incarnation)=rpc_journal::current_codex_model_binding(
        db,&proof.custody,&proof.domain,&proof.session)?;
    if operation!=proof.operation || open_id!=proof.open_request_id
        || seat_id!=proof.seat || incarnation!=proof.incarnation {
        return Err(ModelCallError::Denied);
    }
    let source=ledger::read_captured_raw_source(db,&proof.source.operation_id,
        &proof.source.source_epoch,&proof.source.source_cursor)?
        .ok_or(ModelCallError::Denied)?;
    if source.raw_bytes!=proof.raw || sha256_hex(&source.raw_bytes)!=proof.raw_sha256
        || source.key.operation_id!=operation
        || source.process_ticket!=proof.custody.ticket.opaque()
        || source.custodian_nonce!=proof.custody.custodian_nonce
        || source.domain_id!=proof.domain || source.session_id!=proof.session
        || source.generation!=proof.custody.binding.generation {
        return Err(ModelCallError::Denied);
    }
    let call=codex_rpc::decode_dynamic_tool_call(&source.raw_bytes)?
        .ok_or(ModelCallError::Denied)?;
    if call.request_id!=proof.rpc_id || call.call_id!=proof.call_id
        || call.thread_id!=proof.thread || call.turn_id!=proof.turn
        || call.tool!=proof.tool || call.namespace.is_some()
        || call.arguments.canonical()!=proof.arguments_json {
        return Err(ModelCallError::Denied);
    }
    let actual=rpc_journal::observed_thread_id(db,&proof.domain,&proof.session,
        &operation,&proof.custody.binding.generation,&open_id,
        proof.custody.ticket.opaque(),&proof.custody.custodian_nonce)?;
    if actual!=proof.thread {return Err(ModelCallError::Denied);}
    original_user_turn(db,proof)?;
    current_turn_stream(db,proof)?;
    let duplicates=Statement::prepare(db.as_ptr(),
        "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source
          WHERE operation_id=?1 AND process_ticket=?2 AND custodian_nonce=?3
            AND domain_id=?4 AND session_id=?5 AND generation=?6")?;
    for (index,value) in [operation.as_str(),proof.custody.ticket.opaque(),
        proof.custody.custodian_nonce.as_str(),proof.domain.as_str(),
        proof.session.as_str(),proof.custody.binding.generation.as_str()]
        .iter().enumerate() {duplicates.bind_text((index+1) as i32,value)?;}
    while duplicates.step_row()? {
        let bytes=unhex(&duplicates.column_text(0)?)?;
        if let Ok(root)=object(&bytes) {
            if string(&root,"method").ok().as_deref()==Some("item/tool/call")
                && id(&root).ok().as_ref()==Some(&proof.rpc_id)
                && bytes!=proof.raw {
                return Err(ModelCallError::Conflict);
            }
        }
    }
    Ok(())
}

/// The Root calls this only after A durably captures this OriginBoundFrame.
/// It returns an opaque caller; no model-supplied identity or grant is used.
pub(crate) fn observe_model_call(db:&VerifiedDatabaseConnection<'_>,
    custody:&PreparedCustody,frame:&OriginBoundFrame,key:&RawSourceKey,
    expected_thread:&str,expected_turn:&str)->Result<NativeSeatCall> {
    if frame.custody()!=custody || key.operation_id.is_empty()
        || key.source_cursor.parse::<u64>().ok().filter(|n|*n>0
            && n.to_string()==key.source_cursor).is_none() {
        return Err(ModelCallError::Denied);
    }
    let source=ledger::read_captured_raw_source(db,&key.operation_id,
        &key.source_epoch,&key.source_cursor)?.ok_or(ModelCallError::Denied)?;
    if source.raw_bytes!=frame.bytes() || source.key!=*key
        || source.process_ticket!=custody.ticket.opaque()
        || source.custodian_nonce!=custody.custodian_nonce
        || source.domain_id!=custody.binding.domain_id
        || source.generation!=custody.binding.generation {
        return Err(ModelCallError::Denied);
    }
    build_from_captured_source(db,custody,key,expected_thread,expected_turn,source)
}

/// Durable recovery reads the already captured A row under the original
/// physical custody. It never reconstructs an OriginBoundFrame or pipe read.
pub(crate) fn recover_model_call_from_source(db:&VerifiedDatabaseConnection<'_>,
    custody:&PreparedCustody,key:&RawSourceKey,
    expected_thread:&str,expected_turn:&str)->Result<NativeSeatCall> {
    let source=ledger::read_captured_raw_source(db,&key.operation_id,
        &key.source_epoch,&key.source_cursor)?.ok_or(ModelCallError::Denied)?;
    if source.key!=*key || source.process_ticket!=custody.ticket.opaque()
        || source.custodian_nonce!=custody.custodian_nonce
        || source.domain_id!=custody.binding.domain_id
        || source.generation!=custody.binding.generation {
        return Err(ModelCallError::Denied);
    }
    build_from_captured_source(db,custody,key,expected_thread,expected_turn,source)
}

fn build_from_captured_source(db:&VerifiedDatabaseConnection<'_>,
    custody:&PreparedCustody,key:&RawSourceKey,
    expected_thread:&str,expected_turn:&str,source:RawSourceRecord)->Result<NativeSeatCall> {
    let call=codex_rpc::decode_dynamic_tool_call(&source.raw_bytes)?
        .ok_or(ModelCallError::Denied)?;
    if call.namespace.is_some() || call.thread_id!=expected_thread
        || call.turn_id!=expected_turn
        || !matches!(call.tool.as_str(),"gogoke_seat"|"gogoke_policy"|
            "gogoke_worktree"|"gogoke_takeover") {
        return Err(ModelCallError::Denied);
    }
    let (operation,open_request_id,seat,incarnation)=
        rpc_journal::current_codex_model_binding(db,custody,
            &source.domain_id,&source.session_id)?;
    if operation!=key.operation_id {return Err(ModelCallError::Denied);}
    let typed_id=match &call.request_id {
        codex_rpc::RpcId::Number(number)=>format!("n:{number}"),
        codex_rpc::RpcId::String(value)=>format!("s:{value}"),
    };
    let identity=format!("{}\n{}\n{}\n{}",operation,custody.ticket.opaque(),
        custody.custodian_nonce,typed_id);
    let proof=ModelCallProof {custody:custody.clone(),source:key.clone(),
        raw_sha256:sha256_hex(&source.raw_bytes),raw:source.raw_bytes,
        domain:source.domain_id,session:source.session_id,operation,open_request_id,
        seat,incarnation,thread:call.thread_id,turn:call.turn_id,
        rpc_id:call.request_id,call_id:call.call_id,tool:call.tool,
        host_request_id:format!("model-{}",&sha256_hex(identity.as_bytes())[..40]),
        arguments_json:call.arguments.canonical()};
    original_source(db,&proof)?;
    NativeSeatCall::from_model_proof(proof).map_err(ModelCallError::from)
}

/// E invokes this in each native transaction. It is a pure H/A read and must
/// never call E's current_caller, preventing an authority recursion.
pub(crate) fn revalidate_model_call_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    caller:&NativeSeatCall)->Result<Seat> {
    let proof=caller.model_proof().ok_or(ModelCallError::Denied)?;
    original_source(db,proof)?;
    let seat=seat::get(db,proof.domain_id(),proof.seat_id())?
        .ok_or(ModelCallError::Denied)?;
    if seat.state!=State::Busy || seat.incarnation!=proof.incarnation()
        || seat.generation!=proof.generation() || seat.instance_id.is_empty() {
        return Err(ModelCallError::Denied);
    }
    Ok(seat)
}

#[cfg(all(test, windows))]
#[path = "model_call_tests.rs"]
mod tests;
