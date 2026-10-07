//! User-only D composition. Logical selections are checked against the actual
//! native H/E/A records; the service pipe cannot choose a side-chat principal.
//! K-* envelopes remain frozen. These UI composition frames carry selections,
//! not authority, paths, process identities or a model-provided reference text.
use super::*;
use crate::store::atomic::Parser;
use crate::store::ledger;
use crate::store::sidechat::{self as d, SideError};
use crate::store::seat::{self,NativeSeatCall,CallAction};

const OPEN: &str = "gogoke.37.owner-side-open.v1";
const QUESTION: &str = "gogoke.37.owner-side-question.v1";
const COLLECT: &str = "gogoke.37.owner-side-collect.v1";
const THREAD: &str = "gogoke.37.owner-side-thread.v1";
const LIST: &str = "gogoke.37.owner-side-list.v1";
const QUESTION_BOUNDARY: &str = "\nExplicit user question:\n";

fn key(name: &str) -> JsonString { JsonString::from_str(name) }
fn text(value: &str) -> Json { Json::String(key(value)) }
fn string(fields: &mut BTreeMap<JsonString, Json>, name: &'static str) -> Result<String> {
    match fields.remove(&key(name)) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or(OrchestrationError::Invalid(name)),
        _ => Err(OrchestrationError::Invalid(name)),
    }
}
fn decode(bytes: &[u8]) -> Result<V37Request> {
    decode_request(bytes).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("side inner request: {error:?}")))
}
fn side_error(error: SideError) -> OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("native side: {error:?}"))
}
fn from_hex(value: &str) -> Result<Vec<u8>> {
    if value.len()%2!=0 { return Err(OrchestrationError::Invalid("side original hex")); }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("side original hex UTF-8: {error}")))?;
        u8::from_str_radix(text,16).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("side original hex digit: {error}")))
    }).collect()
}
fn sync_reply(sync: &d::Sync) -> Vec<u8> {
    Json::Object(BTreeMap::from([
        (key("schema"),text("gogoke.37.side-sync.v1")),
        (key("sideId"),text(&sync.side_id)),(key("requestId"),text(&sync.sync_id)),
        (key("status"),text(&sync.state)),(key("maySubmit"),Json::Bool(false)),
        (key("nativeReceiptId"),text(&sync.native_receipt_id)),
        (key("generation"),text(&sync.generation)),(key("throughCursor"),text(&sync.through.to_string())),
    ])).canonical().into_bytes()
}

/// A bounded dispatch hint. The parent proves the exact User pipe process
/// before this is inspected, and the handler validates the complete object.
pub(super) fn is_owner_side_frame(frame: &[u8]) -> bool {
    if frame.len()>crate::ipc::MAX_FRAME_BYTES || !super::v37_seat::configuration_depth_ok(frame) { return false; }
    let Ok(value)=std::str::from_utf8(frame) else {return false;};
    let Ok(Json::Object(fields))=Parser::parse(value) else {return false;};
    matches!(fields.get(&key("schema")),Some(Json::String(schema))
        if matches!(schema.to_well_formed_string().as_deref(),Some(OPEN|QUESTION|COLLECT|THREAD|LIST)))
}

impl<'root> ProductDatabase<'root> {
    /// Native model tool, separate from every Owner frame. Model arguments
    /// name only an existing side and the words to send; D/E/H derive every
    /// principal, target and one-shot C identity from sealed current facts.
    pub(super) fn dispatch_model_side_message(&mut self,caller:&NativeSeatCall)->Result<Vec<u8>> {
        if caller.tool()!=Some("gogoke_side_message") {return Err(OrchestrationError::AccessDenied);}
        let args=caller.arguments_json().ok_or(OrchestrationError::AccessDenied)?;
        let Json::Object(mut fields)=Parser::parse(args)? else {return Err(OrchestrationError::Invalid("side tool arguments"));};
        if fields.len()!=4 || string(&mut fields,"operation")?!="send" ||
            !matches!(fields.remove(&key("expectedRevision")),Some(Json::Null)) {
            return Err(OrchestrationError::Invalid("side tool operation"));
        }
        let side_id=string(&mut fields,"targetId")?;
        let Some(Json::Object(mut payload))=fields.remove(&key("payload")) else {
            return Err(OrchestrationError::Invalid("side tool payload"));
        };
        if !fields.is_empty() || payload.len()!=1 {return Err(OrchestrationError::Invalid("side tool fields"));}
        let body=string(&mut payload,"body")?;
        if !payload.is_empty() {return Err(OrchestrationError::Invalid("side tool body"));}
        let domain=caller.domain_id();
        let id=caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?;
        let session=caller.session_id().ok_or(OrchestrationError::AccessDenied)?;
        let side=d::list(&mut self.connection,&self.owner,domain).map_err(side_error)?
            .into_iter().find(|row|row.side_id==side_id).ok_or(OrchestrationError::AccessDenied)?;
        let direction=if caller.seat_id()==side.seat_id && caller.incarnation()==side.seat_incarnation {
            d::delivery::Direction::SideToLead
        } else if caller.seat_id()==side.source_seat_id && caller.incarnation()==side.source_seat_incarnation {
            d::delivery::Direction::LeadToSide
        } else {return Err(OrchestrationError::AccessDenied)};
        let lead_id=side.source_seat_id.clone();let lead_inc=side.source_seat_incarnation.clone();
        let intent=d::delivery::prepare(&mut self.connection,&self.owner,domain,&side_id,id,session,
            direction,&body,|db,source,target| {
                let lead=Statement::prepare(db.as_ptr(),"SELECT 1 FROM main.gogoke_v37_seat_project_lead WHERE domain_id=?1 AND seat_id=?2 AND incarnation=?3")?;
                lead.bind_text(1,domain)?;lead.bind_text(2,&lead_id)?;lead.bind_text(3,&lead_inc)?;
                if !lead.step_row()? || lead.step_row()? {return Ok(false);}
                if source!=caller.seat_id() {return Ok(false);}
                match seat::authorize_current_call(db,caller,domain,target,CallAction::Message) {
                    Ok(_)=>Ok(true),
                    Err(seat::SeatError::Denied|seat::SeatError::Conflict)=>Ok(false),
                    Err(error)=>Err(SideError::Corrupt(format!("E MESSAGE authority: {error:?}"))),
                }
            }).map_err(side_error)?;
        if intent.may_dispatch {
            if let Err(error)=self.dispatch_side_delivery(&intent,caller) {
                d::delivery::record_error(&mut self.connection,&self.owner,domain,id,&format!("{error:?}"))
                    .map_err(side_error)?;
            }
        }
        let observed=d::delivery::observe(&mut self.connection,&self.owner,domain,id).map_err(side_error)?;
        let status=match observed.state {
            d::delivery::DeliveryState::Steered|d::delivery::DeliveryState::NewTurn=>V37Status::Applied,
            d::delivery::DeliveryState::Failed=>V37Status::Failed,
            d::delivery::DeliveryState::Unknown=>V37Status::Unknown,
        };
        let kind=match observed.state {
            d::delivery::DeliveryState::Steered=>"steered",d::delivery::DeliveryState::NewTurn=>"new-turn",
            d::delivery::DeliveryState::Failed=>"failed",d::delivery::DeliveryState::Unknown=>"unknown",
        };
        let receipt=V37Request {raw_bytes:Vec::new(),family:"K-INBOX".into(),operation:"deliver".into(),
            request_id:id.into(),target_id:intent.message_id.clone(),domain_id:domain.into(),
            expected_revision:0,payload:BTreeMap::new()};
        Ok(encode_receipt(&receipt,status,0,0,BTreeMap::from([
            (key("state"),text(kind)),(key("sourceSeatId"),text(&intent.source_seat_id)),
            (key("targetSeatId"),text(&intent.target_seat_id)),
            (key("nativeReceiptId"),text(&observed.native_receipt_id)),
            (key("reason"),text(&observed.reason)),
        ])))
    }
    /// All lifecycle mutations remain D's existing same-connection operations.
    /// Fresh creation requires the native side-open composition below; a plain
    /// wire request cannot manufacture the source or SIDE_CHAT registration.
    pub(super) fn dispatch_user_side(&mut self, request:&V37Request)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        d::execute(&mut self.connection,&self.owner,request,None).map_err(side_error)
    }

    fn side_source_seat(&self, domain:&str, session:&str)->Result<String> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT l.seat_id FROM main.v37_ledger_session l
             JOIN main.gogoke_v37_h_claim h ON h.domain_id=l.domain_id AND h.session_id=l.session_id
             JOIN main.gogoke_v37_h_seat_binding sb ON sb.domain_id=h.domain_id AND sb.session_id=h.session_id AND sb.generation=h.generation AND sb.seat_id=l.seat_id
             JOIN main.gogoke_v37_seats e ON e.domain_id=sb.domain_id AND e.seat_id=sb.seat_id AND e.incarnation=sb.seat_incarnation AND CAST(e.generation AS TEXT)=sb.generation AND e.instance_id=h.instance_id
             JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=h.binding_id AND b.domain_id=h.domain_id AND b.instance_id=h.instance_id AND b.kind='SESSION' AND b.owner_id=h.session_id AND b.generation=h.generation AND b.state='ACTIVE'
             WHERE l.domain_id=?1 AND l.session_id=?2 AND l.purpose='WORK' AND l.side_id IS NULL AND h.state IN ('COMMITTED','STOPPED') AND e.state IN ('BUSY','IDLE')")?;
        q.bind_text(1,domain)?;q.bind_text(2,session)?;
        if !q.step_row()? {return Err(OrchestrationError::AccessDenied);}
        let seat=q.column_text(0)?;
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
        Ok(seat)
    }

    /// The E designation is exact, and an absent or ambiguous live WORK
    /// binding is reported as no source choice. A ledger cursor is a source
    /// position, never a claimed count of lead rounds.
    fn side_current_lead(&self,domain:&str,head:&ledger::LedgerPosition)->Result<Json> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT p.seat_id,p.incarnation,h.session_id,h.generation,h.revision,h.process_operation_id
             FROM main.gogoke_v37_seat_project_lead p
             JOIN main.gogoke_v37_seats e ON e.domain_id=p.domain_id AND e.seat_id=p.seat_id AND e.incarnation=p.incarnation AND e.layer='USER' AND e.state IN ('IDLE','BUSY')
             JOIN main.gogoke_v37_h_seat_binding sb ON sb.domain_id=e.domain_id AND sb.seat_id=e.seat_id AND sb.seat_incarnation=e.incarnation
             JOIN main.gogoke_v37_h_claim h ON h.domain_id=sb.domain_id AND h.session_id=sb.session_id AND h.generation=sb.generation AND h.instance_id=e.instance_id AND h.state='COMMITTED'
             JOIN main.v37_ledger_session l ON l.domain_id=h.domain_id AND l.session_id=h.session_id AND l.seat_id=e.seat_id AND l.purpose='WORK' AND l.side_id IS NULL
             JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=h.binding_id AND b.domain_id=h.domain_id AND b.instance_id=h.instance_id AND b.kind='SESSION' AND b.owner_id=h.session_id AND b.generation=h.generation AND b.state='ACTIVE'
             WHERE p.domain_id=?1")?;
        q.bind_text(1,domain)?;
        let mut found=None;
        while q.step_row()? {
            let seat=q.column_text(0)?;let inc=q.column_text(1)?;
            let session=q.column_text(2)?;let generation=q.column_text(3)?;
            let revision=q.column_text(4)?;let operation=q.column_text(5)?;
            let Some(run)=self.native_sessions.get(&(domain.to_owned(),session.clone())) else {continue};
            if run.operation_id!=operation || run.custody.binding.generation!=generation || !run.allows_input() {
                continue;
            }
            if found.is_some() {return Ok(Json::Null);}
            found=Some(Json::Object(BTreeMap::from([
                (key("seatId"),text(&seat)),(key("seatIncarnation"),text(&inc)),
                (key("sessionId"),text(&session)),(key("generation"),text(&generation)),
                (key("claimRevision"),text(&revision)),
                (key("sourceEpoch"),text(&head.epoch)),
                (key("sourceCursor"),text(&head.cursor.to_string())),
            ])));
        }
        Ok(found.unwrap_or(Json::Null))
    }

    /// Complete an Owner-created side only after its original H/D identities
    /// have produced the two current MESSAGE edges. The policy event is keyed
    /// by the original nested request bytes, so a replay cannot grant a new
    /// seat incarnation or revive an Owner-revoked edge.
    fn finish_side_open_message_pair(&mut self,open:&V37Request,create:&V37Request,
        source_session:&str,receipt:Vec<u8>)->Result<Vec<u8>> {
        let status=super::super::session_transport::decode_receipt(&receipt).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("side create receipt: {error:?}")))?;
        if !matches!(status.status,V37Status::Applied|V37Status::Replayed) {
            return Err(OrchestrationError::V37StoreFailure(format!("side create unresolved: {}",
                String::from_utf8_lossy(&receipt))));
        }
        let side=d::list(&mut self.connection,&self.owner,&create.domain_id).map_err(side_error)?
            .into_iter().find(|side|side.side_id==create.target_id)
            .ok_or(OrchestrationError::OperationConflict)?;
        if side.source_session_id!=source_session || side.session_id!=open.target_id ||
            side.state=="DELETED" {return Err(OrchestrationError::OperationConflict);}
        seat::ensure_side_message_pair(&mut self.connection,&self.owner,&create.domain_id,
            &side.side_id,&side.source_seat_id,&side.source_seat_incarnation,
            &side.seat_id,&side.seat_incarnation,&open.request_id,&open.raw_bytes,
            &create.request_id,&create.raw_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("side MESSAGE pair: {error:?}")))?;
        Ok(receipt)
    }

    pub(super) fn dispatch_owner_side_frame(&mut self, frame:&[u8])->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if !is_owner_side_frame(frame) {return Err(OrchestrationError::Invalid("owner side frame"));}
        let Json::Object(mut fields)=Parser::parse(std::str::from_utf8(frame).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("owner side UTF-8: {error}")))?)? else {
            return Err(OrchestrationError::Invalid("owner side object"));
        };
        match string(&mut fields,"schema")?.as_str() {
            LIST => {
                let domain=string(&mut fields,"domainId")?;
                if !fields.is_empty() {return Err(OrchestrationError::Invalid("side list fields"));}
                let head=ledger::recover(&self.connection)?;
                let lead=self.side_current_lead(&domain,&head)?;
                let registry=d::list(&mut self.connection,&self.owner,&domain).map_err(side_error)?;
                let mut chats=Vec::new();
                for side in registry {
                    let pending_question=Statement::prepare(self.connection.as_ptr(),
                        "SELECT 1 FROM main.gogoke_v37_side_sync WHERE domain_id=?1 AND side_id=?2 AND mode='QUESTION' AND state IN ('PREPARED','UNKNOWN') LIMIT 1")?;
                    pending_question.bind_text(1,&domain)?;pending_question.bind_text(2,&side.side_id)?;
                    let question_unresolved=pending_question.step_row()?;
                    drop(pending_question);
                    // A missing exact H process/claim is unknown to the page,
                    // never an idle session available for a new question.
                    let host=if let Some(run)=self.native_sessions.get(&(domain.clone(),side.session_id.clone())) {
                        if run.operation_id==side.binding_process_operation_id &&
                            run.custody.binding.generation==side.binding_generation {
                            let claim=Statement::prepare(self.connection.as_ptr(),
                                "SELECT revision FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND process_operation_id=?4 AND instance_id=?5 AND state='COMMITTED'")?;
                            for (index,value) in [&domain,&side.session_id,&side.binding_generation,
                                &side.binding_process_operation_id,&side.binding_instance_id].iter().enumerate() {
                                claim.bind_text((index+1) as i32,value)?;
                            }
                            if claim.step_row()? {
                                let revision=claim.column_text(0)?;
                                if claim.step_row()? {return Err(OrchestrationError::OperationConflict);}
                                Json::Object(BTreeMap::from([
                                    (key("sessionId"),text(&side.session_id)),
                                    (key("generation"),text(&side.binding_generation)),
                                    (key("expectedRevision"),text(&revision)),
                                    (key("instanceId"),text(&side.binding_instance_id)),
                                    (key("driverId"),text(run.evidence.driver_id())),
                                    (key("model"),text(&run.model)),(key("effort"),text(&run.effort)),
                                    (key("answering"),Json::Bool(run.turn_id.is_some())),
                                    (key("canAsk"),Json::Bool(run.turn_id.is_none()&&run.allows_input()&&!question_unresolved)),
                                    (key("questionUnresolved"),Json::Bool(question_unresolved)),
                                ]))
                            } else {Json::Null}
                        } else {Json::Null}
                    } else {Json::Null};
                    let lines=d::delivery::lines(&mut self.connection,&self.owner,&domain,&side.side_id)
                        .map_err(side_error)?;
                    let mut transfers=Vec::new();
                    for line in lines {
                        let state=match line.state {
                            d::delivery::DeliveryState::Steered=>"steered",
                            d::delivery::DeliveryState::NewTurn=>"new-turn",
                            d::delivery::DeliveryState::Failed=>"failed",
                            d::delivery::DeliveryState::Unknown=>"unknown",
                        };
                        transfers.push(Json::Object(BTreeMap::from([
                            (key("id"),text(&line.intent.request_id)),
                            (key("direction"),text(&line.intent.direction)),
                            (key("sourceSeatId"),text(&line.intent.source_seat_id)),
                            (key("targetSeatId"),text(&line.intent.target_seat_id)),
                            (key("body"),text(&line.intent.body)),
                            (key("createdAt"),text(&line.intent.created_at)),
                            (key("state"),text(state)),
                            (key("reason"),text(&line.reason)),
                            (key("nativeReceiptId"),text(&line.native_receipt_id)),
                        ])));
                    }
                    chats.push(Json::Object(BTreeMap::from([
                        (key("sideId"),text(&side.side_id)),(key("state"),text(&side.state)),
                        (key("seatId"),text(&side.seat_id)),
                        (key("seatIncarnation"),text(&side.seat_incarnation)),
                        (key("sourceSeatId"),text(&side.source_seat_id)),
                        (key("sourceSeatIncarnation"),text(&side.source_seat_incarnation)),
                        (key("sourceEpoch"),text(&side.epoch)),
                        (key("sourceCursor"),text(&side.cursor.to_string())),
                        (key("syncedCursor"),text(&side.synced_cursor.to_string())),
                        (key("revision"),text(&side.revision.to_string())),
                        (key("host"),host),
                        (key("transfers"),Json::Array(transfers)),
                    ])));
                }
                let bytes=Json::Object(BTreeMap::from([
                    (key("schema"),text("gogoke.37.side-list.v1")),
                    (key("domainId"),text(&domain)),
                    (key("ledgerEpoch"),text(&head.epoch)),
                    (key("ledgerCursor"),text(&head.cursor.to_string())),
                    (key("lead"),lead),
                    (key("chats"),Json::Array(chats)),
                ])).canonical().into_bytes();
                if bytes.len()>crate::ipc::MAX_FRAME_BYTES {
                    return Err(OrchestrationError::Invalid("side list exceeds transport bound"));
                }
                Ok(bytes)
            },
            OPEN => {
                let source=string(&mut fields,"sourceSessionId")?;
                let open=decode(string(&mut fields,"openRequest")?.as_bytes())?;
                let create=decode(string(&mut fields,"createRequest")?.as_bytes())?;
                if !fields.is_empty() || open.family!="K-SESSION" || open.operation!="open"
                    || create.family!="K-SIDE" || create.operation!="create"
                    || create.domain_id!=open.domain_id || source==open.target_id {
                    return Err(OrchestrationError::Invalid("owner side open shape"));
                }
                let known=Statement::prepare(self.connection.as_ptr(),
                    "SELECT source_session_id,session_id FROM main.gogoke_v37_side_registry WHERE domain_id=?1 AND side_id=?2")?;
                known.bind_text(1,&create.domain_id)?;known.bind_text(2,&create.target_id)?;
                if known.step_row()? {
                    if known.column_text(0)?!=source || known.column_text(1)?!=open.target_id || known.step_row()? {
                        return Err(OrchestrationError::OperationConflict);
                    }
                    drop(known);
                    if !self.user_session_request_identity_matches(&open)? {
                        return Err(OrchestrationError::OperationConflict);
                    }
                    // The existing side belongs to its original H open, not
                    // merely to the same logical session selector. A new or
                    // differently encoded nested request cannot claim replay.
                    let original=Statement::prepare(self.connection.as_ptr(),
                        "SELECT request_id,raw_hex,status FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='open'")?;
                    original.bind_text(1,&open.domain_id)?;original.bind_text(2,&open.target_id)?;
                    let raw_hex:String=open.raw_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
                    if !original.step_row()? || original.column_text(0)?!=open.request_id
                        || original.column_text(1)?!=raw_hex || original.column_text(2)?!="APPLIED"
                        || original.step_row()? {return Err(OrchestrationError::OperationConflict);}
                    drop(original);
                    let receipt=self.dispatch_user_side(&create)?;
                    return self.finish_side_open_message_pair(&open,&create,&source,receipt);
                }
                drop(known);
                if create.expected_revision!=0 || create.payload.len()!=1 {
                    return Err(OrchestrationError::Invalid("side create fields"));
                }
                let source_cursor=user_payload_string(&create,"sourceCursor")?;
                if (source_cursor!="0" && source_cursor.starts_with('0'))
                    || !source_cursor.bytes().all(|byte|byte.is_ascii_digit()) {
                    return Err(OrchestrationError::Invalid("side source cursor"));
                }
                let cursor=source_cursor.parse::<u64>().map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("side source cursor: {error}")))?;
                if cursor>ledger::recover(&self.connection)?.cursor {
                    return Err(OrchestrationError::OperationConflict);
                }
                let source_seat=self.side_source_seat(&create.domain_id,&source)?;
                let side_seat=user_payload_string(&open,"seatId")?;
                // H prepares the already admitted process with an empty thread.
                // No send, append or model turn occurs on open.
                let opened=self.dispatch_native_side_open(&open,&create.target_id)?;
                let receipt=super::super::session_transport::decode_receipt(&opened).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("side open receipt: {error:?}")))?;
                if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
                    return Err(OrchestrationError::V37StoreFailure(format!("side open unresolved: {}",
                        String::from_utf8_lossy(&opened))));
                }
                let binding=d::CreateBinding {source_seat_id:source_seat,source_session_id:source,
                    seat_id:side_seat,session_id:open.target_id.clone()};
                let receipt=d::execute(&mut self.connection,&self.owner,&create,Some(&binding))
                    .map_err(side_error)?;
                self.finish_side_open_message_pair(&open,&create,&binding.source_session_id,receipt)
            },
            COLLECT => {
                let domain=string(&mut fields,"domainId")?;
                let id=string(&mut fields,"sideId")?;
                if !fields.is_empty() {return Err(OrchestrationError::Invalid("side collect fields"));}
                let side=self.collect_side_to_head(&domain,&id)?;
                Ok(Json::Object(BTreeMap::from([
                    (key("schema"),text("gogoke.37.side-pending.v1")),(key("sideId"),text(&id)),
                    (key("revision"),text(&side.revision.to_string())),(key("sourceEpoch"),text(&side.epoch)),
                    (key("sourceCursor"),text(&side.cursor.to_string())),(key("syncedCursor"),text(&side.synced_cursor.to_string())),
                ])).canonical().into_bytes())
            },
            THREAD => {
                let domain=string(&mut fields,"domainId")?;
                let id=string(&mut fields,"sideId")?;
                let epoch=string(&mut fields,"ledgerEpoch")?;
                let after=string(&mut fields,"afterCursor")?;
                if !fields.is_empty() {return Err(OrchestrationError::Invalid("side thread fields"));}
                let cursor=after.parse::<u64>().map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("side thread cursor: {error}")))?;
                if cursor.to_string()!=after || cursor>i64::MAX as u64 {
                    return Err(OrchestrationError::Invalid("side thread cursor"));
                }
                // D derives both persisted seat incarnations and sideId before
                // A returns history. Current model admission is not its source.
                let page=d::read_thread(&mut self.connection,&self.owner,&domain,&id,
                    &ledger::LedgerPosition {epoch:epoch.clone(),cursor},2).map_err(side_error)?;
                let mut events=Vec::new();
                for event in page.events {
                    events.push(Json::Object(BTreeMap::from([
                        (key("cursor"),text(&event.cursor.to_string())),
                        (key("sourceEventId"),text(&event.input.event_id)),
                        (key("sourceEpoch"),text(&event.input.source_epoch)),
                        (key("sourceCursor"),text(&event.input.source_cursor)),
                        (key("occurredAt"),text(&event.input.occurred_at)),
                        (key("sessionId"),text(&event.input.session_id)),
                        (key("sideId"),text(&id)),
                        (key("update"),Parser::parse(&event.input.update_json)?),
                    ])));
                }
                let bytes=Json::Object(BTreeMap::from([
                    (key("schema"),text("gogoke.37.side-thread.v1")),(key("sideId"),text(&id)),
                    (key("ledgerEpoch"),text(&epoch)),(key("afterCursor"),text(&after)),
                    (key("cursor"),text(&page.cursor.to_string())),(key("events"),Json::Array(events)),
                ])).canonical().into_bytes();
                if bytes.len()>crate::ipc::MAX_FRAME_BYTES {
                    return Err(OrchestrationError::Invalid("side thread response exceeds transport bound"));
                }
                Ok(bytes)
            },
            QUESTION => {
                let id=string(&mut fields,"sideId")?;
                let request=decode(string(&mut fields,"questionRequest")?.as_bytes())?;
                if !fields.is_empty() || request.family!="K-SESSION" || request.operation!="send"
                    || request.payload.len()!=2 {return Err(OrchestrationError::Invalid("side question fields"));}
                self.dispatch_side_question(&id,&request)
            },
            _ => Err(OrchestrationError::Invalid("owner side schema")),
        }
    }

    fn collect_side_to_head(&mut self,domain:&str,id:&str)->Result<d::Side> {
        let head=ledger::recover(&self.connection)?;
        loop {
            let side=d::collect(&mut self.connection,&self.owner,domain,id,32).map_err(side_error)?;
            if side.epoch!=head.epoch {return Err(OrchestrationError::OperationConflict);}
            if side.cursor>=head.cursor {return Ok(side);}
        }
    }

    fn dispatch_side_question(&mut self,id:&str,request:&V37Request)->Result<Vec<u8>> {
        let question=user_payload_string(request,"body")?;
        if question.trim().is_empty() {return Err(OrchestrationError::Invalid("side explicit question"));}
        let generation=user_payload_string(request,"generation")?;
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT side_id,session_id,generation,mode,origin_request_digest FROM main.gogoke_v37_side_sync WHERE domain_id=?1 AND sync_id=?2")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        if prior.step_row()? {
            if prior.column_text(0)?!=id || prior.column_text(1)?!=request.target_id
                || prior.column_text(2)?!=generation || prior.column_text(3)?!="QUESTION"
                || prior.column_text(4)?!=crate::store::digest::sha256_hex(&request.raw_bytes) || prior.step_row()? {
                return Err(OrchestrationError::OperationConflict);
            }
            drop(prior);
            let original=Statement::prepare(self.connection.as_ptr(),
                "SELECT request_hex FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND request_id=?2")?;
            original.bind_text(1,&request.domain_id)?;original.bind_text(2,&request.request_id)?;
            if original.step_row()? {
                let stored=decode(&from_hex(&original.column_text(0)?)?)?;
                let body=user_payload_string(&stored,"body")?;
                if stored.family!=request.family || stored.operation!=request.operation
                    || stored.domain_id!=request.domain_id || stored.target_id!=request.target_id
                    || stored.expected_revision!=request.expected_revision
                    || user_payload_string(&stored,"generation")?!=generation
                    || body.rsplit_once(QUESTION_BOUNDARY).map(|(_,q)|q)!=Some(text(&question).canonical().as_str())
                    || original.step_row()? {return Err(OrchestrationError::OperationConflict);}
            }
            drop(original);
            // Reconcile the same D/H intention only. In particular, a D intent
            // without an H writer step never grants a replay after restart.
            return d::settle_sync(&mut self.connection,&self.owner,&request.domain_id,&request.request_id)
                .map(|sync|sync_reply(&sync)).map_err(side_error);
        }
        drop(prior);
        d::rebind_current(&mut self.connection,&self.owner,&request.domain_id,id,
            d::read_current_cache_continuity).map_err(side_error)?;
        let live=d::derive_current(&mut self.connection,&self.owner,&request.domain_id,id).map_err(side_error)?;
        if live.session_id!=request.target_id || live.generation!=generation {
            return Err(OrchestrationError::AccessDenied);
        }
        let claim=Statement::prepare(self.connection.as_ptr(),
            "SELECT revision FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND state='COMMITTED'")?;
        claim.bind_text(1,&request.domain_id)?;claim.bind_text(2,&request.target_id)?;claim.bind_text(3,&generation)?;
        if !claim.step_row()? || claim.column_text(0)?!=request.expected_revision.to_string() || claim.step_row()? {
            return Err(OrchestrationError::OperationConflict);
        }
        drop(claim);
        let run=self.native_sessions.get(&(request.domain_id.clone(),request.target_id.clone()))
            .ok_or(OrchestrationError::AccessDenied)?;
        if !run.allows_input() {return Err(OrchestrationError::OperationConflict);}
        let side=self.collect_side_to_head(&request.domain_id,id)?;
        let reference=d::reference_batch(&mut self.connection,&self.owner,&request.domain_id,id,side.cursor).map_err(side_error)?;
        let body=format!("{reference}{QUESTION_BOUNDARY}{}",text(&question).canonical());
        let Json::Object(mut assembled)=Parser::parse(std::str::from_utf8(&request.raw_bytes).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("side request UTF-8: {error}")))?)? else {
            return Err(OrchestrationError::Invalid("side question object"));
        };
        let Some(Json::Object(payload))=assembled.get_mut(&key("payload")) else {
            return Err(OrchestrationError::Invalid("side question payload"));
        };
        payload.insert(key("body"),text(&body));
        let input=decode(Json::Object(assembled).canonical().as_bytes())?;
        let sync=d::begin_sync_from_user(&mut self.connection,&self.owner,&input.domain_id,id,&input,&request.raw_bytes,
            d::SyncMode::Question,side.cursor,|_,_,_|Ok(false)).map_err(side_error)?;
        if sync.may_submit {
            // All current grant and H stdin checks are the actual existing
            // User session route. D records intent first, never a new writer.
            let sent=self.dispatch_user_session(&input);
            let failed=match sent {
                Err(error)=>Some(format!("{error:?}")),
                Ok(bytes)=>{
                    let receipt=super::super::session_transport::decode_receipt(&bytes).map_err(|error|
                        OrchestrationError::V37StoreFailure(format!("side send receipt: {error:?}")))?;
                    (!matches!(receipt.status,V37Status::Applied|V37Status::Replayed))
                        .then(||String::from_utf8_lossy(&bytes).into_owned())
                },
            };
            if let Some(error)=failed {
                let settled=d::settle_sync(&mut self.connection,&self.owner,&input.domain_id,&input.request_id);
                return Err(OrchestrationError::V37StoreFailure(format!("side send: {error}; original settlement: {}",
                    match settled {Ok(sync)=>sync.state,Err(error)=>format!("{error:?}")})));
            }
        }
        d::settle_sync(&mut self.connection,&self.owner,&input.domain_id,&input.request_id)
            .map(|sync|sync_reply(&sync)).map_err(side_error)
    }
}
