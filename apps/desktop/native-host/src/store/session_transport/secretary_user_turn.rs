//! Recover a secretary tool call's original USER send from H's durable input,
//! its observed turn/start RPC and E's direct USER input marker. The model
//! supplies none of these locators.
use super::{decode_receipt,decode_request,model_call,V37Status};
use crate::store::atomic::{AtomicError,Json,JsonString,Parser,Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::NativeSeatCall;

#[derive(Debug)]
pub(crate) enum UserTurnError {Denied,Ambiguous,Store(AtomicError)}
impl From<AtomicError> for UserTurnError {fn from(error:AtomicError)->Self {Self::Store(error)}}

pub(crate) struct OriginalSecretaryUserTurn {
    pub(crate) body:String,
    pub(crate) source_operation_id:String,
    pub(crate) source_epoch:String,
    pub(crate) source_cursor:String,
    pub(crate) request_id:String,
    pub(crate) request_bytes:Vec<u8>,
}

fn unhex(value:&str)->Result<Vec<u8>,UserTurnError> {
    if value.len()%2!=0 {return Err(UserTurnError::Denied);}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let pair=std::str::from_utf8(pair).map_err(|_|UserTurnError::Denied)?;
        u8::from_str_radix(pair,16).map_err(|_|UserTurnError::Denied)
    }).collect()
}
fn object(raw:&[u8])->Result<std::collections::BTreeMap<JsonString,Json>,UserTurnError> {
    let text=std::str::from_utf8(raw).map_err(|_|UserTurnError::Denied)?;
    match Parser::parse(text.trim_end_matches('\n')).map_err(|_|UserTurnError::Denied)? {
        Json::Object(fields)=>Ok(fields),_=>Err(UserTurnError::Denied),
    }
}
fn field(fields:&std::collections::BTreeMap<JsonString,Json>,name:&str)->Result<String,UserTurnError> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::String(value))=>value.to_well_formed_string().ok_or(UserTurnError::Denied),
        _=>Err(UserTurnError::Denied),
    }
}
fn fields<'a>(fields:&'a std::collections::BTreeMap<JsonString,Json>,name:&str)
    ->Result<&'a std::collections::BTreeMap<JsonString,Json>,UserTurnError> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::Object(value))=>Ok(value),_=>Err(UserTurnError::Denied),
    }
}
fn rpc_id(fields:&std::collections::BTreeMap<JsonString,Json>)->Result<String,UserTurnError> {
    match fields.get(&JsonString::from_str("id")) {
        Some(Json::String(value))=>Ok(format!("s:{}",value.to_well_formed_string().ok_or(UserTurnError::Denied)?)),
        Some(Json::Number(value))=>Ok(format!("n:{value}")),
        _=>Err(UserTurnError::Denied),
    }
}

/// Must be called within the caller's BEGIN IMMEDIATE. The model proof is
/// revalidated against current H custody, E BUSY generation and turn first.
pub(crate) fn read_original_user_turn_in_transaction(
    db:&VerifiedDatabaseConnection<'_>,caller:&NativeSeatCall,
)->Result<OriginalSecretaryUserTurn,UserTurnError> {
    model_call::revalidate_model_call_in_transaction(db,caller)
        .map_err(|_|UserTurnError::Denied)?;
    let proof=caller.model_proof().ok_or(UserTurnError::Denied)?;
    if proof.domain_id()!="global" || proof.tool()!="gogoke_routine" {
        return Err(UserTurnError::Denied);
    }
    let source=proof.source();
    let call_cursor=source.source_cursor.parse::<u64>().map_err(|_|UserTurnError::Denied)?;
    let journal=Statement::prepare(db.as_ptr(),
        "SELECT request_id,request_hex,receipt_hex FROM main.gogoke_v37_h_stdin_journal
         WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
           AND ticket=?4 AND custodian_nonce=?5 AND generation=?6
           AND operation='send' AND phase='RECEIPTED' AND receipt_status='APPLIED'")?;
    for (index,value) in [proof.domain_id(),proof.session_id(),source.operation_id.as_str(),
        proof.custody().ticket.opaque(),proof.custody().custodian_nonce.as_str(),
        proof.physical_generation()].iter().enumerate() {
        journal.bind_text((index+1) as i32,value)?;
    }
    let mut found=None;
    while journal.step_row()? {
        let request_id=journal.column_text(0)?;
        let original=unhex(&journal.column_text(1)?)?;
        let receipt=unhex(&journal.column_text(2)?)?;
        let request=decode_request(&original).map_err(|_|UserTurnError::Denied)?;
        let receipt=decode_receipt(&receipt).map_err(|_|UserTurnError::Denied)?;
        if request.family!="K-SESSION" || request.operation!="send"
            || request.payload.len()!=2 || request.domain_id!="global"
            || request.target_id!=proof.session_id() || request.request_id!=request_id
            || receipt.status!=V37Status::Applied || receipt.family!="K-SESSION"
            || receipt.operation!="send" || receipt.request_id!=request_id
            || receipt.target_id!=proof.session_id()
            || receipt.previous_revision!=request.expected_revision
            || receipt.revision!=request.expected_revision.checked_add(1).ok_or(UserTurnError::Denied)? {
            continue;
        }
        let result=receipt.into_result();
        if !matches!(result.get(&JsonString::from_str("turnId")),
            Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some(proof.turn_id()))
            || !matches!(result.get(&JsonString::from_str("createdTurn")),Some(Json::Bool(true))) {
            continue;
        }
        let body=field(&request.payload,"body")?;
        if body.is_empty() || body.contains('\0') {return Err(UserTurnError::Denied);}
        let marker_id=format!("H-USER:global:{}:{}",proof.session_id(),request_id);
        let marker=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_secretary_presence
             WHERE source_id=?1 AND kind='INPUT' AND source_operation_id=?2
               AND source_epoch=?3 AND source_cursor=?4")?;
        for (index,value) in [marker_id.as_str(),source.operation_id.as_str(),
            proof.custody().custodian_nonce.as_str(),request_id.as_str()].iter().enumerate() {
            marker.bind_text((index+1) as i32,value)?;
        }
        if !marker.step_row()? || marker.step_row()? {continue;}
        let step_id=format!("send-{}",&sha256_hex(&original)[..40]);
        let observed=Statement::prepare(db.as_ptr(),
            "SELECT s.command_hex,s.source_epoch,s.source_cursor,hex(r.raw_bytes)
             FROM main.gogoke_v37_rpc_steps s JOIN main.v37_ledger_raw_source r
              ON r.operation_id=s.process_operation_id AND r.source_epoch=s.source_epoch
               AND r.source_cursor=s.source_cursor AND r.process_ticket=s.ticket
               AND r.custodian_nonce=s.custodian_nonce AND r.domain_id=s.domain_id
               AND r.session_id=s.session_id AND r.generation=s.generation
             WHERE s.domain_id='global' AND s.session_id=?1 AND s.process_operation_id=?2
               AND s.ticket=?3 AND s.custodian_nonce=?4 AND s.generation=?5
               AND s.step_id=?6 AND s.phase='OBSERVED'
               AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
        for (index,value) in [proof.session_id(),source.operation_id.as_str(),
            proof.custody().ticket.opaque(),proof.custody().custodian_nonce.as_str(),
            proof.physical_generation(),step_id.as_str()].iter().enumerate() {
            observed.bind_text((index+1) as i32,value)?;
        }
        if !observed.step_row()? {continue;}
        let command=unhex(&observed.column_text(0)?)?;
        let epoch=observed.column_text(1)?;
        let cursor=observed.column_text(2)?;
        let response=unhex(&observed.column_text(3)?)?;
        if observed.step_row()? {return Err(UserTurnError::Ambiguous);}
        let response_cursor=cursor.parse::<u64>().map_err(|_|UserTurnError::Denied)?;
        if epoch!=source.source_epoch || response_cursor>=call_cursor {continue;}
        let command_fields=object(&command)?;
        if field(&command_fields,"method")?!="turn/start" {return Err(UserTurnError::Denied);}
        let command_id=rpc_id(&command_fields)?;
        let params=fields(&command_fields,"params")?;
        if field(params,"threadId")?!=proof.thread_id() {return Err(UserTurnError::Denied);}
        let Some(Json::Array(input))=params.get(&JsonString::from_str("input")) else {return Err(UserTurnError::Denied)};
        let [Json::Object(message)]=input.as_slice() else {return Err(UserTurnError::Denied)};
        if field(message,"type")?!="text" || field(message,"text")?!=body {return Err(UserTurnError::Denied);}
        let response_fields=object(&response)?;
        if rpc_id(&response_fields)?!=command_id {return Err(UserTurnError::Denied);}
        let turn=fields(fields(&response_fields,"result")?,"turn")?;
        if field(turn,"id")?!=proof.turn_id() || field(turn,"status")?!="inProgress" {
            return Err(UserTurnError::Denied);
        }
        let receipt_identity=format!("{}\n{}\n{}\n{}",sha256_hex(&original),
            sha256_hex(&command),source.operation_id,proof.custody().custodian_nonce);
        let expected=format!("rpc-{}",&sha256_hex(receipt_identity.as_bytes())[..40]);
        if !matches!(result.get(&JsonString::from_str("receiptId")),
            Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some(expected.as_str())) {
            return Err(UserTurnError::Denied);
        }
        if found.is_some() {return Err(UserTurnError::Ambiguous);}
        found=Some(OriginalSecretaryUserTurn {body,source_operation_id:source.operation_id.clone(),
            source_epoch:epoch,source_cursor:cursor,request_id,request_bytes:original});
    }
    found.ok_or(UserTurnError::Denied)
}
