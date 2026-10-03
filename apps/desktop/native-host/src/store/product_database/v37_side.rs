//! User-only D composition. Logical selections are checked against the actual
//! native H/E/A records; the service pipe cannot choose a side-chat principal.
//! K-* envelopes remain frozen. These UI composition frames carry selections,
//! not authority, paths, process identities or a model-provided reference text.
use super::*;
use crate::store::atomic::Parser;
use crate::store::ledger;
use crate::store::sidechat::{self as d, SideError};

const OPEN: &str = "gogoke.37.owner-side-open.v1";
const QUESTION: &str = "gogoke.37.owner-side-question.v1";
const COLLECT: &str = "gogoke.37.owner-side-collect.v1";
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
        if matches!(schema.to_well_formed_string().as_deref(),Some(OPEN|QUESTION|COLLECT)))
}

impl<'root> ProductDatabase<'root> {
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

    pub(super) fn dispatch_owner_side_frame(&mut self, frame:&[u8])->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if !is_owner_side_frame(frame) {return Err(OrchestrationError::Invalid("owner side frame"));}
        let Json::Object(mut fields)=Parser::parse(std::str::from_utf8(frame).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("owner side UTF-8: {error}")))?)? else {
            return Err(OrchestrationError::Invalid("owner side object"));
        };
        match string(&mut fields,"schema")?.as_str() {
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
                    return self.dispatch_user_side(&create);
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
                    seat_id:side_seat,session_id:open.target_id};
                d::execute(&mut self.connection,&self.owner,&create,Some(&binding)).map_err(side_error)
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
            "SELECT side_id,session_id,generation,mode FROM main.gogoke_v37_side_sync WHERE domain_id=?1 AND sync_id=?2")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        if prior.step_row()? {
            if prior.column_text(0)?!=id || prior.column_text(1)?!=request.target_id
                || prior.column_text(2)?!=generation || prior.column_text(3)?!="QUESTION" || prior.step_row()? {
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
        let sync=d::begin_sync(&mut self.connection,&self.owner,&input.domain_id,id,&input,
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
