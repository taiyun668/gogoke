//! Host health is an original failed WORK turn, not a model tool authority.
//! The caller holds the existing same-connection transaction and live native
//! process witness. This module reads H/A/F/E facts; it writes and sends nothing.

use super::{codex_output, codex_rpc, decode_receipt, decode_request, generation_change,
    rpc_journal, V37Status};
use crate::process::PreparedCustody;
use crate::store::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::digest::sha256_hex;
use crate::store::ledger::{self, RawSourceKey, RawSourceRecord, RawSourceState, SessionPurpose};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, HealthSignal, Seat, State};
use std::collections::BTreeMap;

#[derive(Debug)]
pub(crate) enum HostHealthError {
    Denied,
    Conflict,
    Utf8 { field:&'static str, cause:std::str::Utf8Error },
    Integer { field:&'static str, cause:std::num::ParseIntError },
    Wire { field:&'static str, cause:super::V37WireError },
    Store(AtomicError),
    Authority(OrchestrationError),
    Rpc(rpc_journal::RpcJournalError),
    Codec(codex_rpc::RpcError),
    Output(codex_output::OutputError),
    Seat(seat::SeatError),
}
impl From<AtomicError> for HostHealthError { fn from(e:AtomicError)->Self {Self::Store(e)} }
impl From<OrchestrationError> for HostHealthError { fn from(e:OrchestrationError)->Self {Self::Authority(e)} }
impl From<rpc_journal::RpcJournalError> for HostHealthError {
    fn from(e:rpc_journal::RpcJournalError)->Self {Self::Rpc(e)}
}
impl From<codex_rpc::RpcError> for HostHealthError { fn from(e:codex_rpc::RpcError)->Self {Self::Codec(e)} }
impl From<codex_output::OutputError> for HostHealthError { fn from(e:codex_output::OutputError)->Self {Self::Output(e)} }
impl From<seat::SeatError> for HostHealthError { fn from(e:seat::SeatError)->Self {Self::Seat(e)} }
type Result<T> = std::result::Result<T, HostHealthError>;
type Fields = BTreeMap<JsonString, Json>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cause { ContextCompact, RepeatedFailure }
impl Cause {
    fn signal(self)->HealthSignal {match self {
        Self::ContextCompact=>HealthSignal::ContextCompact,
        Self::RepeatedFailure=>HealthSignal::RepeatedFailure,
    }}
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkTurn {
    request_id:String,
    request_sha256:String,
    receipt_sha256:String,
    command_sha256:String,
    ack_sha256:String,
    ack_cursor:u64,
    revision:u64,
}

/// Only observe_codex_host_health constructs this seal. There is no wire
/// constructor, model-call conversion, or caller/grant field in the proof.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostHealthProof {
    custody:PreparedCustody,
    source:RawSourceKey,
    source_event_id:String,
    raw:Vec<u8>,
    raw_sha256:String,
    domain:String,
    session:String,
    operation:String,
    open_request_id:String,
    seat:String,
    incarnation:String,
    instance:String,
    generation:i64,
    thread:String,
    turn:String,
    cause:Cause,
    work:WorkTurn,
}
impl HostHealthProof {
    pub(crate) fn domain_id(&self)->&str {&self.domain}
    pub(crate) fn session_id(&self)->&str {&self.session}
    pub(crate) fn seat_id(&self)->&str {&self.seat}
    pub(crate) fn incarnation(&self)->&str {&self.incarnation}
    pub(crate) fn instance_id(&self)->&str {&self.instance}
    pub(crate) fn generation(&self)->i64 {self.generation}
    pub(crate) fn process_operation_id(&self)->&str {&self.operation}
    pub(crate) fn thread_id(&self)->&str {&self.thread}
    pub(crate) fn turn_id(&self)->&str {&self.turn}
    pub(crate) fn source_event_id(&self)->&str {&self.source_event_id}
    pub(crate) fn source(&self)->&RawSourceKey {&self.source}
    pub(crate) fn raw_sha256(&self)->&str {&self.raw_sha256}
    pub(crate) fn raw_source_bytes(&self)->&[u8] {&self.raw}
    pub(crate) fn custody(&self)->&PreparedCustody {&self.custody}
    pub(crate) fn signal(&self)->HealthSignal {self.cause.signal()}
    pub(crate) fn work_send_request_id(&self)->&str {&self.work.request_id}
}

fn key(value:&str)->JsonString {JsonString::from_str(value)}
fn object(raw:&[u8])->Result<Fields> {
    let text=std::str::from_utf8(raw).map_err(|cause|HostHealthError::Utf8 {
        field:"JSON object",cause})?;
    let Json::Object(fields)=Parser::parse(text.trim_end_matches('\n'))? else {
        return Err(HostHealthError::Denied);
    };
    Ok(fields)
}
fn fields<'a>(object:&'a Fields,name:&str)->Option<&'a Fields> {
    match object.get(&key(name)) {Some(Json::Object(value))=>Some(value),_=>None}
}
fn string(object:&Fields,name:&str)->Option<String> {
    match object.get(&key(name)) {
        Some(Json::String(value))=>value.to_well_formed_string(),_=>None,
    }
}
fn unhex(value:&str)->Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(HostHealthError::Denied)}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|cause|HostHealthError::Utf8 {
            field:"hex pair",cause})?;
        u8::from_str_radix(text,16).map_err(|cause|HostHealthError::Integer {
            field:"hex byte",cause})
    }).collect()
}
/// Fixed 0.160.0 typed error only. Neither message text nor an ordinary error
/// notification supplies a cause; CLI retry exhaustion is already a fact.
fn typed_terminal(raw:&[u8])->Result<Option<(String,String,Cause)>> {
    // Use the existing bounded LF/depth/RPC decoder before parsing raw JSON.
    let codex_rpc::Reply::TurnNotification {thread_id,turn_id,status:codex_rpc::TurnStatus::Failed,..}=
        codex_rpc::decode(raw,None)? else {return Ok(None)};
    let root=object(raw)?;
    if root.contains_key(&key("id")) || string(&root,"method").as_deref()!=Some("turn/completed") {
        return Ok(None);
    }
    let Some(params)=fields(&root,"params") else {return Ok(None)};
    if params.len()!=2 {return Ok(None)} // willRetry belongs to error, never a terminal cause.
    let Some(turn)=fields(params,"turn") else {return Ok(None)};
    if string(turn,"status").as_deref()!=Some("failed") {return Ok(None)}
    let Some(error)=fields(turn,"error") else {return Ok(None)};
    let cause=match error.get(&key("codexErrorInfo")) {
        Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some("contextWindowExceeded")=>Cause::ContextCompact,
        Some(Json::Object(info)) if info.len()==1=>{
            let Some(details)=fields(info,"responseTooManyFailedAttempts") else {return Ok(None)};
            if details.len()!=1 {return Ok(None)}
            match details.get(&key("httpStatusCode")) {
                Some(Json::Null)=>{},
                Some(Json::Number(value)) if value.parse::<u16>().is_ok()=>{},
                _=>return Ok(None),
            }
            Cause::RepeatedFailure
        },
        _=>return Ok(None),
    };
    let codex_output::Output::TurnTerminal {status:codex_rpc::TurnStatus::Failed,..}=
        codex_output::normalize(raw,&thread_id)? else {return Ok(None)};
    Ok(Some((thread_id,turn_id,cause)))
}

fn resolved_event(db:&VerifiedDatabaseConnection<'_>,source:&RawSourceRecord,
    seat:&str,thread:&str,turn:&str)->Result<String> {
    if source.state!=RawSourceState::Resolved || source.no_event_reason.is_some() {
        return Err(HostHealthError::Denied);
    }
    let event=source.resolved_event_id.as_deref().filter(|id|!id.is_empty())
        .ok_or(HostHealthError::Denied)?;
    let q=Statement::prepare(db.as_ptr(),
        "SELECT update_json FROM main.v37_ledger_index WHERE source_kind='v37'
          AND source_event_id=?1 AND domain_id=?2 AND seat_id=?3 AND session_id=?4
          AND source_epoch=?5 AND tier='SESSION' AND side_id IS NULL")?;
    for (index,value) in [event,source.domain_id.as_str(),seat,source.session_id.as_str(),
        source.key.source_epoch.as_str()].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    if !q.step_row()? {return Err(HostHealthError::Denied)}
    let update=object(q.column_text(0)?.as_bytes())?;
    if q.step_row()? {return Err(HostHealthError::Conflict)}
    let meta=fields(&update,"_meta").ok_or(HostHealthError::Denied)?;
    if string(meta,"provider").as_deref()!=Some("codex")
        || string(meta,"codexMethod").as_deref()!=Some("turn/completed")
        || string(meta,"threadId").as_deref()!=Some(thread)
        || string(meta,"turnId").as_deref()!=Some(turn)
        || string(meta,"turnStatus").as_deref()!=Some("failed")
        || string(meta,"rawSourceCursor").as_deref()!=Some(source.key.source_cursor.as_str()) {
        return Err(HostHealthError::Denied);
    }
    let raw=object(&source.raw_bytes)?;
    let original_error=fields(&raw,"params").and_then(|params|fields(params,"turn"))
        .and_then(|turn|turn.get(&key("error"))).ok_or(HostHealthError::Denied)?;
    if meta.get(&key("codexError")).map(Json::canonical)!=Some(original_error.canonical()) {
        return Err(HostHealthError::Denied);
    }
    Ok(event.to_owned())
}

/// Same ordinary H send/createdTurn/RPC relation as model_call, including a
/// normally admitted child's original dispatch send. A control/compact turn
/// or append-without-turn cannot acquire this witness. No active tool rule is used.
fn ordinary_work_turn(db:&VerifiedDatabaseConnection<'_>,custody:&PreparedCustody,
    source:&RawSourceRecord,open_id:&str,thread:&str,turn:&str)->Result<WorkTurn> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT request_id,request_hex,receipt_hex FROM main.gogoke_v37_h_stdin_journal
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND ticket=?4 AND custodian_nonce=?5 AND generation=?6
            AND operation='send' AND phase='RECEIPTED' AND receipt_status='APPLIED'")?;
    for (index,value) in [source.domain_id.as_str(),source.session_id.as_str(),
        source.key.operation_id.as_str(),custody.ticket.opaque(),custody.custodian_nonce.as_str(),
        custody.binding.generation.as_str()].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    let mut found=None;
    while q.step_row()? {
        let request_id=q.column_text(0)?;
        let raw=unhex(&q.column_text(1)?)?;
        let receipt_raw=unhex(&q.column_text(2)?)?;
        let request=decode_request(&raw).map_err(|cause|HostHealthError::Wire {
            field:"saved H request",cause})?;
        let receipt=decode_receipt(&receipt_raw).map_err(|cause|HostHealthError::Wire {
            field:"saved H receipt",cause})?;
        if request.family!="K-SESSION" || request.operation!="send" || request.payload.len()!=2
            || request.domain_id!=source.domain_id || request.target_id!=source.session_id
            || request.request_id!=request_id || receipt.status!=V37Status::Applied
            || receipt.family!="K-SESSION" || receipt.operation!="send"
            || receipt.target_id!=source.session_id || receipt.request_id!=request_id
            || receipt.previous_revision!=request.expected_revision
            || Some(receipt.revision)!=request.expected_revision.checked_add(1) {continue}
        let result=receipt.into_result();
        if string(&result,"turnId").as_deref()!=Some(turn)
            || !matches!(result.get(&key("createdTurn")),Some(Json::Bool(true))) {continue}
        if string(&request.payload,"generation").as_deref()!=Some(source.generation.as_str()) {
            return Err(HostHealthError::Denied);
        }
        let body=string(&request.payload,"body").ok_or(HostHealthError::Denied)?;
        let step_id=format!("send-{}",&sha256_hex(&raw)[..40]);
        let ack=Statement::prepare(db.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes),r.source_cursor FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
                AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.generation=?6
                AND s.open_request_id=?7 AND s.step_id=?8 AND s.phase='OBSERVED'
                AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'
                AND r.source_epoch=?9")?;
        for (index,value) in [source.domain_id.as_str(),source.session_id.as_str(),
            source.key.operation_id.as_str(),custody.ticket.opaque(),custody.custodian_nonce.as_str(),
            source.generation.as_str(),open_id,step_id.as_str(),source.key.source_epoch.as_str()]
            .iter().enumerate() {ack.bind_text((index+1) as i32,value)?;}
        if !ack.step_row()? {continue}
        let command_raw=unhex(&ack.column_text(0)?)?;
        let response_raw=unhex(&ack.column_text(1)?)?;
        let ack_cursor=ack.column_text(2)?.parse::<u64>().map_err(|cause|
            HostHealthError::Integer {field:"ordinary ACK cursor",cause})?;
        if ack.step_row()? {return Err(HostHealthError::Conflict)}
        // H's complete_codex_turn_in_transaction already uses this fixed
        // codec before writing APPLIED. Revalidation uses that same mechanism
        // on the exact saved command and A response, not a second JSON meaning.
        let (id,command)=codex_rpc::decode_stored_turn_start(&command_raw)?;
        if !matches!(&command,codex_rpc::Command::TurnStart {thread_id,text,..}
                if thread_id==thread && text==&body)
            || !matches!(codex_rpc::decode(&response_raw,Some((&id,&command)))?,
                codex_rpc::Reply::Turn {turn_id,status:codex_rpc::TurnStatus::InProgress,..} if turn_id==turn) {
            return Err(HostHealthError::Denied);
        }
        let request_sha256=sha256_hex(&raw);
        let command_sha256=sha256_hex(&command_raw);
        let identity=format!("{}\n{}\n{}\n{}",request_sha256,command_sha256,
            source.key.operation_id,custody.custodian_nonce);
        if string(&result,"receiptId")!=Some(format!("rpc-{}",&sha256_hex(identity.as_bytes())[..40])) {
            return Err(HostHealthError::Denied);
        }
        if found.is_some() {return Err(HostHealthError::Conflict)}
        found=Some(WorkTurn {ack_cursor,revision:request.expected_revision+1,request_id,request_sha256,command_sha256,
            receipt_sha256:sha256_hex(&receipt_raw),ack_sha256:sha256_hex(&response_raw)});
    }
    found.ok_or(HostHealthError::Denied)
}

/// A retains every duplicate. Only the first actual terminal of this physical
/// thread/turn may anchor a health seal, even when the earlier terminal carried
/// no actionable cause or has not yet been normalized. No text LIKE or clock rule.
fn first_terminal(db:&VerifiedDatabaseConnection<'_>,source:&RawSourceRecord,
    thread:&str,turn:&str)->Result<bool> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source
          WHERE domain_id=?1 AND session_id=?2 AND operation_id=?3 AND process_ticket=?4
            AND custodian_nonce=?5 AND generation=?6 AND source_epoch=?7
            AND CAST(source_cursor AS INTEGER)<CAST(?8 AS INTEGER)
          ORDER BY CAST(source_cursor AS INTEGER)")?;
    for (index,value) in [source.domain_id.as_str(),source.session_id.as_str(),
        source.key.operation_id.as_str(),source.process_ticket.as_str(),source.custodian_nonce.as_str(),
        source.generation.as_str(),source.key.source_epoch.as_str(),source.key.source_cursor.as_str()]
        .iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    while q.step_row()? {
        let raw=unhex(&q.column_text(0)?)?;
        if let Ok(codex_rpc::Reply::TurnNotification {thread_id,turn_id,status,..})=
            codex_rpc::decode(&raw,None) {
            if thread_id==thread && turn_id==turn && status!=codex_rpc::TurnStatus::InProgress {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// H calls after the original A terminal row and its ledger resolution exist,
/// inside BEGIN on the Owner's verified connection. The physical custody is
/// retained by H, never constructed from notification/request fields.
pub(crate) fn observe_codex_host_health(db:&VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    custody:&PreparedCustody,key:&RawSourceKey)->Result<Option<HostHealthProof>> {
    check_owner_in_current_transaction(db,owner)?;
    let source=ledger::read_captured_raw_source(db,&key.operation_id,&key.source_epoch,&key.source_cursor)?
        .ok_or(HostHealthError::Denied)?;
    let Some((thread,turn,cause))=typed_terminal(&source.raw_bytes)? else {return Ok(None)};
    let physical_generation=custody.binding.generation.parse::<i64>().ok().filter(|value|*value>0
        && value.to_string()==custody.binding.generation).ok_or(HostHealthError::Denied)?;
    if source.key!=*key || source.key.source_epoch!=custody.custodian_nonce
        || source.domain_id!=custody.binding.domain_id || source.generation!=custody.binding.generation
        || source.process_ticket!=custody.ticket.opaque() || source.custodian_nonce!=custody.custodian_nonce
        || key.source_cursor.parse::<u64>().ok().filter(|n|*n>0 && *n<=i64::MAX as u64
            && n.to_string()==key.source_cursor).is_none() {
        return Err(HostHealthError::Denied);
    }
    let (operation,open_request_id,seat_id,incarnation)=rpc_journal::current_codex_model_binding(
        db,custody,&source.domain_id,&source.session_id)?;
    let generation=super::session_binding::authorization_generation(db,
        &source.domain_id,&source.session_id)
        .map_err(|error|HostHealthError::Store(AtomicError::DurabilityContractFailed(
            format!("host health relationship: {error:?}"))))?;
    if operation!=source.key.operation_id || generation_change::active_for_session(db,
        &source.domain_id,&source.session_id)?.is_some() {return Err(HostHealthError::Denied)}
    let registration=ledger::read_registered_session(db,&source.session_id)?.ok_or(HostHealthError::Denied)?;
    if registration.purpose!=SessionPurpose::Work || registration.side_id.is_some()
        || registration.domain_id!=source.domain_id || registration.seat_id!=seat_id {
        return Err(HostHealthError::Denied);
    }
    let actual_thread=rpc_journal::observed_thread_id(db,&source.domain_id,&source.session_id,
        &operation,&source.generation,&open_request_id,custody.ticket.opaque(),&custody.custodian_nonce)?;
    if actual_thread!=thread {return Err(HostHealthError::Denied)}
    if !first_terminal(db,&source,&thread,&turn)? {return Ok(None)}
    let source_event_id=resolved_event(db,&source,&seat_id,&thread,&turn)?;
    let work=ordinary_work_turn(db,custody,&source,&open_request_id,&thread,&turn)?;
    let seat=seat::get(db,&source.domain_id,&seat_id)?.ok_or(HostHealthError::Denied)?;
    if physical_generation<generation || seat.state!=State::Busy
        || seat.incarnation!=incarnation || seat.generation!=generation
        || seat.instance_id.is_empty() {return Err(HostHealthError::Denied)}
    Ok(Some(HostHealthProof {custody:custody.clone(),source:key.clone(),source_event_id,
        raw_sha256:sha256_hex(&source.raw_bytes),raw:source.raw_bytes,
        domain:source.domain_id,session:source.session_id,operation,open_request_id,
        seat:seat_id,incarnation,instance:seat.instance_id,generation,thread,turn,cause,work}))
}

/// Re-read the original bytes and current admission before E/H mutations in
/// the same transaction. An ended work turn is required, never an ACTIVE model
/// tool turn. A stop intent, changed custody/seat or active generation change denies.
pub(crate) fn revalidate_host_health_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,proof:&HostHealthProof)->Result<Seat> {
    let actual=observe_codex_host_health(db,owner,&proof.custody,&proof.source)?
        .ok_or(HostHealthError::Denied)?;
    if actual!=*proof {return Err(HostHealthError::Conflict)}
    seat::get(db,proof.domain_id(),proof.seat_id())?.ok_or(HostHealthError::Denied)
}

/// Composite, private H/A/E authority: an original typed failed ordinary work
/// turn and its exact health repair's definite unsupported response. No caller
/// can construct this from a status string or model output.
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct StalledHealthProof {
    health:HostHealthProof,
    prefix_appends:Vec<String>,
    repair_event:String,
    repair_request:String,
    receipt_sha256:String,
    response_sha256:String,
    event:String,
}
impl StalledHealthProof {
    pub(crate) fn domain_id(&self)->&str {self.health.domain_id()}
    pub(crate) fn seat_id(&self)->&str {self.health.seat_id()}
    pub(crate) fn generation(&self)->i64 {self.health.generation()}
    pub(crate) fn event_id(&self)->&str {&self.event}
    pub(crate) fn source_event_id(&self)->&str {self.health.source_event_id()}
    pub(crate) fn fingerprint(&self)->String {
        let material=[self.health.raw_sha256.as_str(),self.health.work.request_sha256.as_str(),
            self.health.work.receipt_sha256.as_str(),self.health.work.command_sha256.as_str(),
            self.health.work.ack_sha256.as_str(),self.repair_request.as_str(),self.receipt_sha256.as_str(),
            self.response_sha256.as_str(),self.health.domain.as_str(),self.health.session.as_str(),
            self.health.operation.as_str(),self.health.custody.ticket.opaque(),
            self.health.custody.custodian_nonce.as_str(),self.health.custody.binding.generation.as_str(),
            self.health.seat.as_str(),self.health.incarnation.as_str()].join("\n");
        sha256_hex(format!("{material}\n{}",self.prefix_appends.join("\n")).as_bytes())
    }
    pub(crate) fn notice_basis(&self)->String {format!(
        "original failed work turn {}; health repair {} returned definite UNSUPPORTED",
        self.health.turn_id(),self.repair_request)}
}

/// Revisions order H ordinary sends; A cursors order actual provider turns.
/// Neither clocks nor the current idle/busy hint prove absence of later work.
fn no_successor_work(db:&VerifiedDatabaseConnection<'_>,proof:&HostHealthProof)->Result<Vec<String>> {
    let mut prefix_appends=Vec::new();
    let active=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1
          AND (session_id=?2 OR seat_id=?3) AND phase IN ('INTENT','PREPARED','ACTIVE','UNKNOWN')
          AND (COALESCE(process_operation_id,'')<>?4 OR generation<>?5)")?;
    for (i,v) in [proof.domain.as_str(),proof.session.as_str(),proof.seat.as_str(),
        proof.operation.as_str(),proof.custody.binding.generation.as_str()].iter().enumerate() {
        active.bind_text((i+1) as i32,v)?;
    }
    if active.step_row()? {return Err(HostHealthError::Denied);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT request_id,request_hex,COALESCE(receipt_hex,''),phase,COALESCE(receipt_status,'') FROM main.gogoke_v37_h_stdin_journal
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND ticket=?4 AND custodian_nonce=?5 AND generation=?6 AND operation IN ('send','append-without-turn') ORDER BY request_id")?;
    for (i,v) in [proof.domain.as_str(),proof.session.as_str(),proof.operation.as_str(),
        proof.custody.ticket.opaque(),proof.custody.custodian_nonce.as_str(),
        proof.custody.binding.generation.as_str()].iter().enumerate() {q.bind_text((i+1) as i32,v)?;}
    while q.step_row()? {
        if q.column_text(0)?==proof.work.request_id {continue;}
        let request=decode_request(&unhex(&q.column_text(1)?)?).map_err(|cause|
            HostHealthError::Wire {field:"successor H send",cause})?;
        if request.expected_revision>=proof.work.revision {
            if request.operation!="append-without-turn" || q.column_text(3)?!="RECEIPTED"
                || q.column_text(4)?!="APPLIED" {return Err(HostHealthError::Denied);}
            prefix_appends.push(prior_append(db,proof,&request,&unhex(&q.column_text(2)?)?)?);
        }
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT command_hex,COALESCE(source_cursor,'') FROM main.gogoke_v37_rpc_steps
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND ticket=?4 AND custodian_nonce=?5 AND generation=?6")?;
    for (i,v) in [proof.domain.as_str(),proof.session.as_str(),proof.operation.as_str(),
        proof.custody.ticket.opaque(),proof.custody.custodian_nonce.as_str(),
        proof.custody.binding.generation.as_str()].iter().enumerate() {q.bind_text((i+1) as i32,v)?;}
    let terminal=proof.source.source_cursor.parse::<u64>().map_err(|cause|
        HostHealthError::Integer {field:"terminal cursor",cause})?;
    while q.step_row()? {
        let command=unhex(&q.column_text(0)?)?;
        if sha256_hex(&command)==proof.work.command_sha256 {continue;}
        let method=string(&object(&command)?,"method");
        if !matches!(method.as_deref(),Some("turn/start"|"thread/inject_items")) {continue;}
        let cursor=q.column_text(1)?;
        // An unresolved ordinary RPC cannot prove that no work followed.
        let cursor=if cursor.is_empty() {return Err(HostHealthError::Denied)} else {
            cursor.parse::<u64>().map_err(|cause|HostHealthError::Integer {field:"successor RPC cursor",cause})?
        };
        if cursor>=terminal || (method.as_deref()==Some("turn/start") && cursor>proof.work.ack_cursor) {
            return Err(HostHealthError::Denied);
        }
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source WHERE operation_id=?1
          AND source_epoch=?2 AND domain_id=?3 AND session_id=?4
          AND CAST(source_cursor AS INTEGER)>CAST(?5 AS INTEGER)")?;
    for (i,v) in [proof.operation.as_str(),proof.source.source_epoch.as_str(),proof.domain.as_str(),
        proof.session.as_str(),&proof.work.ack_cursor.to_string()].iter().enumerate() {q.bind_text((i+1) as i32,v)?;}
    while q.step_row()? {
        if let Ok(codex_rpc::Reply::TurnNotification {thread_id,turn_id,..})=
            codex_rpc::decode(&unhex(&q.column_text(0)?)?,None) {
            if thread_id==proof.thread && turn_id!=proof.turn {return Err(HostHealthError::Denied);}
        }
    }
    Ok(prefix_appends)
}

fn prior_append(db:&VerifiedDatabaseConnection<'_>,proof:&HostHealthProof,
    request:&super::V37Request,receipt_raw:&[u8])->Result<String> {
    let receipt=decode_receipt(receipt_raw).map_err(|cause|HostHealthError::Wire {field:"prior append receipt",cause})?;
    if request.family!="K-SESSION" || request.domain_id!=proof.domain || request.target_id!=proof.session
        || request.payload.len()!=2 || string(&request.payload,"generation").as_deref()!=Some(proof.custody.binding.generation.as_str())
        || receipt.status!=V37Status::Applied || receipt.family!=request.family
        || receipt.operation!=request.operation || receipt.request_id!=request.request_id
        || receipt.target_id!=request.target_id || receipt.previous_revision!=request.expected_revision
        || Some(receipt.revision)!=request.expected_revision.checked_add(1) {return Err(HostHealthError::Denied);}
    let result=receipt.into_result();
    if !matches!(result.get(&key("createdTurn")),Some(Json::Bool(false))) || result.contains_key(&key("turnId"))
        || string(&result,"deliveryBasis").as_deref()!=Some("NATIVE_INJECT_ITEMS_ACK")
        || string(&result,"generation").as_deref()!=Some(proof.custody.binding.generation.as_str()) {
        return Err(HostHealthError::Denied);
    }
    let step=format!("append-{}",&sha256_hex(&request.raw_bytes)[..40]);
    let ack=Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes),r.source_cursor FROM main.gogoke_v37_rpc_steps s
          JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
            AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
            AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
            AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
          WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
            AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.generation=?6
            AND s.open_request_id=?7 AND s.step_id=?8 AND s.phase='OBSERVED'
            AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE' AND r.source_epoch=?9")?;
    for (i,v) in [proof.domain.as_str(),proof.session.as_str(),proof.operation.as_str(),proof.custody.ticket.opaque(),
        proof.custody.custodian_nonce.as_str(),proof.custody.binding.generation.as_str(),proof.open_request_id.as_str(),
        step.as_str(),proof.source.source_epoch.as_str()].iter().enumerate() {ack.bind_text((i+1) as i32,v)?;}
    if !ack.step_row()? {return Err(HostHealthError::Denied);}
    let command_raw=unhex(&ack.column_text(0)?)?;let response_raw=unhex(&ack.column_text(1)?)?;
    let cursor=ack.column_text(2)?.parse::<u64>().map_err(|cause|HostHealthError::Integer {field:"prior append ACK cursor",cause})?;
    let terminal=proof.source.source_cursor.parse::<u64>().map_err(|cause|HostHealthError::Integer {field:"terminal cursor",cause})?;
    if ack.step_row()? || cursor<=proof.work.ack_cursor || cursor>=terminal {return Err(HostHealthError::Denied);}
    let (id,command)=codex_rpc::decode_stored_append(&command_raw)?;
    if !matches!(&command,codex_rpc::Command::AppendWithoutTurn {thread_id,text}
        if thread_id==&proof.thread && Some(text.clone())==string(&request.payload,"body"))
        || !matches!(codex_rpc::decode(&response_raw,Some((&id,&command)))?,codex_rpc::Reply::Ack {..}) {
        return Err(HostHealthError::Denied);
    }
    let identity=format!("{}\n{}\n{}\n{}",sha256_hex(&request.raw_bytes),sha256_hex(&command_raw),
        proof.operation,proof.custody.custodian_nonce);
    if string(&result,"receiptId")!=Some(format!("rpc-{}",&sha256_hex(identity.as_bytes())[..40])) {
        return Err(HostHealthError::Denied);
    }
    Ok(sha256_hex(format!("{}\n{}\n{}\n{}",sha256_hex(&request.raw_bytes),sha256_hex(receipt_raw),
        sha256_hex(&command_raw),sha256_hex(&response_raw)).as_bytes()))
}

pub(crate) fn observe_stalled_host_health_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,custody:&PreparedCustody,source:&RawSourceKey,repair_event:&str)
    ->Result<Option<StalledHealthProof>> {
    let Some(health)=observe_codex_host_health(db,owner,custody,source)? else {return Ok(None)};
    let prefix_appends=no_successor_work(db,&health)?;
    if repair_event!=format!("health-{}",sha256_hex(health.source_event_id().as_bytes())) {
        return Err(HostHealthError::Denied);
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT session_request_id,receipt_id FROM main.gogoke_v37_seat_health
          WHERE domain_id=?1 AND seat_id=?2 AND event_id=?3 AND generation=?4
            AND source_event_id=?5 AND state='RECEIPTED' AND signal=?6 AND action=?7")?;
    let (signal,action)=match health.cause {Cause::ContextCompact=>("CONTEXT_COMPACT","COMPACT"),
        Cause::RepeatedFailure=>("REPEATED_FAILURE","RENEW")};
    for (i,v) in [health.domain.as_str(),health.seat.as_str(),repair_event,
        custody.binding.generation.as_str(),health.source_event_id(),signal,action]
        .iter().enumerate() {q.bind_text((i+1) as i32,v)?;}
    if !q.step_row()? {return Ok(None)}
    let repair_request=q.column_text(0)?;let receipt_id=q.column_text(1)?;
    if q.step_row()? {return Err(HostHealthError::Conflict)}
    let c=generation_change::read(db,&health.domain,&repair_request)?.ok_or(HostHealthError::Denied)?;
    if c.stage!="UNSUPPORTED" || c.owner_stop_request_id.is_some() || c.unknown_revision.is_some()
        || c.session_id!=health.session || c.old_generation!=custody.binding.generation
        || c.old_operation!=health.operation || c.old_ticket!=custody.ticket.opaque()
        || c.old_nonce!=custody.custodian_nonce || c.thread_id!=health.thread || c.seat_id!=health.seat
        || c.source_watermark<source.source_cursor.parse::<i64>().map_err(|cause|
            HostHealthError::Integer {field:"repair source watermark",cause})? {
        return Err(HostHealthError::Denied);
    }
    // Currently only compact method-not-found has a definite unsupported A
    // fact. RENEW/other remote errors and diagnostic text remain UNKNOWN.
    if health.cause!=Cause::ContextCompact || c.operation!="compact" {return Ok(None)}
    let request_raw=unhex(&c.raw_hex)?;
    let request=decode_request(&request_raw).map_err(|cause|HostHealthError::Wire {field:"repair request",cause})?;
    let step=format!("compact-{}",&sha256_hex(&request_raw)[..40]);
    if c.ack_step_id.as_deref()!=Some(step.as_str()) || request.request_id!=repair_request
        || request.family!="K-SESSION" || request.operation!=c.operation || request.domain_id!=health.domain
        || request.target_id!=health.session || request.payload.len()!=1
        || string(&request.payload,"generation").as_deref()!=Some(c.old_generation.as_str())
        || i64::try_from(request.expected_revision).ok()!=Some(c.previous_revision) {
        return Err(HostHealthError::Denied);
    }
    let ack=Statement::prepare(db.as_ptr(),
        "SELECT open_request_id,source_epoch,source_cursor FROM main.gogoke_v37_rpc_steps
          WHERE domain_id=?1 AND session_id=?2 AND step_id=?3 AND phase='OBSERVED'")?;
    ack.bind_text(1,&health.domain)?;ack.bind_text(2,&health.session)?;ack.bind_text(3,&step)?;
    if !ack.step_row()? || ack.column_text(0)?!=health.open_request_id
        || ack.column_text(1)?!=source.source_epoch
        || ack.column_text(2)?.parse::<i64>().map_err(|cause|
            HostHealthError::Integer {field:"unsupported A cursor",cause})?<=c.source_watermark
        || ack.step_row()? {return Err(HostHealthError::Denied);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT hex(request_bytes),hex(receipt_bytes) FROM main.v37_ledger_receipt
          WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
    q.bind_text(1,&health.domain)?;q.bind_text(2,&repair_request)?;
    if !q.step_row()? || unhex(&q.column_text(0)?)?!=request_raw {return Err(HostHealthError::Denied)}
    let receipt_raw=unhex(&q.column_text(1)?)?;
    if q.step_row()? {return Err(HostHealthError::Conflict)}
    let receipt=decode_receipt(&receipt_raw).map_err(|cause|HostHealthError::Wire {field:"repair receipt",cause})?;
    if receipt.status!=V37Status::Unsupported || receipt.family!=request.family
        || receipt.operation!=request.operation || receipt.request_id!=request.request_id
        || receipt.target_id!=request.target_id || receipt.previous_revision!=request.expected_revision
        || receipt.revision!=request.expected_revision || !receipt.into_result().is_empty()
        || receipt_id!=format!("health-receipt-{}",sha256_hex(&receipt_raw)) {
        return Err(HostHealthError::Denied);
    }
    let response=match rpc_journal::observed_compact_ack(db,&health.domain,&health.session,
        &health.operation,&c.old_generation,&c.old_ticket,&c.old_nonce,&step,&health.thread) {
        Err(rpc_journal::RpcJournalError::Codec(codex_rpc::RpcError::RemoteResponse(raw)))=>raw,
        Ok(_)=>return Ok(None),Err(error)=>return Err(error.into()),
    };
    let root=object(&response)?;
    if !matches!(fields(&root,"error").and_then(|error|error.get(&key("code"))),
        Some(Json::Number(code)) if code=="-32601") {return Ok(None)}
    let response_sha256=sha256_hex(&response);let receipt_sha256=sha256_hex(&receipt_raw);
    // Identity belongs to the original failure/repair, not mutable route or
    // response content. A changed source is a conflicting seal, never a new send.
    let event=format!("stalled-health-{}",sha256_hex(format!("{repair_event}\n{repair_request}").as_bytes()));
    Ok(Some(StalledHealthProof {health,prefix_appends,repair_event:repair_event.into(),repair_request,
        receipt_sha256,response_sha256,event}))
}

pub(crate) fn revalidate_stalled_host_health_in_transaction(db:&VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,proof:&StalledHealthProof)->Result<()> {
    let actual=observe_stalled_host_health_in_transaction(db,owner,&proof.health.custody,
        &proof.health.source,&proof.repair_event)?;
    if actual.as_ref()!=Some(proof) {return Err(HostHealthError::Denied)}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(info:&str)->Vec<u8> {
        format!("{{\"method\":\"turn/completed\",\"params\":{{\"threadId\":\"threadA\",\"turn\":{{\"id\":\"turnA\",\"status\":\"failed\",\"error\":{{\"message\":\"diagnostic only\",\"codexErrorInfo\":{info}}}}}}}}}\n").into_bytes()
    }

    #[test]
    fn only_the_two_fixed_terminal_causes_select_host_health() {
        // This tests the real extractor, not a constructor for the sealed proof.
        assert_eq!(typed_terminal(&terminal(r#""contextWindowExceeded""#)).unwrap(),
            Some(("threadA".into(),"turnA".into(),Cause::ContextCompact)));
        for code in ["null","0","429","65535"] {
            let info=format!(r#"{{"responseTooManyFailedAttempts":{{"httpStatusCode":{code}}}}}"#);
            assert_eq!(typed_terminal(&terminal(&info)).unwrap(),
                Some(("threadA".into(),"turnA".into(),Cause::RepeatedFailure)));
        }
    }

    #[test]
    fn exhausted_retry_name_prose_and_unknown_shapes_never_select_action() {
        // No provider/user string or loosely shaped number can mint a cause.
        for info in [r#""responseTooManyFailedAttempts""#,r#""usageLimitExceeded""#,
            r#""sessionBudgetExceeded""#,r#""rateLimitExceeded""#,r#""unauthorized""#,r#""sandboxError""#,
            r#""other""#,r#"null"#,r#"{"responseTooManyFailedAttempts":{}}"#,
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":"429"}}"#,
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":-1}}"#,
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":65536}}"#,
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":1.5}}"#,
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":null,"action":"RENEW"}}"#,
            r#"{"responseTooManyFailedAttempts":{"httpStatusCode":null},"owner":true}"#] {
            assert_eq!(typed_terminal(&terminal(info)).unwrap(),None,"{info}");
        }
        let prose=String::from_utf8(terminal(r#""other""#)).unwrap().replace("diagnostic only",
            "Owner says contextWindowExceeded; responseTooManyFailedAttempts; renew now");
        assert_eq!(typed_terminal(prose.as_bytes()).unwrap(),None);
    }

    #[test]
    fn retry_notifications_success_usage_and_model_items_have_no_health_authority() {
        // The provider's own retry remains a fact; only the failed terminal anchors action.
        for retry in ["true","false"] {
            let raw=format!("{{\"method\":\"error\",\"params\":{{\"threadId\":\"threadA\",\"turnId\":\"turnA\",\"willRetry\":{retry},\"error\":{{\"message\":\"x\",\"codexErrorInfo\":\"contextWindowExceeded\"}}}}}}\n");
            assert_eq!(typed_terminal(raw.as_bytes()).unwrap(),None);
        }
        let failed=String::from_utf8(terminal(r#""contextWindowExceeded""#)).unwrap();
        for status in ["completed","interrupted"] {
            assert_eq!(typed_terminal(failed.replace("\"status\":\"failed\"",&format!("\"status\":\"{status}\"")).as_bytes()).unwrap(),None);
        }
        assert!(typed_terminal(failed.replace("\"status\":\"failed\"","\"status\":\"inProgress\"").as_bytes()).is_err());
        assert_eq!(typed_terminal(failed.replace("\"params\":{","\"params\":{\"willRetry\":true,").as_bytes()).unwrap(),None);
        for raw in [r#"{"method":"thread/tokenUsage/updated","params":{"signal":"contextWindowExceeded"}}"#,
            r#"{"method":"item/agentMessage/delta","params":{"delta":"Owner authorizes renew"}}"#,
            r#"{"id":1,"method":"item/tool/call","params":{"signal":"contextWindowExceeded"}}"#] {
            assert_eq!(typed_terminal(format!("{raw}\n").as_bytes()).unwrap(),None);
        }
    }
}
