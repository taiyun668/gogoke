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
         JOIN main.gogoke_v37_h_seat_binding sb ON sb.domain_id=h.domain_id
           AND sb.session_id=h.session_id AND sb.generation=h.generation
         JOIN main.gogoke_v37_seats e ON e.domain_id=sb.domain_id AND e.seat_id=sb.seat_id
           AND e.incarnation=sb.seat_incarnation AND CAST(e.generation AS TEXT)=sb.generation
           AND e.instance_id=h.instance_id AND e.state='BUSY'
         WHERE h.domain_id=?1 AND h.session_id=?2 AND sb.seat_id=?3 AND h.generation=?4
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

impl<'root> ProductDatabase<'root> {
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
