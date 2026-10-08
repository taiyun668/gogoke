//! C native question cards backed by exact Codex 0.149 server requests.
//!
//! The first source is A's captured provider frame under H custody. C stores
//! a complete canonical question payload and a native source descriptor;
//! neither a Node claim nor a successful pipe write is a provider ACK.
use super::*;
use crate::store::atomic::Parser;
use crate::store::inbox::{self, CardEnvelope, InboxError, NativeAnswer,
    NativeAnswerDisposition, NativeAnswerIntent, NativeAnswerShape,
    NativeCardOperation, NativeQuestion, NativeQuestionCard, NativeQuestionOption};
use crate::store::ledger::{self, RawSourceKey, RawSourceRecord};
use crate::store::seat::NativeOrigin;
use crate::store::session_transport::{codex_rpc::{self, Command, QuestionCard, Reply, RpcId}, runtime};
use crate::store::session_transport::{self, provider_evidence::claude_question,
    rpc_journal as rpc};
use crate::store::digest::sha256_hex;
use std::collections::{BTreeMap, BTreeSet};

fn store_error(scope: &'static str, error: impl std::fmt::Debug) -> OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("native qcard {scope}: {error:?}"))
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    if value.len()%2!=0 || value.is_empty() {
        return Err(OrchestrationError::V37StoreFailure("native qcard descriptor hex length".into()));
    }
    let nibble=|byte:u8| match byte {b'0'..=b'9'=>Some(byte-b'0'),b'a'..=b'f'=>Some(byte-b'a'+10),_=>None};
    value.as_bytes().chunks_exact(2).enumerate().map(|(index,pair)| {
        let hi=nibble(pair[0]).ok_or_else(||OrchestrationError::V37StoreFailure(
            format!("native qcard descriptor invalid hex at offset {}",index*2)))?;
        let lo=nibble(pair[1]).ok_or_else(||OrchestrationError::V37StoreFailure(
            format!("native qcard descriptor invalid hex at offset {}",index*2+1)))?;
        Ok((hi<<4)|lo)
    }).collect()
}
fn json_field<'a>(fields:&'a BTreeMap<JsonString,Json>,name:&str)->Result<&'a Json> {
    fields.get(&JsonString::from_str(name)).ok_or(OrchestrationError::OperationConflict)
}
fn json_object<'a>(value:&'a Json)->Result<&'a BTreeMap<JsonString,Json>> {
    match value {Json::Object(fields)=>Ok(fields),_=>Err(OrchestrationError::OperationConflict)}
}
fn json_text(fields:&BTreeMap<JsonString,Json>,name:&str)->Result<String> {
    let Json::String(value)=json_field(fields,name)? else {return Err(OrchestrationError::OperationConflict);};
    value.to_well_formed_string().filter(|value|!value.is_empty()&&!value.contains('\0'))
        .ok_or(OrchestrationError::OperationConflict)
}
fn request_id_wire(id:&RpcId)->String {
    match id {RpcId::Number(value)=>Json::Number(value.to_string()).canonical(),
        RpcId::String(value)=>Json::String(JsonString::from_str(value)).canonical()}
}
fn request_id_from_wire(value:&str)->Result<RpcId> {
    let parsed=Parser::parse(value)?;
    if parsed.canonical()!=value {return Err(OrchestrationError::OperationConflict);}
    match parsed {
        Json::Number(value)=>value.parse::<i64>().map(RpcId::Number)
            .map_err(|_|OrchestrationError::OperationConflict),
        Json::String(value)=>value.to_well_formed_string().map(RpcId::String)
            .ok_or(OrchestrationError::OperationConflict),
        _=>Err(OrchestrationError::OperationConflict),
    }
}
fn source_descriptor(source:&RawSourceRecord)->String {
    Json::Object(BTreeMap::from([
        (JsonString::from_str("operationId"),Json::String(JsonString::from_str(&source.key.operation_id))),
        (JsonString::from_str("sourceEpoch"),Json::String(JsonString::from_str(&source.key.source_epoch))),
        (JsonString::from_str("sourceCursor"),Json::String(JsonString::from_str(&source.key.source_cursor))),
        (JsonString::from_str("frameSha256"),Json::String(JsonString::from_str(&sha256_hex(&source.raw_bytes)))),
    ])).canonical()
}
pub(super) fn read_source_descriptor(db:&VerifiedDatabaseConnection<'_>,domain:&str,card_id:&str)
    ->Result<(RawSourceKey,String)> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT request_hex FROM main.gogoke_v37_qcard_native_operations
         WHERE domain_id=?1 AND card_id=?2 AND state='RAISED'")?;
    q.bind_text(1,domain)?;q.bind_text(2,card_id)?;
    if !q.step_row()? {return Err(OrchestrationError::OperationConflict);}
    let descriptor=unhex(&q.column_text(0)?)?;
    if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
    let text=std::str::from_utf8(&descriptor).map_err(|error|
        store_error("source descriptor UTF-8",error))?;
    let parsed=Parser::parse(text)?;
    if parsed.canonical()!=text {return Err(OrchestrationError::OperationConflict);}
    let Json::Object(fields)=parsed else {
        return Err(OrchestrationError::OperationConflict);
    };
    if fields.len()!=4 {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok((RawSourceKey {operation_id:json_text(&fields,"operationId")?,
        source_epoch:json_text(&fields,"sourceEpoch")?,
        source_cursor:json_text(&fields,"sourceCursor")?},
        json_text(&fields,"frameSha256")?))
}
fn card_identity(domain:&str,session:&str,source:&RawSourceKey)->(String,String) {
    let basis=format!("{domain}\n{session}\n{}\n{}\n{}",source.operation_id,source.source_epoch,source.source_cursor);
    let digest=sha256_hex(basis.as_bytes());
    (format!("card{digest}"),format!("raise{digest}"))
}

struct CurrentCardBinding {
    domain:String,session:String,seat:String,generation:String,operation:String,
    ticket:String,nonce:String,thread:String,turn:String,
}
fn native_binding_present(db:&VerifiedDatabaseConnection<'_>,binding:&CurrentCardBinding,captured_only:bool)
    -> std::result::Result<bool,InboxError> {
    if !captured_only && crate::store::session_transport::generation_change::active_for_session(db,&binding.domain,&binding.session)?.is_some() {return Ok(false);}
    let relationship=crate::store::session_transport::session_binding::current_relationship(
        db,&binding.domain,&binding.session)
        .map_err(|error|InboxError::InvalidEvidence(format!("question H/E relationship: {error:?}")))?;
    let Some(relationship)=relationship else {return Ok(false);};
    if relationship.seat_id!=binding.seat || relationship.session_generation!=binding.generation {
        return Ok(false);
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_claim h
         JOIN main.gogoke_v37_h_process_episode p ON p.process_operation_id=h.process_operation_id
           AND p.domain_id=h.domain_id AND p.session_id=h.session_id AND p.generation=h.generation
           AND p.phase IN ('ACTIVE','UNKNOWN')
         JOIN main.gogoke_coordination_process_custody c ON c.operation_id=h.process_operation_id
           AND c.domain_id=h.domain_id AND c.generation=h.generation
         JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=h.binding_id
           AND b.instance_id=h.instance_id AND b.domain_id=h.domain_id AND b.kind='SESSION'
           AND b.owner_id=h.session_id AND b.generation=h.generation AND b.state='ACTIVE'
         WHERE h.domain_id=?1 AND h.session_id=?2 AND p.seat_id=?3 AND h.generation=?4
           AND h.process_operation_id=?5 AND c.ticket=?6 AND c.custodian_nonce=?7
           AND h.state='COMMITTED' AND (c.state='ACTIVE' OR (?8=1 AND c.state='UNKNOWN'))")?;
    for (index,value) in [binding.domain.as_str(),binding.session.as_str(),binding.seat.as_str(),
        binding.generation.as_str(),binding.operation.as_str(),binding.ticket.as_str(),
        binding.nonce.as_str()].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    q.bind_i64(8,i64::from(captured_only))?;
    Ok(q.step_row()? && !q.step_row()?)
}
fn source_matches(source:&RawSourceRecord,binding:&CurrentCardBinding)->bool {
    source.key.operation_id==binding.operation && source.key.source_epoch==binding.nonce
        && source.process_ticket==binding.ticket && source.custodian_nonce==binding.nonce
        && source.domain_id==binding.domain && source.session_id==binding.session
        && source.generation==binding.generation
}
fn claude_send_present(db:&VerifiedDatabaseConnection<'_>,binding:&CurrentCardBinding)->Result<bool> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT h.request_hex FROM main.gogoke_v37_h_stdin_journal h
          WHERE h.domain_id=?1 AND h.session_id=?2 AND h.request_id=?3
            AND h.operation='send' AND h.process_operation_id=?4
            AND h.ticket=?5 AND h.custodian_nonce=?6 AND h.generation=?7
            AND h.phase IN ('PREPARED','RECEIPTED')")?;
    for (index,value) in [binding.domain.as_str(),binding.session.as_str(),
        binding.turn.as_str(),binding.operation.as_str(),binding.ticket.as_str(),
        binding.nonce.as_str(),binding.generation.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Ok(false);}
    let bytes=unhex(&q.column_text(0)?)?;
    if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
    drop(q);
    let request=session_transport::decode_request(&bytes).map_err(|error|
        store_error("Claude H send request",error))?;
    if request.family!="K-SESSION" || request.operation!="send"
        || request.request_id!=binding.turn || request.domain_id!=binding.domain
        || request.target_id!=binding.session {
        return Ok(false);
    }
    let digest=sha256_hex(&bytes);
    let step_id=format!("claude-send-{}",&digest[..40]);
    let step=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1
          AND session_id=?2 AND step_id=?3 AND process_operation_id=?4
          AND ticket=?5 AND custodian_nonce=?6 AND generation=?7
          AND phase='OBSERVED' AND requires_response=1")?;
    for (index,value) in [binding.domain.as_str(),binding.session.as_str(),
        step_id.as_str(),binding.operation.as_str(),binding.ticket.as_str(),
        binding.nonce.as_str(),binding.generation.as_str()].iter().enumerate() {
        step.bind_text((index+1) as i32,value)?;
    }
    Ok(step.step_row()? && !step.step_row()?)
}
fn decoded_question(source:&RawSourceRecord,binding:&CurrentCardBinding)->Result<(QuestionCard,String)> {
    if !source_matches(source,binding) {return Err(OrchestrationError::OperationConflict);}
    let Reply::Question(card)=codex_rpc::decode(&source.raw_bytes,None).map_err(|error|store_error("provider question",error))?
        else {return Err(OrchestrationError::OperationConflict);};
    if card.thread_id!=binding.thread || card.turn_id!=binding.turn {
        return Err(OrchestrationError::OperationConflict);
    }
    let Json::Object(frame)=Parser::parse(std::str::from_utf8(&source.raw_bytes).map_err(|error|
        store_error("provider UTF-8",error))?)? else {return Err(OrchestrationError::OperationConflict);};
    let params=json_field(&frame,"params")?.canonical();
    Ok((card,params))
}
fn stored_source(db:&VerifiedDatabaseConnection<'_>,binding:&CurrentCardBinding,
    card:&NativeQuestionCard)->Result<RawSourceRecord> {
    let (source_key,digest)=read_source_descriptor(db,&binding.domain,&card.card_id)?;
    if card_identity(&binding.domain,&binding.session,&source_key).0!=card.card_id {
        return Err(OrchestrationError::OperationConflict);
    }
    let source=ledger::read_captured_raw_source(db,&source_key.operation_id,
        &source_key.source_epoch,&source_key.source_cursor)?
        .ok_or(OrchestrationError::OperationConflict)?;
    if sha256_hex(&source.raw_bytes)!=digest {return Err(OrchestrationError::OperationConflict);}
    let (original,payload)=decoded_question(&source,binding)?;
    if payload!=card.question_payload || request_id_wire(&original.request_id)!=card.vendor_request_id
        || original.item_id!=card.vendor_item_id || original.thread_id!=card.vendor_thread_id
        || original.turn_id!=card.turn_id {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok(source)
}
fn stored_claude_source(db:&VerifiedDatabaseConnection<'_>,binding:&CurrentCardBinding,
    card:&NativeQuestionCard)->Result<RawSourceRecord> {
    let (source_key,digest)=read_source_descriptor(db,&binding.domain,&card.card_id)?;
    if card_identity(&binding.domain,&binding.session,&source_key).0!=card.card_id
        || !claude_send_present(db,binding)? {
        return Err(OrchestrationError::OperationConflict);
    }
    let source=ledger::read_captured_raw_source(db,&source_key.operation_id,
        &source_key.source_epoch,&source_key.source_cursor)?
        .ok_or(OrchestrationError::OperationConflict)?;
    if !source_matches(&source,binding) || sha256_hex(&source.raw_bytes)!=digest {
        return Err(OrchestrationError::OperationConflict);
    }
    let original=claude_question::decode(&source.raw_bytes).map_err(|error|
        store_error("Claude stored question",error))?
        .ok_or(OrchestrationError::OperationConflict)?;
    let first=original.questions().first().ok_or(OrchestrationError::OperationConflict)?;
    if card.question_payload!=original.display_payload()
        || card.vendor_request_id!=Json::String(JsonString::from_str(original.request_id())).canonical()
        || card.vendor_item_id!=original.tool_use_id()
        || card.vendor_thread_id!=binding.thread || card.turn_id!=binding.turn
        || card.question_id!="host0" || card.header!=first.header
        || card.question!=first.question || card.seat_id!=binding.seat
        || card.generation!=binding.generation || !card.auto_resolution_ms.is_empty() {
        return Err(OrchestrationError::OperationConflict);
    }
    if card.options.len()!=first.options.len() || card.options.iter().enumerate().any(|(index,option)|
        option.id!=format!("option{index}") || option.label!=first.options[index].label
            || option.description!=first.options[index].description) {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok(source)
}
/// C and A are read in the same C query transaction. A's own recovery reader
/// validates the H claim/custody state and the stop proof; this checks the
/// original C/H/E/Owner relationship without granting a new live turn.
fn recovered_card_present(db:&VerifiedDatabaseConnection<'_>,key:&(String,String),card_id:&str)
    ->std::result::Result<bool,InboxError> {
    let row=Statement::prepare(db.as_ptr(),
        "SELECT vendor_request_id,vendor_thread_id,vendor_item_id,question_payload,
                question_id,question_header,question_text,seat_id,turn_id,generation
         FROM main.gogoke_v37_qcard_native WHERE domain_id=?1 AND card_id=?2")?;
    row.bind_text(1,&key.0)?;row.bind_text(2,card_id)?;
    if !row.step_row()? {return Ok(true);}
    let vendor_request=row.column_text(0)?;
    let thread=row.column_text(1)?;
    let item=row.column_text(2)?;
    let payload=row.column_text(3)?;
    let question_id=row.column_text(4)?;
    let header=row.column_text(5)?;
    let question_text=row.column_text(6)?;
    let seat=row.column_text(7)?;
    let turn=row.column_text(8)?;
    let generation=row.column_text(9)?;
    if row.step_row()? {return Err(InboxError::Conflict);}
    drop(row);
    let (source_key,digest)=read_source_descriptor(db,&key.0,card_id)
        .map_err(|error|InboxError::InvalidEvidence(format!("C source descriptor: {error:?}")))?;
    if card_identity(&key.0,&key.1,&source_key).0!=card_id {return Ok(false);}
    let h=Statement::prepare(db.as_ptr(),
        "SELECT h.process_operation_id,c.ticket,c.custodian_nonce
         FROM main.gogoke_v37_h_process_episode h
         JOIN main.gogoke_coordination_process_custody c
           ON c.operation_id=h.process_operation_id AND c.domain_id=h.domain_id
           AND c.generation=h.generation
         JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=h.binding_id
           AND b.instance_id=h.instance_id AND b.domain_id=h.domain_id
           AND b.kind='SESSION' AND b.owner_id=h.session_id AND b.generation=h.generation
         JOIN main.gogoke_v37_seats e ON e.domain_id=h.domain_id AND e.seat_id=h.seat_id
           AND e.incarnation=h.seat_incarnation
           AND e.instance_id=h.instance_id AND e.state IN ('BUSY','IDLE')
         WHERE h.domain_id=?1 AND h.session_id=?2 AND h.seat_id=?3
           AND h.generation=?4 AND h.process_operation_id=?5
           AND ((h.phase IN ('PREPARED','ACTIVE','UNKNOWN') AND b.state='ACTIVE'
                  AND (c.state IN ('ACTIVE','UNKNOWN')
                    OR (c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL)))
                OR (h.phase='STOPPED' AND c.state='STOPPED' AND h.stop_fact_id=c.stop_proof_hash
                    AND b.state IN ('ACTIVE','REVOKED')))")?;
    for (index,value) in [key.0.as_str(),key.1.as_str(),seat.as_str(),generation.as_str(),
        source_key.operation_id.as_str()].iter().enumerate() {
        h.bind_text((index+1) as i32,value)?;
    }
    if !h.step_row()? {return Ok(false);}
    let binding=CurrentCardBinding {domain:key.0.clone(),session:key.1.clone(),seat,
        generation,operation:h.column_text(0)?,ticket:h.column_text(1)?,
        nonce:h.column_text(2)?,thread,turn};
    if h.step_row()? {return Err(InboxError::Conflict);}
    drop(h);
    if source_key.source_epoch!=binding.nonce {return Ok(false);}
    let Some(source)=ledger::read_captured_raw_source(db,&source_key.operation_id,
        &source_key.source_epoch,&source_key.source_cursor)? else {return Ok(false);};
    if !source_matches(&source,&binding) || sha256_hex(&source.raw_bytes)!=digest {
        return Ok(false);
    }
    if matches!(Parser::parse(&payload),Ok(Json::Object(ref fields))
        if matches!(fields.get(&JsonString::from_str("provider")),
            Some(Json::String(provider)) if provider.to_well_formed_string().as_deref()==Some("claude"))) {
        if !claude_send_present(db,&binding).map_err(|error|
            InboxError::InvalidEvidence(format!("Claude original H send: {error:?}")))? {
            return Ok(false);
        }
        let Some(original)=claude_question::decode(&source.raw_bytes).map_err(|error|
            InboxError::InvalidEvidence(format!("A Claude question: {error:?}")))? else {
            return Ok(false);
        };
        let Some(first)=original.questions().first() else {return Ok(false);};
        if payload!=original.display_payload() || vendor_request!=Json::String(JsonString::from_str(original.request_id())).canonical()
            || item!=original.tool_use_id() || question_id!="host0"
            || first.header!=header || first.question!=question_text {
            return Ok(false);
        }
        let options=Statement::prepare(db.as_ptr(),
            "SELECT option_id,label,description FROM main.gogoke_v37_qcard_native_options
             WHERE domain_id=?1 AND card_id=?2 ORDER BY CAST(ordinal AS INTEGER),option_id")?;
        options.bind_text(1,&key.0)?;options.bind_text(2,card_id)?;
        for (index,choice) in first.options.iter().enumerate() {
            if !options.step_row()? || options.column_text(0)?!=format!("option{index}")
                || options.column_text(1)?!=choice.label
                || options.column_text(2)?!=choice.description {
                return Ok(false);
            }
        }
        return Ok(!options.step_row()?);
    }
    let (original,actual_payload)=decoded_question(&source,&binding)
        .map_err(|error|InboxError::InvalidEvidence(format!("A original question: {error:?}")))?;
    let Some(first)=original.questions.first() else {return Ok(false);};
    if actual_payload!=payload || request_id_wire(&original.request_id)!=vendor_request
        || original.item_id!=item || first.id!=question_id || first.header!=header
        || first.question!=question_text {return Ok(false);}
    let options=Statement::prepare(db.as_ptr(),
        "SELECT option_id,label,description FROM main.gogoke_v37_qcard_native_options
         WHERE domain_id=?1 AND card_id=?2 ORDER BY CAST(ordinal AS INTEGER),option_id")?;
    options.bind_text(1,&key.0)?;options.bind_text(2,card_id)?;
    let expected=first.options.as_ref().map(|options|options.as_slice()).unwrap_or(&[]);
    for (index,(label,description)) in expected.iter().enumerate() {
        if !options.step_row()? || options.column_text(0)?!=format!("option{index}")
            || options.column_text(1)?!=label.as_str()
            || options.column_text(2)?!=description.as_str() {
            return Ok(false);
        }
    }
    Ok(!options.step_row()?)
}
fn answer_command(card:&NativeQuestionCard,answers:BTreeMap<String,Vec<String>>)->Result<Command> {
    let Json::Object(params)=Parser::parse(&card.question_payload)? else {
        return Err(OrchestrationError::OperationConflict);
    };
    if json_text(&params,"threadId")?!=card.vendor_thread_id
        || json_text(&params,"turnId")?!=card.turn_id
        || json_text(&params,"itemId")?!=card.vendor_item_id {
        return Err(OrchestrationError::OperationConflict);
    }
    let Json::Array(questions)=json_field(&params,"questions")? else {
        return Err(OrchestrationError::OperationConflict);
    };
    if questions.is_empty() {return Err(OrchestrationError::OperationConflict);}
    let mut expected=BTreeSet::new();
    for question in questions {
        let id=json_text(json_object(question)?,"id")?;
        if !expected.insert(id) {return Err(OrchestrationError::OperationConflict);}
    }
    if expected.len()!=answers.len() || answers.keys().any(|key|!expected.contains(key))
        || answers.values().any(|values|values.is_empty() || values.iter().any(|value|
            value.is_empty() || value.contains('\0'))) {
        return Err(OrchestrationError::OperationConflict);
    }
    let command=Command::QuestionAnswer {request_id:request_id_from_wire(&card.vendor_request_id)?,answers};
    command.encode(None).map_err(|error|store_error("answer encoding",error))?;
    Ok(command)
}

pub(super) struct RaisedCodexCard {
    pub(super) card_id:String,
    pub(super) operation:NativeCardOperation,
}
pub(super) struct CodexAnswerWrite {
    pub(super) card_id:String,
    pub(super) request_id:String,
    pub(super) step_id:String,
    pub(super) operation:NativeCardOperation,
    pub(super) newly_written:bool,
}
pub(super) struct ClaudeAnswerWrite {
    pub(super) card_id:String,
    pub(super) request_id:String,
    pub(super) step_id:String,
    pub(super) wire:Vec<u8>,
    pub(super) source_key:RawSourceKey,
    pub(super) operation:NativeCardOperation,
    /// Only a newly created C intent can be passed to the sole H writer.
    pub(super) write_permitted:bool,
}

impl<'root> ProductDatabase<'root> {
    /// Read a completed original USER answer after H generation or E instance
    /// changes. This cannot create a card, settle UNKNOWN or acquire a writer.
    pub(super) fn visible_original_answer_receipt(&self,
        association:&super::v37_visible_conversation::Association,request:&V37Request,
        thread:&str)->Result<Option<Json>> {
        use super::v37_visible_conversation::{k,s,object,decimal};
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT o.request_hex,o.state,o.answer_kind,o.answer,o.native_receipt_id,q.vendor_request_id,q.turn_id,q.generation,q.seat_id,q.vendor_thread_id FROM main.gogoke_v37_qcard_native_operations o JOIN main.gogoke_v37_qcard_native q ON q.domain_id=o.domain_id AND q.card_id=o.card_id WHERE o.domain_id=?1 AND o.request_id=?2 AND o.card_id=?3")?;
        row.bind_text(1,&association.domain)?;row.bind_text(2,&request.request_id)?;row.bind_text(3,&request.target_id)?;
        if !row.step_row()? {return Ok(None);}
        if unhex(&row.column_text(0)?)?!=request.raw_bytes||row.column_text(2)?!="WIRE"
            ||row.column_text(7)?!=association.generation||row.column_text(8)?!=association.seat||row.column_text(9)?!=thread {
            return Err(OrchestrationError::AccessDenied);
        }
        let state=row.column_text(1)?;let answer=row.column_text(3)?;let receipt=row.column_text(4)?;
        let vendor=row.column_text(5)?;let turn=row.column_text(6)?;
        if row.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(row);
        if state!="ANSWERED" {return Ok(None);}
        let (source_key,digest)=read_source_descriptor(&self.connection,&association.domain,&request.target_id)?;
        let source=ledger::read_captured_raw_source(&self.connection,&source_key.operation_id,&source_key.source_epoch,&source_key.source_cursor)?
            .ok_or(OrchestrationError::OperationConflict)?;
        if source.domain_id!=association.domain||source.session_id!=association.session||source.generation!=association.generation
            ||sha256_hex(&source.raw_bytes)!=digest||card_identity(&association.domain,&association.session,&source.key).0!=request.target_id {
            return Err(OrchestrationError::AccessDenied);
        }
        let Reply::Question(question)=codex_rpc::decode(&source.raw_bytes,None).map_err(|error|store_error("original visible question",error))?
            else {return Err(OrchestrationError::AccessDenied);};
        if request_id_wire(&question.request_id)!=vendor||question.thread_id!=thread||question.turn_id!=turn {
            return Err(OrchestrationError::AccessDenied);
        }
        let mut answers=BTreeMap::new();
        let values=object(request.payload.get(&k("answers")).ok_or(OrchestrationError::OperationConflict)?)?;
        for (id,value) in values {
            let Json::Array(values)=value else {return Err(OrchestrationError::OperationConflict);};
            let values=values.iter().map(|value|match value {Json::String(value)=>value.to_well_formed_string()
                .ok_or(OrchestrationError::OperationConflict),_=>Err(OrchestrationError::OperationConflict)}).collect::<Result<Vec<_>>>()?;
            answers.insert(id.to_well_formed_string().ok_or(OrchestrationError::OperationConflict)?,values);
        }
        let command=question.answer(answers).map_err(|error|store_error("original visible answer",error))?;
        let wire=command.encode(None).map_err(|error|store_error("original visible answer wire",error))?;
        if std::str::from_utf8(wire.strip_suffix(b"\n").ok_or(OrchestrationError::OperationConflict)?)
            .map_err(|error|store_error("original visible answer UTF-8",error))?!=answer {return Err(OrchestrationError::AccessDenied);}
        let step=format!("qanswer{}",sha256_hex(format!("{}\n{}\n{}",association.domain,association.session,request.request_id).as_bytes()));
        let writer=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_rpc_steps s JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=s.domain_id AND e.session_id=s.session_id AND e.generation=s.generation AND e.process_operation_id=s.process_operation_id JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id AND g.session_id=e.session_id AND g.generation=e.generation AND g.process_operation_id=e.process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id AND c.generation=e.generation AND c.ticket=s.ticket AND c.custodian_nonce=s.custodian_nonce JOIN main.gogoke_v37_session_binding_v2 b ON b.domain_id=e.domain_id AND b.session_id=e.session_id AND b.seat_id=e.seat_id AND b.seat_incarnation=e.seat_incarnation AND b.selected_instance_id=e.instance_id WHERE s.domain_id=?1 AND s.session_id=?2 AND s.generation=?3 AND s.step_id=?4 AND s.process_operation_id=?5 AND s.ticket=?6 AND s.custodian_nonce=?7 AND s.command_hex=?8 AND s.phase='WRITTEN' AND s.requires_response=0 AND e.seat_id=?9 AND e.seat_incarnation=?10 AND e.instance_id=?11 AND b.seat_authorization_generation=?12")?;
        let encoded=super::v37_visible_conversation::encode_hex(&wire);
        for (index,value) in [association.domain.as_str(),association.session.as_str(),association.generation.as_str(),step.as_str(),
            source.key.operation_id.as_str(),source.process_ticket.as_str(),source.custodian_nonce.as_str(),encoded.as_str(),
            association.seat.as_str(),association.incarnation.as_str(),association.instance.as_str(),association.authorization.as_str()].iter().enumerate() {writer.bind_text((index+1) as i32,value)?;}
        if !writer.step_row()?||writer.step_row()? {return Err(OrchestrationError::AccessDenied);}
        decimal(&association.authorization)?;
        if receipt.is_empty() {return Err(OrchestrationError::OperationConflict);}
        Ok(Some(Json::Object(BTreeMap::from([(k("id"),Parser::parse(&vendor)?),(k("result"),Json::Object(BTreeMap::from([
            (k("state"),s("ANSWERED")),(k("nativeReceiptId"),s(&receipt)),
            (k("vendorConsumptionConfirmed"),Json::Bool(false)),
            (k("source"),Json::Object(BTreeMap::from([(k("operationId"),s(&source.key.operation_id)),
                (k("sourceEpoch"),s(&source.key.source_epoch)),(k("sourceCursor"),s(&source.key.source_cursor))]))),
        ])))]))))
    }
    // Claude's fixed stream has a real session_id but no vendor turn_id.
    // The card's turn_id is the original H User send request ID, kept under
    // the same physical custody; it is never presented as a vendor turn.
    fn observed_claude_card_binding(&mut self,key:&(String,String),live_input:bool)
        ->Result<CurrentCardBinding> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if run.evidence.driver_id()!="claude" || (live_input && !run.allows_input()) {
            return Err(OrchestrationError::AccessDenied);
        }
        let (send_bytes,identity)=run.pending_claude.as_ref()
            .ok_or(OrchestrationError::AccessDenied)?;
        let send=session_transport::decode_request(send_bytes).map_err(|error|
            store_error("Claude original H send",error))?;
        if send.family!="K-SESSION" || send.operation!="send" || send.domain_id!=key.0
            || send.target_id!=key.1 || identity.step_id.is_empty() {
            return Err(OrchestrationError::OperationConflict);
        }
        let seat=run.evidence.seat_id().to_owned();
        let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,&seat,&key.1).map_err(|error|store_error("H Claude claim",error))?
            .ok_or(OrchestrationError::AccessDenied)?;
        run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,claim.revision).map_err(OrchestrationError::V37StoreFailure)?;
        if self.process_custodian.active(&run.custody.ticket).is_none() {
            return Err(OrchestrationError::AccessDenied);
        }
        let binding=CurrentCardBinding {domain:key.0.clone(),session:key.1.clone(),seat,
            generation:run.custody.binding.generation.clone(),operation:run.operation_id.clone(),
            ticket:run.custody.ticket.opaque().to_owned(),nonce:run.custody.custodian_nonce.clone(),
            thread:run.thread_id.clone().ok_or(OrchestrationError::AccessDenied)?,
            turn:send.request_id};
        if !claude_send_present(&self.connection,&binding)? {
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(binding)
    }

    /// Retain the original Claude AskUserQuestion request in C. Ordinary
    /// can_use_tool permissions and request_user_dialog never enter this path.
    pub(super) fn raise_claude_card(&mut self,key:&(String,String),source_key:&RawSourceKey)
        ->Result<RaisedCodexCard> {
        let binding=self.observed_claude_card_binding(key,false)?;
        let source=ledger::read_pending_raw_source(&self.connection,&source_key.operation_id,
            &source_key.source_epoch,&source_key.source_cursor)?
            .ok_or(OrchestrationError::OperationConflict)?;
        if !source_matches(&source,&binding) {return Err(OrchestrationError::OperationConflict);}
        let card=claude_question::decode(&source.raw_bytes).map_err(|error|
            store_error("Claude original question",error))?
            .ok_or(OrchestrationError::OperationConflict)?;
        let first=card.questions().first().ok_or(OrchestrationError::OperationConflict)?;
        let payload=card.display_payload();
        let option_ids=(0..first.options.len()).map(|index|format!("option{index}"))
            .collect::<Vec<_>>();
        let options=first.options.iter().zip(&option_ids).map(|(choice,id)|NativeQuestionOption {
            id,label:&choice.label,description:&choice.description,
        }).collect::<Vec<_>>();
        let (card_id,raise_id)=card_identity(&binding.domain,&binding.session,&source.key);
        let descriptor=source_descriptor(&source);
        let envelope=CardEnvelope {domain_id:&binding.domain,card_id:&card_id,request_id:&raise_id,
            request_bytes:descriptor.as_bytes(),expected_revision:0};
        let host_id="host0";
        let vendor_id=Json::String(JsonString::from_str(card.request_id())).canonical();
        let question=NativeQuestion {vendor_request_id:&vendor_id,
            vendor_thread_id:&binding.thread,vendor_item_id:card.tool_use_id(),
            auto_resolution_ms:"",question_payload:&payload,question_id:host_id,
            header:&first.header,question:&first.question,
            answer_shape:NativeAnswerShape::OptionsOrFree,options:&options,
            seat_id:&binding.seat,turn_id:&binding.turn,generation:&binding.generation};
        let mut owner_error=None;
        let result=inbox::raise_native_card(&mut self.connection,&envelope,&question,|db| {
            if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                owner_error=Some(error);return Err(InboxError::Denied);
            }
            if !native_binding_present(db,&binding,true)? ||
                !claude_send_present(db,&binding).map_err(|error|
                    InboxError::InvalidEvidence(format!("Claude H send: {error:?}")))? {
                return Ok(false);
            }
            Ok(ledger::read_pending_raw_source(db,&source.key.operation_id,
                &source.key.source_epoch,&source.key.source_cursor)?.as_ref()==Some(&source))
        });
        if matches!(&result,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        let operation=result.map_err(|error|store_error("Claude raise",error))?;
        Ok(RaisedCodexCard {card_id,operation})
    }

    pub(super) fn query_claude_card(&mut self,key:&(String,String),card_id:&str)
        ->Result<Option<NativeQuestionCard>> {
        let binding=self.observed_claude_card_binding(key,true)?;
        let mut owner_error=None;
        let result=inbox::query_native_card(&mut self.connection,&binding.domain,card_id,|db| {
            if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                owner_error=Some(error);return Err(InboxError::Denied);
            }
            if !native_binding_present(db,&binding,false)? {return Ok(false);}
            claude_send_present(db,&binding).map_err(|error|
                InboxError::InvalidEvidence(format!("Claude original H send: {error:?}")))
        });
        if matches!(&result,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        let card=result.map_err(|error|store_error("Claude query",error))?;
        if let Some(ref card)=card {stored_claude_source(&self.connection,&binding,card)?;}
        Ok(card)
    }

    /// C's exact intent is committed before Root's sole H stdin writer. A
    /// replay returns its old operation and never grants a second write.
    pub(super) fn begin_claude_card_answer(&mut self,key:&(String,String),
        envelope:&CardEnvelope<'_>,answers:BTreeMap<String,Vec<String>>)
        ->Result<ClaudeAnswerWrite> {
        if envelope.domain_id!=key.0 {return Err(OrchestrationError::OperationConflict);}
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_qcard_native_operations
             WHERE domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,envelope.domain_id)?;prior.bind_text(2,envelope.request_id)?;
        let existing=if prior.step_row()? {
            let bytes=unhex(&prior.column_text(0)?)?;
            if prior.step_row()? || bytes!=envelope.request_bytes {
                return Err(OrchestrationError::OperationConflict);
            }
            true
        } else {false};
        drop(prior);
        let binding=if existing {None} else {Some(self.observed_claude_card_binding(key,true)?)};
        let card=if existing {
            self.query_codex_card_recovered(key,envelope.card_id)?
        } else {
            self.query_claude_card(key,envelope.card_id)?
        }.ok_or(OrchestrationError::OperationConflict)?;
        let (source_key,_)=read_source_descriptor(&self.connection,&key.0,&card.card_id)?;
        let source=ledger::read_captured_raw_source(&self.connection,&source_key.operation_id,
            &source_key.source_epoch,&source_key.source_cursor)?
            .ok_or(OrchestrationError::OperationConflict)?;
        let original=claude_question::decode(&source.raw_bytes).map_err(|error|
            store_error("Claude answer original",error))?
            .ok_or(OrchestrationError::OperationConflict)?;
        if card.question_payload!=original.display_payload()
            || card.vendor_request_id!=Json::String(JsonString::from_str(original.request_id())).canonical()
            || card.vendor_item_id!=original.tool_use_id() {
            return Err(OrchestrationError::OperationConflict);
        }
        let wire=claude_question::encode_host_answers(&original,&answers).map_err(|error|
            store_error("Claude answer encoding",error))?;
        let wire_text=std::str::from_utf8(wire.strip_suffix(b"\n")
            .ok_or(OrchestrationError::OperationConflict)?).map_err(|error|
                store_error("Claude answer UTF-8",error))?;
        let step_id=format!("qanswer{}",sha256_hex(format!("{}\n{}\n{}",key.0,
            key.1,envelope.request_id).as_bytes()));
        let mut owner_error=None;
        let intent=inbox::begin_native_answer_intent(&mut self.connection,envelope,
            &card.vendor_request_id,&card.seat_id,&card.turn_id,&card.generation,
            NativeAnswer::Wire(wire_text),|db| {
                if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                    owner_error=Some(error);return Err(InboxError::Denied);
                }
                if let Some(ref binding)=binding {
                    if !native_binding_present(db,binding,false)? {return Ok(false);}
                    let Some(current)=ledger::read_captured_raw_source(db,&source.key.operation_id,
                        &source.key.source_epoch,&source.key.source_cursor)? else {return Ok(false);};
                    Ok(current==source && claude_send_present(db,binding).map_err(|error|
                        InboxError::InvalidEvidence(format!("Claude H answer send: {error:?}")))?)
                } else {recovered_card_present(db,key,envelope.card_id)}
            });
        if matches!(&intent,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        let NativeAnswerIntent {operation,disposition}=intent.map_err(|error|
            store_error("Claude answer intent",error))?;
        if operation.answer_kind!="WIRE" || operation.answer!=wire_text {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(ClaudeAnswerWrite {card_id:card.card_id,request_id:envelope.request_id.into(),
            step_id,wire,source_key,operation,
            write_permitted:disposition==NativeAnswerDisposition::New})
    }

    /// C's fresh intent is the sole grant for one H exact write. Every replay
    /// reads the original outcome; neither UNKNOWN nor a pipe error retries.
    pub(super) fn answer_claude_card(&mut self,key:&(String,String),
        envelope:&CardEnvelope<'_>,answers:BTreeMap<String,Vec<String>>)
        ->Result<CodexAnswerWrite> {
        let plan=self.begin_claude_card_answer(key,envelope,answers)?;
        if !plan.write_permitted {
            let operation=if plan.operation.phase=="UNKNOWN" {
                match inbox::settle_native_answer_written(&mut self.connection,&self.owner,
                    envelope,&key.1,&plan.step_id) {
                    Ok(settled)=>settled,
                    Err(InboxError::Denied)=>plan.operation,
                    Err(error)=>return Err(store_error("Claude answer readback",error)),
                }
            } else {plan.operation};
            return Ok(CodexAnswerWrite {card_id:plan.card_id,
                request_id:plan.request_id,step_id:plan.step_id,
                operation,newly_written:false});
        }
        // This repeats live physical and owner checks after C's intent was
        // committed, immediately before asking H for an exact write permit.
        let binding=self.observed_claude_card_binding(key,true)?;
        if plan.source_key.operation_id!=binding.operation ||
            plan.source_key.source_epoch!=binding.nonce {
            return Err(OrchestrationError::OperationConflict);
        }
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let custody=run.custody.clone();
        let open_id=run.open_request_id.clone();
        let open_bytes=run.open_request_bytes.clone();
        let step=rpc::ClaudeQuestionStep {domain_id:&key.0,session_id:&key.1,
            open_request_id:&open_id,open_request_bytes:&open_bytes,
            step_id:&plan.step_id,custody:&custody,card_id:&plan.card_id,
            answer_request_id:&plan.request_id,source:&plan.source_key,wire:&plan.wire};
        let prepared=rpc::prepare_claude_question(&mut self.connection,&self.owner,&step)
            .map_err(|error|store_error("Claude H answer intent",error))?;
        if prepared.disposition!=rpc::Disposition::NewWrite {
            return Ok(CodexAnswerWrite {card_id:plan.card_id,
                request_id:plan.request_id,step_id:plan.step_id,
                operation:plan.operation,newly_written:false});
        }
        let process=self.process_custodian.active(&custody.ticket)
            .ok_or(OrchestrationError::AccessDenied)?;
        if let Err(error)=process.write_persistent_frame(&prepared.bytes) {
            let original=self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                crate::process::ProcessCustodyError::ProtocolPipe(error));
            let marked=rpc::mark_claude_question_unknown(&mut self.connection,&self.owner,
                &step,&original.to_string());
            return Err(OrchestrationError::NativeRecipientFailure(format!(
                "Claude question stdin write: {original}; UNKNOWN: {marked:?}")));
        }
        rpc::mark_claude_question_written(&mut self.connection,&self.owner,&step)
            .map_err(|error|store_error("Claude H answer written",error))?;
        let operation=inbox::settle_native_answer_written(&mut self.connection,&self.owner,
            envelope,&key.1,&plan.step_id)
            .map_err(|error|store_error("Claude exactwrite settlement",error))?;
        Ok(CodexAnswerWrite {card_id:plan.card_id,request_id:plan.request_id,
            step_id:plan.step_id,operation,newly_written:true})
    }

    /// Close only OPEN Claude cards for the original H User send whose
    /// committed receipt cites this exact provider result source. A result
    /// from another turn or session cannot expire the current question.
    pub(super) fn expire_claude_terminal_cards(&mut self,key:&(String,String),
        source_key:&RawSourceKey)->Result<()> {
        self.expire_claude_terminal_cards_for_turn(key,source_key,None)
    }

    fn expire_claude_terminal_cards_for_turn(&mut self,key:&(String,String),
        source_key:&RawSourceKey,expected_turn:Option<&str>)->Result<()> {
        use crate::store::session_transport::provider_evidence::stream_json::{self,ClaudeData};
        let source=ledger::read_captured_raw_source(&self.connection,
            &source_key.operation_id,&source_key.source_epoch,&source_key.source_cursor)?
            .ok_or(OrchestrationError::OperationConflict)?;
        let ClaudeData::Result {session_id:vendor_session,..}=
            stream_json::decode_claude_line(&source.raw_bytes).map_err(|error|
                store_error("Claude result source",error))? else {return Ok(());};
        if source.domain_id!=key.0 || source.session_id!=key.1 ||
            source.key!=*source_key || source.custodian_nonce!=source_key.source_epoch {
            return Err(OrchestrationError::OperationConflict);
        }
        let sends=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id,request_hex,generation,ticket,custodian_nonce
             FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND session_id=?2
               AND process_operation_id=?3 AND custodian_nonce=?4 AND operation='send'
               AND (?5='' OR request_id=?5)")?;
        for (index,value) in [key.0.as_str(),key.1.as_str(),source_key.operation_id.as_str(),
            source_key.source_epoch.as_str()].iter().enumerate() {
            sends.bind_text((index+1) as i32,value)?;
        }
        sends.bind_text(5,expected_turn.unwrap_or(""))?;
        let mut matched=None;
        while sends.step_row()? {
            let turn=sends.column_text(0)?;
            let bytes=unhex(&sends.column_text(1)?)?;
            let generation=sends.column_text(2)?;
            let ticket=sends.column_text(3)?;
            let nonce=sends.column_text(4)?;
            let input=session_transport::StdinRequest {domain_id:&key.0,
                session_id:&key.1,ticket:&ticket,generation:&generation,
                request_bytes:&bytes};
            let Some(completed)=session_transport::read_original_claude_send_completed(
                &self.connection,&input).map_err(|error|
                    store_error("Claude original result readback",error))? else {continue;};
            if completed.vendor_session_id!=vendor_session || nonce!=source_key.source_epoch
                || completed.terminal!=stream_json::decode_claude_line(&source.raw_bytes)
                    .map_err(|error|store_error("Claude terminal repeat",error))? {
                continue;
            }
            let receipt=completed.user.record.receipt_bytes.as_ref()
                .ok_or(OrchestrationError::OperationConflict)?;
            let fields=session_transport::decode_receipt(receipt).map_err(|error|
                store_error("Claude result receipt",error))?.into_result();
            let original_field=|name:&str|->Result<String> {
                let Some(Json::String(value))=fields.get(&JsonString::from_str(name)) else {
                    return Err(OrchestrationError::OperationConflict);
                };
                value.to_well_formed_string().ok_or(OrchestrationError::OperationConflict)
            };
            if original_field("sourceEpoch")?!=source_key.source_epoch ||
                original_field("sourceCursor")?!=source_key.source_cursor {
                continue;
            }
            if matched.replace((turn,generation,ticket,nonce)).is_some() {
                return Err(OrchestrationError::OperationConflict);
            }
        }
        drop(sends);
        let Some((turn,generation,ticket,nonce))=matched else {return Ok(());};
        if source.process_ticket!=ticket || source.generation!=generation {
            return Err(OrchestrationError::OperationConflict);
        }
        let cards=Statement::prepare(self.connection.as_ptr(),
            "SELECT card_id FROM main.gogoke_v37_qcard_native
             WHERE domain_id=?1 AND vendor_thread_id=?2 AND turn_id=?3
               AND generation=?4 AND state='OPEN'")?;
        for (index,value) in [key.0.as_str(),vendor_session.as_str(),turn.as_str(),
            generation.as_str()].iter().enumerate() {
            cards.bind_text((index+1) as i32,value)?;
        }
        let mut open=Vec::new();
        while cards.step_row()? {open.push(cards.column_text(0)?);}
        drop(cards);
        let descriptor=source_descriptor(&source);
        let result_cursor=source_key.source_cursor.parse::<u64>().map_err(|error|
            store_error("Claude result cursor",error))?;
        for card_id in open {
            let card=inbox::query_native_card(&mut self.connection,&key.0,&card_id,|db| {
                authority::check_owner_in_current_transaction(db,&self.owner)
                    .map_err(InboxError::Authority)?;
                Ok(true)
            }).map_err(|error|store_error("Claude terminal C card",error))?
                .ok_or(OrchestrationError::OperationConflict)?;
            if card.state!="OPEN" {return Err(OrchestrationError::OperationConflict);}
            let binding=CurrentCardBinding {domain:key.0.clone(),session:key.1.clone(),
                seat:card.seat_id.clone(),generation:generation.clone(),
                operation:source_key.operation_id.clone(),ticket:ticket.clone(),
                nonce:nonce.clone(),thread:vendor_session.clone(),turn:turn.clone()};
            let question_source=stored_claude_source(&self.connection,&binding,&card)?;
            let question_cursor=question_source.key.source_cursor.parse::<u64>().map_err(|error|
                store_error("Claude question cursor",error))?;
            if question_cursor>=result_cursor ||
                question_source.process_ticket!=source.process_ticket ||
                question_source.custodian_nonce!=source.custodian_nonce {
                return Err(OrchestrationError::OperationConflict);
            }
            let expiry=format!("expire{}",sha256_hex(format!("{}\n{}",card_id,descriptor).as_bytes()));
            let envelope=CardEnvelope {domain_id:&key.0,card_id:&card_id,
                request_id:&expiry,request_bytes:descriptor.as_bytes(),expected_revision:card.revision};
            inbox::expire_native_card(&mut self.connection,&envelope,&card.vendor_request_id,&card.seat_id,
                &turn,&generation,|db| {
                    authority::check_owner_in_current_transaction(db,&self.owner)
                        .map_err(InboxError::Authority)?;
                    let current=ledger::read_captured_raw_source(db,&source.key.operation_id,
                        &source.key.source_epoch,&source.key.source_cursor)?;
                    let question=ledger::read_captured_raw_source(db,&question_source.key.operation_id,
                        &question_source.key.source_epoch,&question_source.key.source_cursor)?;
                    Ok(current.as_ref()==Some(&source) &&
                        question.as_ref()==Some(&question_source) && nonce==source.custodian_nonce)
                }).map_err(|error|store_error("Claude terminal card expiry",error))?;
        }
        Ok(())
    }

    /// At the existing authority-thread safe point, close only historical C
    /// OPEN Claude cards whose exact original H User send already has a
    /// committed APPLIED or FAILED Result receipt. This works after restart
    /// without an in-memory NativeSession and performs no provider I/O.
    pub(super) fn reconcile_durable_claude_open_cards(&mut self)->Result<()> {
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT c.domain_id,c.card_id,c.question_payload,h.session_id,h.request_id,h.request_hex,
                    h.generation,h.ticket,h.custodian_nonce,h.process_operation_id,
                    h.receipt_hex,h.receipt_status
               FROM main.gogoke_v37_qcard_native c
               JOIN main.gogoke_v37_h_stdin_journal h
                 ON h.domain_id=c.domain_id AND h.request_id=c.turn_id
                AND h.generation=c.generation AND h.operation='send'
              WHERE c.state='OPEN' AND instr(c.question_payload,'\"provider\":\"claude\"')>0
                AND h.phase='RECEIPTED'
                AND h.receipt_status IN ('APPLIED','FAILED')
              ORDER BY c.domain_id,c.card_id LIMIT 32")?;
        let mut candidates=Vec::new();
        while query.step_row()? {
            let payload=query.column_text(2)?;
            let Json::Object(fields)=Parser::parse(&payload)? else {
                return Err(OrchestrationError::OperationConflict);
            };
            if !matches!(fields.get(&JsonString::from_str("provider")),
                Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some("claude")) {
                continue;
            }
            candidates.push((query.column_text(0)?,query.column_text(1)?,
                query.column_text(3)?,query.column_text(4)?,query.column_text(5)?,
                query.column_text(6)?,query.column_text(7)?,query.column_text(8)?,
                query.column_text(9)?,query.column_text(10)?,query.column_text(11)?));
        }
        drop(query);
        for (domain,card_id,session,turn_id,request_hex,generation,ticket,nonce,operation,
            receipt_hex,receipt_status) in candidates {
            let request_bytes=unhex(&request_hex)?;
            let input=session_transport::StdinRequest {domain_id:&domain,
                session_id:&session,ticket:&ticket,generation:&generation,
                request_bytes:&request_bytes};
            let completed=session_transport::read_original_claude_send_completed(
                &self.connection,&input).map_err(|error|
                    store_error("durable Claude H result",error))?
                .ok_or(OrchestrationError::OperationConflict)?;
            let original=completed.user.record.receipt_bytes.as_ref()
                .ok_or(OrchestrationError::OperationConflict)?;
            if original!=&unhex(&receipt_hex)? ||
                completed.user.record.process_operation_id!=operation ||
                completed.user.record.custodian_nonce!=nonce ||
                completed.user.record.ticket!=ticket {
                return Err(OrchestrationError::OperationConflict);
            }
            let receipt=session_transport::decode_receipt(original).map_err(|error|
                store_error("durable Claude receipt",error))?;
            if receipt.status.wire()!=receipt_status ||
                !matches!(receipt.status,V37Status::Applied|V37Status::Failed) {
                return Err(OrchestrationError::OperationConflict);
            }
            let result=receipt.into_result();
            let field=|name:&str|->Result<String> {
                let Some(Json::String(value))=result.get(&JsonString::from_str(name)) else {
                    return Err(OrchestrationError::OperationConflict);
                };
                value.to_well_formed_string().ok_or(OrchestrationError::OperationConflict)
            };
            let source=RawSourceKey {operation_id:operation.clone(),
                source_epoch:field("sourceEpoch")?,source_cursor:field("sourceCursor")?};
            let crate::store::session_transport::provider_evidence::stream_json::ClaudeData::Result {
                session_id:result_session,subtype:result_subtype,..}= &completed.terminal else {
                return Err(OrchestrationError::OperationConflict);
            };
            if source.source_epoch!=nonce ||
                field("deliveryBasis")?!="CLAUDE_USER_REPLAY_AND_RESULT" ||
                field("vendorSessionId")?!=completed.vendor_session_id ||
                result_session!=&completed.vendor_session_id ||
                field("resultSubtype")?!=result_subtype.as_str() {
                return Err(OrchestrationError::OperationConflict);
            }
            let raw=ledger::read_captured_raw_source(&self.connection,&source.operation_id,
                &source.source_epoch,&source.source_cursor)?
                .ok_or(OrchestrationError::OperationConflict)?;
            if raw.domain_id!=domain || raw.session_id!=session ||
                raw.generation!=generation || raw.process_ticket!=ticket ||
                raw.custodian_nonce!=nonce ||
                raw.state!=ledger::RawSourceState::NoEvent ||
                raw.no_event_reason.as_deref()!=Some("CLAUDE_RESULT_RESPONSE") ||
                sha256_hex(&raw.raw_bytes)!=field("rawResultSha256")? {
                return Err(OrchestrationError::OperationConflict);
            }
            // The original fixed Claude reader rechecks the native pin,
            // echo UUID/text/session, first subsequent Result and receipt.
            // C independently checks its earlier question source below.
            self.expire_claude_terminal_cards_for_turn(&(domain,session),&source,
                Some(&turn_id))?;
            let still_open=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_qcard_native WHERE domain_id=?1
                  AND card_id=?2 AND state='OPEN'")?;
            still_open.bind_text(1,&raw.domain_id)?;still_open.bind_text(2,&card_id)?;
            if still_open.step_row()? {
                return Err(OrchestrationError::OperationConflict);
            }
        }
        Ok(())
    }

    /// The fixed stream's original pure tool_result block is a narrower
    /// question-resolution fact than a whole-turn result. It closes only the
    /// matching source/tool-use ID while the card remains OPEN.
    pub(super) fn expire_claude_resolved_cards(&mut self,key:&(String,String),
        source_key:&RawSourceKey)->Result<()> {
        use crate::store::session_transport::provider_evidence::stream_json;
        let source=ledger::read_captured_raw_source(&self.connection,
            &source_key.operation_id,&source_key.source_epoch,&source_key.source_cursor)?
            .ok_or(OrchestrationError::OperationConflict)?;
        if !stream_json::is_claude_tool_result_line(&source.raw_bytes) {return Ok(());}
        if source.domain_id!=key.0 || source.session_id!=key.1 || source.key!=*source_key
            || source.custodian_nonce!=source_key.source_epoch {
            return Err(OrchestrationError::OperationConflict);
        }
        let cards=Statement::prepare(self.connection.as_ptr(),
            "SELECT card_id,question_payload FROM main.gogoke_v37_qcard_native
             WHERE domain_id=?1 AND generation=?2 AND state='OPEN'")?;
        cards.bind_text(1,&key.0)?;cards.bind_text(2,&source.generation)?;
        let mut ids=Vec::new();
        while cards.step_row()? {
            let payload=cards.column_text(1)?;
            if matches!(Parser::parse(&payload),Ok(Json::Object(ref fields))
                if matches!(fields.get(&JsonString::from_str("provider")),
                    Some(Json::String(provider)) if provider.to_well_formed_string().as_deref()==Some("claude"))) {
                ids.push(cards.column_text(0)?);
            }
        }
        drop(cards);
        let descriptor=source_descriptor(&source);
        let current_cursor=source_key.source_cursor.parse::<u64>()
            .map_err(|_|OrchestrationError::OperationConflict)?;
        for card_id in ids {
            let Some(card)=self.query_codex_card_recovered(key,&card_id)? else {continue;};
            if card.state!="OPEN" || card.question_payload.is_empty() {continue;}
            let (question_key,_)=read_source_descriptor(&self.connection,&key.0,&card_id)?;
            if question_key.operation_id!=source_key.operation_id ||
                question_key.source_epoch!=source_key.source_epoch ||
                question_key.source_cursor.parse::<u64>().ok().map_or(true,|cursor|
                    cursor>=current_cursor) {
                continue;
            }
            if !claude_question::tool_result_resolves(&source.raw_bytes,
                &card.vendor_item_id,&card.vendor_thread_id).map_err(|error|
                    store_error("Claude tool result",error))? {
                continue;
            }
            let expiry=format!("expire{}",sha256_hex(format!("{}\n{}",card_id,descriptor).as_bytes()));
            let envelope=CardEnvelope {domain_id:&key.0,card_id:&card_id,
                request_id:&expiry,request_bytes:descriptor.as_bytes(),expected_revision:card.revision};
            inbox::expire_native_card(&mut self.connection,&envelope,&card.vendor_request_id,
                &card.seat_id,&card.turn_id,&card.generation,|db| {
                    authority::check_owner_in_current_transaction(db,&self.owner)
                        .map_err(InboxError::Authority)?;
                    let current=ledger::read_captured_raw_source(db,&source.key.operation_id,
                        &source.key.source_epoch,&source.key.source_cursor)?;
                    Ok(current.as_ref()==Some(&source) &&
                        recovered_card_present(db,key,&card_id)?)
                }).map_err(|error|store_error("Claude tool result card expiry",error))?;
        }
        Ok(())
    }
    fn current_card_binding(&mut self,key:&(String,String))->Result<CurrentCardBinding> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if !run.allows_input() {return Err(OrchestrationError::AccessDenied);}
        self.observed_card_binding(key)
    }

    // Consuming already-captured provider facts is not a new input grant.
    // EOF/stop fencing may forbid an answer while A's tail still needs C.
    fn observed_card_binding(&mut self,key:&(String,String))->Result<CurrentCardBinding> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let seat=run.evidence.seat_id().to_owned();
        let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,&seat,&key.1).map_err(|error|store_error("H claim",error))?
            .ok_or(OrchestrationError::AccessDenied)?;
        run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,claim.revision).map_err(OrchestrationError::V37StoreFailure)?;
        if self.process_custodian.active(&run.custody.ticket).is_none() {
            return Err(OrchestrationError::AccessDenied);
        }
        Ok(CurrentCardBinding {domain:key.0.clone(),session:key.1.clone(),seat,
            generation:run.custody.binding.generation.clone(),operation:run.operation_id.clone(),
            ticket:run.custody.ticket.opaque().to_owned(),nonce:run.custody.custodian_nonce.clone(),
            thread:run.thread_id.clone().ok_or(OrchestrationError::AccessDenied)?,
            turn:run.turn_id.clone().ok_or(OrchestrationError::AccessDenied)?})
    }

    /// A captured exact server request is raised once under the current
    /// Owner/E/F/H binding. Replaying its same raw source returns C's row.
    pub(super) fn raise_codex_card(&mut self,key:&(String,String),source_key:&RawSourceKey)
        ->Result<RaisedCodexCard> {
        let binding=self.observed_card_binding(key)?;
        let source=ledger::read_pending_raw_source(&self.connection,&source_key.operation_id,
            &source_key.source_epoch,&source_key.source_cursor)?
            .ok_or(OrchestrationError::OperationConflict)?;
        let (card,payload)=decoded_question(&source,&binding)?;
        let first=card.questions.first().ok_or(OrchestrationError::OperationConflict)?;
        let option_ids:Vec<String>=first.options.as_ref().map(|options|
            (0..options.len()).map(|index|format!("option{index}")).collect()).unwrap_or_default();
        let options:Vec<NativeQuestionOption<'_>>=first.options.as_ref().map(|choices|choices.iter()
            .zip(&option_ids).map(|((label,description),id)|NativeQuestionOption {
                id,label,description}).collect()).unwrap_or_default();
        let shape=match (options.is_empty(),first.is_other) {
            (true,_)=>NativeAnswerShape::FreeText,
            (false,true)=>NativeAnswerShape::OptionsOrFree,
            (false,false)=>NativeAnswerShape::Options,
        };
        let vendor_id=request_id_wire(&card.request_id);
        let auto_ms=card.auto_resolution_ms.map(|value|value.to_string()).unwrap_or_default();
        let (card_id,raise_id)=card_identity(&binding.domain,&binding.session,&source.key);
        let descriptor=source_descriptor(&source);
        let envelope=CardEnvelope {domain_id:&binding.domain,card_id:&card_id,request_id:&raise_id,
            request_bytes:descriptor.as_bytes(),expected_revision:0};
        let question=NativeQuestion {vendor_request_id:&vendor_id,vendor_thread_id:&card.thread_id,
            vendor_item_id:&card.item_id,auto_resolution_ms:&auto_ms,question_payload:&payload,
            question_id:&first.id,header:&first.header,question:&first.question,
            answer_shape:shape,options:&options,seat_id:&binding.seat,
            turn_id:&card.turn_id,generation:&binding.generation};
        let mut owner_error=None;
        let result=inbox::raise_native_card(&mut self.connection,&envelope,&question,|db| {
            if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                owner_error=Some(error);return Err(InboxError::Denied);
            }
            // A's original captured source is a historical provider fact;
            // UNKNOWN current custody grants no new answer or RPC write.
            if !native_binding_present(db,&binding,true)? {return Ok(false);}
            let Some(current)=ledger::read_pending_raw_source(db,&source.key.operation_id,
                &source.key.source_epoch,&source.key.source_cursor)? else {return Ok(false);};
            Ok(current==source)
        });
        if matches!(&result,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        let operation=result.map_err(|error|store_error("raise",error))?;
        Ok(RaisedCodexCard {card_id,operation})
    }

    /// Read C's exact native card only while its original A source and the
    /// current H/E/F binding still agree. A missing card is an ordinary None.
    pub(super) fn query_codex_card(&mut self,key:&(String,String),card_id:&str)
        ->Result<Option<NativeQuestionCard>> {
        let binding=self.current_card_binding(key)?;
        let mut owner_error=None;
        let result=inbox::query_native_card(&mut self.connection,&binding.domain,card_id,|db| {
            if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                owner_error=Some(error);return Err(InboxError::Denied);
            }
            native_binding_present(db,&binding,false)
        });
        if matches!(&result,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        let card=result.map_err(|error|store_error("query",error))?;
        if let Some(ref card)=card {
            if card.seat_id!=binding.seat || card.generation!=binding.generation
                || card.vendor_thread_id!=binding.thread || card.turn_id!=binding.turn {
                return Err(OrchestrationError::OperationConflict);
            }
            stored_source(&self.connection,&binding,card)?;
        }
        Ok(card)
    }

    /// Read a previously captured native card after its turn or process has
    /// ended. This never consults a live session map and never permits a new
    /// answer write; C, A, H, E, and current Owner facts are rechecked on one
    /// verified product connection. A changed original binding is denied.
    pub(super) fn query_codex_card_recovered(&mut self,key:&(String,String),card_id:&str)
        ->Result<Option<NativeQuestionCard>> {
        let mut owner_error=None;
        let result=inbox::query_native_card(&mut self.connection,&key.0,card_id,|db| {
            if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                owner_error=Some(error);return Err(InboxError::Denied);
            }
            recovered_card_present(db,key,card_id)
        });
        if matches!(&result,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        result.map_err(|error|store_error("recovered query",error))
    }

    /// The C intent transaction grants at most one physical write. Existing
    /// UNKNOWN is readback only; a completed persistent H write still needs
    /// the separate typed native write proof to settle C's card state.
    pub(super) fn answer_codex_card(&mut self,key:&(String,String),
        envelope:&CardEnvelope<'_>,answers:BTreeMap<String,Vec<String>>)
        ->Result<CodexAnswerWrite> {
        // A persisted C request can be read back after the turn and native
        // process ended. Its original exact bytes are checked before any
        // readback, and this branch can never acquire a new write permit.
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_qcard_native_operations
             WHERE domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,envelope.domain_id)?;prior.bind_text(2,envelope.request_id)?;
        let existing=if prior.step_row()? {
            let bytes=unhex(&prior.column_text(0)?)?;
            if prior.step_row()? || bytes!=envelope.request_bytes {
                return Err(OrchestrationError::OperationConflict);
            }
            true
        } else {false};
        drop(prior);
        if existing {
            if envelope.domain_id!=key.0 {return Err(OrchestrationError::OperationConflict);}
            let card=self.query_codex_card_recovered(key,envelope.card_id)?
                .ok_or(OrchestrationError::OperationConflict)?;
            let command=answer_command(&card,answers)?;
            let wire=command.encode(None).map_err(|error|store_error("answer replay wire",error))?;
            let wire_text=std::str::from_utf8(wire.strip_suffix(b"\n")
                .ok_or(OrchestrationError::OperationConflict)?).map_err(|error|
                    store_error("answer replay UTF-8",error))?;
            let step_id=format!("qanswer{}",sha256_hex(format!("{}\n{}\n{}",key.0,
                key.1,envelope.request_id).as_bytes()));
            let mut owner_error=None;
            let result=inbox::begin_native_answer_intent(&mut self.connection,envelope,
                &card.vendor_request_id,&card.seat_id,&card.turn_id,&card.generation,
                NativeAnswer::Wire(wire_text),|db| {
                    if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                        owner_error=Some(error);return Err(InboxError::Denied);
                    }
                    recovered_card_present(db,key,envelope.card_id)
                });
            if matches!(&result,Err(InboxError::Denied)) {
                if let Some(error)=owner_error {return Err(error);}
            }
            let NativeAnswerIntent {operation,disposition}=result
                .map_err(|error|store_error("answer readback",error))?;
            if disposition!=NativeAnswerDisposition::Existing || operation.answer_kind!="WIRE"
                || operation.answer!=wire_text {
                return Err(OrchestrationError::OperationConflict);
            }
            let operation=if operation.phase=="UNKNOWN" {
                // The typed factory only settles a durable exact H WRITTEN
                // step with the original C/A/H binding. If no such step
                // exists, UNKNOWN remains honest and no retry is attempted.
                match inbox::settle_native_answer_written(&mut self.connection,&self.owner,
                    envelope,&key.1,&step_id) {
                    Ok(settled)=>settled,
                    Err(InboxError::Denied)=>operation,
                    Err(error)=>return Err(store_error("answer readback proof",error)),
                }
            } else {operation};
            return Ok(CodexAnswerWrite {card_id:card.card_id,
                request_id:envelope.request_id.into(),step_id,operation,newly_written:false});
        }
        let binding=self.current_card_binding(key)?;
        if envelope.domain_id!=binding.domain {return Err(OrchestrationError::OperationConflict);}
        let card=self.query_codex_card(key,envelope.card_id)?
            .ok_or(OrchestrationError::OperationConflict)?;
        let source=stored_source(&self.connection,&binding,&card)?;
        let command=answer_command(&card,answers)?;
        let wire=command.encode(None).map_err(|error|store_error("answer wire",error))?;
        let wire_text=std::str::from_utf8(wire.strip_suffix(b"\n")
            .ok_or(OrchestrationError::OperationConflict)?).map_err(|error|
                store_error("answer wire UTF-8",error))?;
        let step_id=format!("qanswer{}",sha256_hex(format!("{}\n{}\n{}",binding.domain,
            binding.session,envelope.request_id).as_bytes()));
        let mut owner_error=None;
        let intent=inbox::begin_native_answer_intent(&mut self.connection,envelope,
            &card.vendor_request_id,&binding.seat,&binding.turn,&binding.generation,
            NativeAnswer::Wire(wire_text),|db| {
                if let Err(error)=authority::check_owner_in_current_transaction(db,&self.owner) {
                    owner_error=Some(error);return Err(InboxError::Denied);
                }
                if !native_binding_present(db,&binding,false)? {return Ok(false);}
                let Some(current)=ledger::read_captured_raw_source(db,&source.key.operation_id,
                    &source.key.source_epoch,&source.key.source_cursor)? else {return Ok(false);};
                Ok(current==source)
            });
        if matches!(&intent,Err(InboxError::Denied)) {
            if let Some(error)=owner_error {return Err(error);}
        }
        let NativeAnswerIntent {operation,disposition}=intent.map_err(|error|store_error("answer intent",error))?;
        if disposition==NativeAnswerDisposition::Existing {
            if operation.phase=="UNKNOWN" {
                let q=Statement::prepare(self.connection.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2
                     AND step_id=?3 AND process_operation_id=?4 AND ticket=?5
                     AND custodian_nonce=?6 AND phase='WRITTEN' AND requires_response=0")?;
                for (index,value) in [binding.domain.as_str(),binding.session.as_str(),step_id.as_str(),
                    binding.operation.as_str(),binding.ticket.as_str(),binding.nonce.as_str()].iter().enumerate() {
                    q.bind_text((index+1) as i32,value)?;
                }
                let written=q.step_row()?;
                if written && q.step_row()? {return Err(OrchestrationError::OperationConflict);}
                drop(q);
                if written {
                    let settled=inbox::settle_native_answer_written(&mut self.connection,
                        &self.owner,envelope,&binding.session,&step_id)
                        .map_err(|error|store_error("answer recovery proof",error))?;
                    return Ok(CodexAnswerWrite {card_id:card.card_id,
                        request_id:envelope.request_id.into(),step_id,operation:settled,newly_written:false});
                }
            }
            return Ok(CodexAnswerWrite {card_id:card.card_id,request_id:envelope.request_id.into(),
                step_id,operation,newly_written:false});
        }
        let observed=self.native_rpc(key,&step_id,None,&command)?;
        if observed.is_some() {return Err(OrchestrationError::OperationConflict);}
        let operation=inbox::settle_native_answer_written(&mut self.connection,&self.owner,
            envelope,&binding.session,&step_id).map_err(|error|store_error("answer written proof",error))?;
        Ok(CodexAnswerWrite {card_id:card.card_id,request_id:envelope.request_id.into(),
            step_id,operation,newly_written:true})
    }
}
