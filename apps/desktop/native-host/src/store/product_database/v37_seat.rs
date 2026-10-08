//! User-side E.1/F.1 ingress on the already verified product database.
//! The parent must verify UserOriginProof before invoking either method.
use super::*;
use crate::store::atomic::Parser;
use crate::store::seat::{self, CreateSeat, Kind, NativeOrigin, Seat, SeatChange, SeatError, SeatReceipt, State};

fn key(name: &str) -> JsonString { JsonString::from_str(name) }

struct SecretaryInputHistoryRead {
    seat_id:String,
    incarnation:String,
    authorization_generation:String,
    session_id:String,
    h_generation:String,
    continuation:Option<String>,
}

fn secretary_input_history_read(fields:&BTreeMap<JsonString,Json>)
    ->Result<SecretaryInputHistoryRead> {
    let Some(Json::Object(read))=fields.get(&key("inputHistory")) else {
        return Err(OrchestrationError::Invalid("inputHistory"));
    };
    if read.len()!=5 && read.len()!=6 {
        return Err(OrchestrationError::Invalid("inputHistory fields"));
    }
    let continuation=if read.contains_key(&key("continuation")) {
        let value=string_field(read,"continuation")?;
        if value.len()!=64 || !value.bytes().all(|byte|byte.is_ascii_hexdigit()) {
            return Err(OrchestrationError::Invalid("inputHistory continuation"));
        }
        Some(value)
    } else {None};
    Ok(SecretaryInputHistoryRead {
        seat_id:string_field(read,"seatId")?,
        incarnation:string_field(read,"incarnation")?,
        authorization_generation:string_field(read,"authorizationGeneration")?,
        session_id:string_field(read,"sessionId")?,
        h_generation:string_field(read,"hGeneration")?,
        continuation,
    })
}

pub(super) fn configuration_depth_ok(frame: &[u8]) -> bool {
    let mut depth = 0usize;
    let mut quoted = false;
    let mut escaped = false;
    for &byte in frame {
        if quoted {
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { quoted = false; }
        } else if byte == b'"' { quoted = true; }
        else if byte == b'{' || byte == b'[' {
            depth += 1;
            if depth > 128 { return false; }
        } else if byte == b'}' || byte == b']' { depth = depth.saturating_sub(1); }
    }
    true
}

/// Parent dispatch hint only. A frame with another schema stays on the
/// operations decoder; this does not authenticate the frame or caller.
pub(super) fn is_user_v37_configuration_frame(frame: &[u8]) -> bool {
    if frame.is_empty() || frame.len() > crate::ipc::MAX_FRAME_BYTES
        || !configuration_depth_ok(frame) { return false; }
    let Ok(text) = std::str::from_utf8(frame) else { return false; };
    let Ok(Json::Object(fields)) = Parser::parse(text) else { return false; };
    matches!(fields.get(&key("schema")), Some(Json::String(schema))
        if schema.to_well_formed_string().as_deref() == Some("gogoke.37.owner-configuration.v1"))
}

pub(super) fn string_field(payload: &BTreeMap<JsonString, Json>, name: &'static str) -> Result<String> {
    match payload.get(&key(name)) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .ok_or(OrchestrationError::Invalid(name)),
        _ => Err(OrchestrationError::Invalid(name)),
    }
}

fn exact_payload(request: &V37Request, fields: &[&str]) -> bool {
    request.payload.len() == fields.len()
        && fields.iter().all(|field| request.payload.contains_key(&key(field)))
}

fn seat_revision(seat: &Seat) -> Result<u64> {
    u64::try_from(seat.revision).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("seat revision: {error}")))
}

fn seat_result(seat: &Seat) -> Result<BTreeMap<JsonString, Json>> {
    let mut result = BTreeMap::from([
        (key("layer"), Json::String(JsonString::from_str(match seat.layer {
            seat::Layer::User => "USER", seat::Layer::Lead => "LEAD",
        }))),
        (key("kind"), Json::String(JsonString::from_str(match seat.kind {
            Kind::Long => "LONG", Kind::Short => "SHORT",
        }))),
        (key("state"), Json::String(JsonString::from_str(match seat.state {
            State::Idle => "IDLE", State::Busy => "BUSY", State::Reclaimed => "RECLAIMED",
        }))),
        (key("generation"), Json::String(JsonString::from_str(&seat.generation.to_string()))),
    ]);
    result.insert(key("instanceId"), if seat.instance_id.is_empty() { Json::Null }
        else { Json::String(JsonString::from_str(&seat.instance_id)) });
    result.insert(key("templateId"), match &seat.template_id {
        Some(id) => Json::String(JsonString::from_str(id)), None => Json::Null,
    });
    result.insert(key("settings"), match &seat.settings_json {
        Some(json) => Parser::parse(json).map_err(OrchestrationError::Atomic)?, None => Json::Null,
    });
    Ok(result)
}

fn receipt(request: &V37Request, status: V37Status, previous: u64, revision: u64,
    result: BTreeMap<JsonString, Json>) -> Vec<u8> {
    encode_receipt(request, status, previous, revision, result)
}

fn current(product: &mut ProductDatabase<'_>, request: &V37Request) -> Result<Option<Seat>> {
    product.connection.execute("BEGIN IMMEDIATE")
        .map_err(OrchestrationError::CommitUnknownWithCause)?;
    let found: Result<Option<Seat>> = (|| {
        authority::check_owner_in_current_transaction(&product.connection, &product.owner)?;
        seat::get(&product.connection, &request.domain_id, &request.target_id)
            .map_err(OrchestrationError::from)
    })();
    match found {
        Ok(found) => {
            product.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Ok(found)
        }
        Err(error) => {
            product.connection.execute("ROLLBACK")
                .map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        }
    }
}

fn status_for(error: &SeatError, request: &V37Request, present: Option<&Seat>) -> V37Status {
    match error {
        SeatError::Invalid(_) => V37Status::Denied,
        SeatError::Denied if present.is_some_and(|seat| seat.state == State::Reclaimed) => V37Status::Conflict,
        SeatError::Denied => V37Status::Denied,
        SeatError::Busy | SeatError::Conflict => {
            if present.is_some_and(|seat| u64::try_from(seat.revision).ok() != Some(request.expected_revision))
                && request.operation != "create-from-template" { V37Status::Stale }
            else { V37Status::Conflict }
        }
        SeatError::Unknown => V37Status::Conflict,
        SeatError::Store(_) | SeatError::Open(_) | SeatError::CommitUnknown(_)
        | SeatError::RollbackUnknown(_) | SeatError::HostResourceObservation(_) | SeatError::HostHealthObservation(_)
        | SeatError::InstanceManagement(_) | SeatError::SchemaDrift => V37Status::Unknown,
    }
}

impl<'root> ProductDatabase<'root> {
    /// A USER display projection. E identifies the sole seat; original H/A
    /// records identify its conversation. This never creates a session or
    /// treats a committed H claim as proof that a process survived restart.
    fn read_secretary_conversation_in_transaction(&self, seat_id:&str,
        incarnation:&str, generation:i64)->Result<Json> {
        let text=|value:&str| Json::String(JsonString::from_str(value));
        let state=|value:&str| Json::Object(BTreeMap::from([(key("state"),text(value))]));
        let current=Statement::prepare(self.connection.as_ptr(),
            "SELECT session_id,selected_instance_id \
             FROM main.gogoke_v37_native_selection \
             WHERE domain_id='global' AND seat_id=?1 AND seat_incarnation=?2 \
               AND seat_authorization_generation=?3 ORDER BY session_id LIMIT 2")?;
        current.bind_text(1,seat_id)?;
        current.bind_text(2,incarnation)?;
        current.bind_i64(3,generation)?;
        if current.step_row()? {
            let session=current.column_text(0)?;
            let selected_instance=current.column_text(1)?;
            if current.step_row()? {return Ok(state("CONFLICT"));}
            drop(current);
            return self.read_secretary_conversation_candidate_in_transaction(
                seat_id,incarnation,generation,&session,&selected_instance,true);
        }
        drop(current);
        // Release makes E IDLE and advances its generation, while H/A retain
        // the original authorized session. Only the same incarnation's
        // released history may be presented when no current selection exists.
        let history=Statement::prepare(self.connection.as_ptr(),
            "SELECT session_id,selected_instance_id,seat_authorization_generation \
             FROM main.gogoke_v37_native_selection \
             WHERE domain_id='global' AND seat_id=?1 AND seat_incarnation=?2 \
               AND seat_authorization_generation<?3 \
             ORDER BY seat_authorization_generation DESC,session_id")?;
        history.bind_text(1,seat_id)?;
        history.bind_text(2,incarnation)?;
        history.bind_i64(3,generation)?;
        let mut latest:Option<(i64,Json)>=None;
        let mut unresolved:Option<i64>=None;
        while history.step_row()? {
            let session=history.column_text(0)?;
            let instance=history.column_text(1)?;
            let old_generation=history.column_text(2)?.parse::<i64>().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("secretary historical E generation: {error}")))?;
            let candidate=self.read_secretary_conversation_candidate_in_transaction(
                seat_id,incarnation,old_generation,&session,&instance,false)?;
            let found=matches!(&candidate,Json::Object(fields) if fields.get(&key("state"))
                .is_some_and(|value|matches!(value,Json::String(value)
                    if value.to_well_formed_string().as_deref()==Some("FOUND"))));
            if found {
                if latest.as_ref().is_some_and(|(old,_)|*old==old_generation) {
                    return Ok(state("CONFLICT"));
                }
                if latest.is_none() {latest=Some((old_generation,candidate));}
            } else {
                unresolved=Some(unresolved.map_or(old_generation,|old|old.max(old_generation)));
            }
        }
        drop(history);
        if let Some((selected_generation,conversation))=latest {
            return Ok(if unresolved.is_some_and(|old|old>=selected_generation) {
                state("UNKNOWN")
            } else {conversation});
        }
        if unresolved.is_some() {return Ok(state("UNKNOWN"));}
        // A mismatched incarnation or impossible future generation is not a
        // clean empty history for this singleton seat.
        let other=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_native_selection \
             WHERE domain_id='global' AND seat_id=?1 LIMIT 1")?;
        other.bind_text(1,seat_id)?;
        Ok(if other.step_row()? {state("UNKNOWN")} else {state("NONE")})
    }

    fn read_secretary_conversation_candidate_in_transaction(&self,seat_id:&str,
        incarnation:&str,authorization_generation:i64,session:&str,
        selected_instance:&str,current:bool)->Result<Json> {
        let text=|value:&str| Json::String(JsonString::from_str(value));
        let state=|value:&str| Json::Object(BTreeMap::from([(key("state"),text(value))]));
        let Some(binding)=crate::store::session_transport::session_binding::read(
            &self.connection,"global",session).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("secretary original binding: {error:?}")))?
            else {return Ok(state("UNKNOWN"));};
        if binding.provenance!=crate::store::session_transport::session_binding::Provenance::NativeV2
            || binding.seat_id!=seat_id || binding.seat_incarnation!=incarnation
            || binding.seat_authorization_generation!=authorization_generation
            || binding.selected_instance_id!=selected_instance {
            return Ok(state("UNKNOWN"));
        }
        let Some(registration)=crate::store::ledger::read_registered_session(&self.connection,&session)?
            else {return Ok(state("UNKNOWN"));};
        if registration.domain_id!="global" || registration.seat_id!=seat_id
            || registration.purpose!=crate::store::ledger::SessionPurpose::Secretary {
            return Ok(state("UNKNOWN"));
        }
        // This existing reader checks the original A purpose, current E
        // incarnation, H claim, Owner binding and SESSION home together.
        match self.native_secretary_ledger_reader(&session) {
            Ok(_) => {},
            Err(OrchestrationError::AccessDenied | OrchestrationError::OperationConflict) =>
                return Ok(state("UNKNOWN")),
            Err(error) => return Err(error),
        }
        let claim=Statement::prepare(self.connection.as_ptr(),
            "SELECT generation,revision,state,instance_id,COALESCE(process_operation_id,''), \
                    CASE WHEN stop_fact_id IS NULL THEN '0' ELSE '1' END,COALESCE(stop_fact_id,'') \
             FROM main.gogoke_v37_h_claim WHERE domain_id='global' AND session_id=?1")?;
        claim.bind_text(1,&session)?;
        if !claim.step_row()? {return Ok(state("UNKNOWN"));}
        let claim_generation=claim.column_text(0)?;
        let claim_revision=claim.column_text(1)?;
        let claim_state=claim.column_text(2)?;
        let claim_instance=claim.column_text(3)?;
        let process_operation=claim.column_text(4)?;
        let stop_fact_present=claim.column_text(5)?=="1";
        let stop_fact_id=claim.column_text(6)?;
        if claim.step_row()? {return Ok(state("CONFLICT"));}
        drop(claim);
        if claim_instance!=selected_instance || !matches!(claim_state.as_str(),
            "COMMITTED"|"STOPPED"|"RELEASED") {return Ok(state("UNKNOWN"));}
        if !current && claim_state!="RELEASED" {return Ok(state("UNKNOWN"));}
        if (claim_state=="STOPPED" && !stop_fact_present)
            || (claim_state=="COMMITTED" && stop_fact_present) {
            return Ok(state("UNKNOWN"));
        }
        let stopped=if claim_state=="STOPPED" {
            crate::store::session_transport::runtime::observe_stop_fact(
                &self.connection,"global",&session)?.is_some()
        } else if claim_state=="RELEASED" && stop_fact_present {
            let proof=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_coordination_process_custody \
                 WHERE operation_id=?1 AND domain_id='global' AND generation=?2 \
                   AND state='STOPPED' AND stop_proof_hash=?3")?;
            proof.bind_text(1,&process_operation)?;
            proof.bind_text(2,&claim_generation)?;
            proof.bind_text(3,&stop_fact_id)?;
            let found=proof.step_row()?;
            if found && proof.step_row()? {return Ok(state("CONFLICT"));}
            found
        } else {false};
        if stop_fact_present && !stopped {
            return Ok(state("UNKNOWN"));
        }
        let thread_id=match self.original_native_continuation("global",&session) {
            Ok((_,_,thread_id))=>thread_id,
            Err(OrchestrationError::AccessDenied | OrchestrationError::OperationConflict) =>
                return Ok(state("UNKNOWN")),
            Err(error)=>return Err(error),
        };
        let position=crate::store::ledger::recover(&self.connection)?;
        let runtime_available=if current && claim_state=="COMMITTED" {
            if let Some(run)=self.native_sessions.get(&("global".to_owned(),session.to_owned())) {
                if run.evidence.seat_id()==seat_id
                    && run.evidence.seat_incarnation()==incarnation
                    && run.custody.binding.generation==claim_generation
                    && run.operation_id==process_operation
                    && run.thread_id.as_deref()==Some(thread_id.as_str())
                    && run.allows_input() {
                    if let Some(process)=self.process_custodian.active(&run.custody.ticket) {
                        if process.identity()==&run.custody.identity {
                            process.exit_code().map_err(|error|
                                OrchestrationError::V37StoreFailure(format!(
                                    "secretary runtime exit observation: {error:?}")))?.is_none()
                        } else {false}
                    } else {false}
                } else {false}
            } else {false}
        } else {false};
        // Only Codex maintains turn_id from an observed turn/start ACK and
        // clears it after the matching terminal source is recorded in A.
        // Other providers have no equivalent live turn field in this host.
        let turn_state=if runtime_available {
            match self.native_sessions.get(&("global".to_owned(),session.to_owned())) {
                Some(run) if run.evidence.driver_id()=="codex" =>
                    if run.turn_id.is_some() {"RUNNING"} else {"IDLE"},
                _=>"UNKNOWN",
            }
        } else {"UNKNOWN"};
        Ok(Json::Object(BTreeMap::from([
            (key("state"),text("FOUND")),
            (key("sessionId"),text(&session)),
            (key("generation"),text(&claim_generation)),
            (key("revision"),text(&claim_revision)),
            (key("claimState"),text(&claim_state)),
            (key("stoppedFact"),if stop_fact_present {Json::Bool(stopped)} else {Json::Null}),
            (key("historical"),Json::Bool(!current)),
            (key("runtimeAvailable"),Json::Bool(runtime_available)),
            (key("turnState"),text(turn_state)),
            (key("threadId"),text(&thread_id)),
            (key("ledgerEpoch"),text(&position.epoch)),
            (key("ledgerCursor"),text(&position.cursor.to_string())),
        ])))
    }

    /// Page original H requests only after the direct USER configuration
    /// path has selected this exact E/H/A Secretary conversation. A missing E
    /// INPUT marker is an explicit provenance gap, never a reason to silently
    /// drop an H request or expose its body.
    fn read_secretary_input_history_in_transaction(&self,
        read:&SecretaryInputHistoryRead,conversation:&Json,
        response:&mut BTreeMap<JsonString,Json>)->Result<()> {
        use crate::store::session_transport::{decode_request,read_stdin_journal,
            JournalState,StdinJournalKey};
        let text=|value:&str| Json::String(JsonString::from_str(value));
        let Json::Object(conversation_fields)=conversation else {
            return Err(OrchestrationError::AccessDenied);
        };
        let is=|name:&str,value:&str| matches!(conversation_fields.get(&key(name)),
            Some(Json::String(found)) if found.to_well_formed_string().as_deref()==Some(value));
        if !is("state","FOUND") {
            if read.continuation.is_some() {return Err(OrchestrationError::AccessDenied);}
            response.insert(key("inputHistory"),Json::Object(BTreeMap::from([
                (key("state"),text("UNKNOWN")),
                (key("items"),Json::Array(Vec::new())),
                (key("nextContinuation"),Json::Null),
            ])));
            return Ok(());
        }
        if !is("sessionId",&read.session_id) || !is("generation",&read.h_generation) {
            return Err(OrchestrationError::AccessDenied);
        }
        let scope=format!("gogoke.37.secretary-original-user.v1\n{}\n{}\n{}\n{}\n{}",
            read.seat_id,read.incarnation,read.authorization_generation,
            read.session_id,read.h_generation);
        let base_len=2+response.iter().map(|(name,value)|
            Json::String(name.clone()).canonical().len()+1+value.canonical().len())
            .sum::<usize>()+response.len().saturating_sub(1)
            +1+Json::String(key("conversation")).canonical().len()+1
            +conversation.canonical().len()
            +1+Json::String(key("inputHistory")).canonical().len()+1;
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id,ticket,generation,expected_revision \
             FROM main.gogoke_v37_h_stdin_journal \
             WHERE domain_id='global' AND session_id=?1 \
               AND operation IN ('send','append-without-turn') \
             ORDER BY CAST(generation AS INTEGER),CAST(expected_revision AS INTEGER),request_id")?;
        query.bind_text(1,&read.session_id)?;
        let mut past_cursor=read.continuation.is_none();
        let mut matched_cursor=false;
        let mut last_token=None;
        let mut items=Vec::new();
        let mut items_wire_len=0usize;
        let mut more=false;
        while query.step_row()? {
            let request_id=query.column_text(0)?;
            let ticket=query.column_text(1)?;
            let generation=query.column_text(2)?;
            let revision=query.column_text(3)?;
            let token=crate::store::digest::sha256_hex(format!(
                "{scope}\n{generation}\n{revision}\n{request_id}").as_bytes());
            if !past_cursor {
                if read.continuation.as_deref()==Some(token.as_str()) {
                    past_cursor=true;
                    matched_cursor=true;
                }
                continue;
            }
            let record=read_stdin_journal(&self.connection,&StdinJournalKey {
                domain_id:"global",request_id:&request_id,session_id:&read.session_id,
                ticket:&ticket,generation:&generation,
            }).map_err(|error|OrchestrationError::V37StoreFailure(format!(
                "secretary original H input: {error:?}")))?
                .ok_or(OrchestrationError::OperationConflict)?;
            if record.expected_revision.to_string()!=revision {
                return Err(OrchestrationError::OperationConflict);
            }
            let request=decode_request(&record.request_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!(
                    "secretary original USER request: {error:?}")))?;
            if request.family!="K-SESSION" || request.operation!=record.operation
                || request.payload.len()!=2 || request.domain_id!="global"
                || request.target_id!=read.session_id || request.request_id!=request_id
                || !matches!(request.operation.as_str(),"send"|"append-without-turn")
                || !matches!(request.payload.get(&key("generation")),
                    Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some(generation.as_str())) {
                return Err(OrchestrationError::OperationConflict);
            }
            let body=string_field(&request.payload,"body")?;
            let marker_id=format!("H-USER:global:{}:{}",read.session_id,request_id);
            let marker=Statement::prepare(self.connection.as_ptr(),
                "SELECT CAST(occurred_at_ms AS TEXT),CAST(observed_at_ms AS TEXT) \
                 FROM main.gogoke_v37_seat_secretary_presence \
                 WHERE source_id=?1 AND kind='INPUT' AND source_operation_id=?2 \
                   AND source_epoch=?3 AND source_cursor=?4")?;
            for (index,value) in [marker_id.as_str(),record.process_operation_id.as_str(),
                record.custodian_nonce.as_str(),request_id.as_str()].iter().enumerate() {
                marker.bind_text((index+1) as i32,value)?;
            }
            let occurred=if marker.step_row()? {
                let occurred=marker.column_text(0)?.parse::<i64>().map_err(|error|
                    OrchestrationError::V37StoreFailure(format!(
                        "secretary original USER time: {error}")))?;
                let observed=marker.column_text(1)?.parse::<i64>().map_err(|error|
                    OrchestrationError::V37StoreFailure(format!(
                        "secretary original USER observation: {error}")))?;
                if occurred<=0 || observed<occurred || marker.step_row()? {
                    return Err(OrchestrationError::OperationConflict);
                }
                Some(occurred)
            } else {None};
            let phase=match record.state {JournalState::Prepared=>"PREPARED",
                JournalState::Unknown=>"UNKNOWN",JournalState::Receipted=>"RECEIPTED"};
            let receipt=record.receipt_bytes.as_ref().map(|bytes| {
                std::str::from_utf8(bytes).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!(
                        "secretary original receipt UTF-8: {error}")))
                    .and_then(|text|Parser::parse(text).map_err(OrchestrationError::Atomic))
            }).transpose()?.unwrap_or(Json::Null);
            let item=BTreeMap::from([
                (key("requestId"),text(&request_id)),
                (key("generation"),text(&generation)),
                (key("operation"),text(&record.operation)),
                (key("expectedRevision"),text(&revision)),
                (key("body"),occurred.map(|_|text(&body)).unwrap_or(Json::Null)),
                (key("bodyState"),text(if occurred.is_some() {"VERIFIED"} else {"UNKNOWN"})),
                (key("occurredAtMs"),occurred.map(|at|text(&at.to_string())).unwrap_or(Json::Null)),
                (key("phase"),text(phase)),
                (key("receiptStatus"),record.receipt_status.map(|status|text(status.wire())).unwrap_or(Json::Null)),
                (key("receipt"),receipt),
            ]);
            let mut item=Json::Object(item);
            let page_shell_len=Json::Object(BTreeMap::from([
                    (key("state"),text("FOUND")),
                    (key("sessionId"),text(&read.session_id)),
                    (key("items"),Json::Array(Vec::new())),
                    (key("nextContinuation"),text(&token)),
                ])).canonical().len();
            let fits=|item:&Json| {
                base_len+page_shell_len+items_wire_len
                    +usize::from(!items.is_empty())+item.canonical().len()
                    <=crate::ipc::MAX_FRAME_BYTES
            };
            if !fits(&item) && items.is_empty() {
                // One admitted original may itself fill the ingress frame.
                // Preserve its position and receipt status without pretending
                // its absent body/receipt is a delivered conversation message.
                if let Json::Object(ref mut fields)=item {
                    fields.insert(key("body"),Json::Null);
                    if occurred.is_some() {fields.insert(key("bodyState"),text("TOO_LARGE"));}
                    fields.insert(key("receipt"),Json::Null);
                    fields.insert(key("receiptState"),text("OMITTED_FOR_FRAME"));
                }
            }
            if !fits(&item) {
                if items.is_empty() {return Err(OrchestrationError::Invalid("inputHistory frame bound"));}
                more=true;
                break;
            }
            items_wire_len+=usize::from(!items.is_empty())+item.canonical().len();
            items.push(item);
            last_token=Some(token);
        }
        if read.continuation.is_some() && !matched_cursor {
            return Err(OrchestrationError::Invalid("inputHistory continuation"));
        }
        response.insert(key("inputHistory"),Json::Object(BTreeMap::from([
            (key("state"),text("FOUND")),
            (key("sessionId"),text(&read.session_id)),
            (key("items"),Json::Array(items)),
            (key("nextContinuation"),if more {text(last_token.as_deref().ok_or(
                OrchestrationError::OperationConflict)?)} else {Json::Null}),
        ])));
        Ok(())
    }
    fn read_instance_seat_occupancy(&self, instance_id: &str) -> Result<(Vec<Json>, Option<usize>)> {
        // E owns assignment across every domain. Names are joined by the exact
        // incarnation; a missing name is never replaced with an internal ID.
        let q = Statement::prepare(self.connection.as_ptr(),
            "SELECT s.domain_id,s.seat_id,s.incarnation,CAST(s.generation AS TEXT),s.state,\
                    COALESCE(n.display_name,'') \
             FROM main.gogoke_v37_seats AS s LEFT JOIN main.gogoke_v37_seat_display_names AS n \
               ON n.domain_id=s.domain_id AND n.seat_id=s.seat_id AND n.incarnation=s.incarnation \
             WHERE s.instance_id=?1 AND s.state<>'RECLAIMED' ORDER BY s.domain_id,s.seat_id")?;
        q.bind_text(1, instance_id)?;
        let mut seats = Vec::new();
        let mut bindings = Vec::new();
        while q.step_row()? {
            let domain = q.column_text(0)?;
            let seat = q.column_text(1)?;
            let incarnation = q.column_text(2)?;
            let generation = q.column_text(3)?;
            let busy = q.column_text(4)? == "BUSY";
            let name = q.column_text(5)?;
            seats.push(Json::String(JsonString::from_str(if name.is_empty() {
                "未命名席位"
            } else { &name })));
            bindings.push((domain, seat, incarnation, generation, busy));
        }
        // A persisted COMMITTED/ACTIVE row is not a live process. The count is
        // known only when every unsettled H holder has current E, native and
        // kernel custody evidence. A restarted host retains UNKNOWN.
        let claims = Statement::prepare(self.connection.as_ptr(),
            "SELECT domain_id,session_id,state,COALESCE(process_operation_id,''),\
                    COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim WHERE instance_id=?1")?;
        claims.bind_text(1, instance_id)?;
        let mut seen_busy = Vec::new();
        let mut counted_sessions = Vec::new();
        let mut running = 0usize;
        let mut unknown = false;
        while claims.step_row()? {
            let domain = claims.column_text(0)?;
            let session = claims.column_text(1)?;
            let phase = claims.column_text(2)?;
            let operation = claims.column_text(3)?;
            let stop_fact = claims.column_text(4)?;
            if phase == "RELEASED" {
                // A terminal label alone is insufficient. Holder disappearance
                // releases resources with its own exact completion receipt and
                // deliberately preserves the original NULL StopFact.
                if !operation.is_empty() {
                    if stop_fact.is_empty() {
                        let driver = self.read_registered_instance(instance_id)?;
                        let recovered = match driver.as_ref().map(|instance|instance.driver_id.as_str()) {
                            Some("codex") => self.completed_codex_holder_release(instance_id,&domain,&session,&operation)?,
                            Some("grok") => self.completed_grok_holder_release(instance_id,&domain,&session,&operation)?,
                            _ => false,
                        };
                        if !recovered { unknown = true; }
                        continue;
                    }
                    let stopped = Statement::prepare(self.connection.as_ptr(),
                        "SELECT 1 FROM main.gogoke_coordination_process_custody \
                         WHERE operation_id=?1 AND domain_id=?2 AND state='STOPPED' \
                           AND stop_proof_hash=?3")?;
                    stopped.bind_text(1, &operation)?;
                    stopped.bind_text(2, &domain)?;
                    stopped.bind_text(3, &stop_fact)?;
                    if !stopped.step_row()? || stopped.step_row()? {
                        unknown = true;
                    }
                }
                continue;
            }
            let relationship=crate::store::session_transport::session_binding::current_relationship(
                &self.connection,&domain,&session).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("instance current H/E relationship: {error:?}")))?;
            let Some(relationship)=relationship else {unknown=true;continue;};
            let seat=relationship.seat_id;
            let incarnation=relationship.seat_incarnation;
            let generation=relationship.session_generation;
            let authorization_generation=relationship.seat_authorization_generation.to_string();
            if relationship.instance_id!=instance_id || !bindings.iter().any(|(d,s,i,g,busy)|
                d==&domain && s==&seat && i==&incarnation && g==&authorization_generation && *busy) {
                unknown=true;continue;
            }
            if phase == "STOPPED" {
                let stopped = !stop_fact.is_empty() &&
                    crate::store::session_transport::runtime::observe_stop_fact(
                        &self.connection, &domain, &session)
                        .map_err(|error| OrchestrationError::V37StoreFailure(
                            format!("instance stop observation: {error:?}")))?.is_some();
                if !stopped { unknown = true; }
                else {
                    seen_busy.push((domain.clone(),seat,incarnation,authorization_generation));
                }
                continue;
            }
            if phase != "COMMITTED" || operation.is_empty() {
                unknown = true;
                continue;
            }
            seen_busy.push((domain.clone(),seat.clone(),incarnation.clone(),authorization_generation));
            let claim = crate::store::session_transport::runtime::observe_claim_bound(
                &self.connection, &domain, &seat, &session)
                .map_err(|error| OrchestrationError::V37StoreFailure(
                    format!("instance H claim observation: {error:?}")))?;
            let Some(claim) = claim else { unknown = true; continue; };
            if claim.phase != crate::store::session_transport::runtime::SessionPhase::Committed ||
                claim.instance_id != instance_id || claim.generation != generation ||
                claim.process_operation_id.as_deref() != Some(operation.as_str()) {
                unknown = true;
                continue;
            }
            let Some(run) = self.native_sessions.get(&(domain.clone(), session.clone())) else {
                unknown = true;
                continue;
            };
            if run.operation_id != operation || run.evidence.instance_id() != instance_id ||
                run.evidence.seat_id() != seat || run.evidence.seat_incarnation() != incarnation ||
                run.custody.binding.domain_id != domain ||
                run.custody.binding.generation != generation || !run.allows_input() {
                unknown = true;
                continue;
            }
            let custody = Statement::prepare(self.connection.as_ptr(),
                "SELECT c.state FROM main.gogoke_coordination_process_custody AS c \
                 JOIN main.gogoke_v37_h_process_episode AS e \
                   ON e.process_operation_id=c.operation_id AND e.domain_id=c.domain_id \
                  AND e.generation=c.generation AND e.session_id=?11 \
                  AND e.instance_id=?12 AND e.phase='ACTIVE' \
                 WHERE c.operation_id=?1 AND c.ticket=?2 AND c.custodian_nonce=?3 AND c.pid=?4 \
                   AND c.creation_time_100ns=?5 AND c.image_path=?6 AND c.binary_digest_sha256=?7 \
                   AND c.profile_id=?8 AND c.domain_id=?9 AND c.generation=?10")?;
            let pid = run.custody.identity.pid.to_string();
            let created = run.custody.identity.creation_time_100ns.to_string();
            let image = run.custody.identity.image_path.to_string_lossy();
            for (index, value) in [operation.as_str(), run.custody.ticket.opaque(),
                run.custody.custodian_nonce.as_str(), pid.as_str(), created.as_str(),
                image.as_ref(), run.custody.binding.binary_digest_sha256.as_str(),
                run.custody.binding.profile_id.as_str(), domain.as_str(), generation.as_str(),
                session.as_str(), instance_id]
                .iter().enumerate() { custody.bind_text((index + 1) as i32, value)?; }
            let durable_active = custody.step_row()? && custody.column_text(0)? == "ACTIVE";
            if custody.step_row()? { unknown = true; continue; }
            let live = self.process_custodian.active(&run.custody.ticket)
                .is_some_and(|process| process.identity() == &run.custody.identity &&
                    process.exit_code().ok() == Some(None));
            if !durable_active || !live { unknown = true; continue; }
            running += 1;
            counted_sessions.push((domain, session));
        }
        if bindings.iter().any(|(d,s,i,g,busy)| *busy &&
            !seen_busy.iter().any(|(sd,ss,si,sg)| sd==d && ss==s && si==i && sg==g)) {
            unknown = true;
        }
        // A current native process without the counted H/E tuple must not be
        // erased by an apparently empty durable claim query.
        for ((domain, session), run) in &self.native_sessions {
            if run.evidence.instance_id() != instance_id ||
                counted_sessions.iter().any(|(d,s)| d==domain && s==session) { continue; }
            if self.process_custodian.active(&run.custody.ticket)
                .is_some_and(|process| process.identity() == &run.custody.identity &&
                    process.exit_code().ok() == Some(None)) { unknown = true; }
        }
        Ok((seats, if unknown { None } else { Some(running) }))
    }

    fn read_user_seats_page(&self, domain: &str) -> Result<Json> {
        let cap = Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_project_caps WHERE domain_id=?1")?;
        cap.bind_text(1, domain)?;
        if !cap.step_row()? { return Ok(Json::Null); }
        drop(cap);
        let (limit, _) = seat::read_effective_project_parallel_cap(&self.connection, domain)?;
        let owner_cap = seat::read_project_parallel_cap(&self.connection, domain)?;
        let facts = seat::list_page_facts(&self.connection, domain)?;
        let profiles = instance::read_instance_profiles(&self.connection).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("seat instance profiles: {error:?}")))?;
        let string = |value: &str| Json::String(JsonString::from_str(value));
        let mut refs = BTreeMap::new();
        let mut choices = Vec::new();
        for profile in profiles {
            let mut fields = BTreeMap::from([
                (key("id"), string(&profile.instance_id)),
                (key("name"), string(profile.display_name.as_deref().unwrap_or("未命名实例"))),
                (key("vendor"), string(&profile.driver_id)),
            ]);
            if let Some(models) = instance::read_instance_evidence(&self.connection, &profile.instance_id)
                .map_err(|error| OrchestrationError::V37StoreFailure(format!("seat verified models: {error:?}")))?
                .and_then(|evidence| evidence.available_models_json) {
                let Json::Array(models) = Parser::parse(&models)? else {
                    return Err(OrchestrationError::Invalid("verified model list"));
                };
                fields.insert(key("models"), Json::Array(models));
            }
            let reference = Json::Object(fields);
            let registered = self.read_registered_instance(&profile.instance_id)?
                .ok_or(OrchestrationError::Invalid("seat registered instance"))?;
            if profile.enabled == Some(true) && registered.login_state == "LOGGED_IN"
                && self.current_login_observation(&profile.instance_id, registered.revision, "LOGGED_IN")? {
                choices.push(Parser::parse(&reference.canonical())?);
            }
            refs.insert(profile.instance_id, reference.canonical());
        }
        let mut rows = Vec::new();
        let mut running = 0usize;
        let mut range = None;
        let mut efforts = Vec::new();
        for fact in facts.seats {
            let seat = fact.seat;
            // E BUSY means the persistent CLI owns the seat, not that a model
            // turn is running. Read the original H session for presentation;
            // retain E's separate mutation/quiescence facts below.
            let mut available = false;
            let mut working = false;
            for ((session_domain, session_id), run) in &self.native_sessions {
                if session_domain != domain || run.evidence.seat_id() != seat.seat_id
                    || run.evidence.seat_incarnation() != seat.incarnation { continue; }
                let Some(relationship)=crate::store::session_transport::session_binding::current_relationship(
                    &self.connection,domain,session_id).map_err(|error|
                        OrchestrationError::V37StoreFailure(format!("seat running relationship: {error:?}")))?
                    else { continue; };
                if relationship.seat_id!=seat.seat_id || relationship.seat_incarnation!=seat.incarnation
                    || relationship.seat_authorization_generation!=seat.generation
                    || relationship.instance_id!=seat.instance_id
                    || relationship.session_generation!=run.custody.binding.generation { continue; }
                if run.allows_input() {
                    available = true;
                    working |= run.turn_id.is_some();
                }
            }
            let display_state = match seat.state {
                State::Reclaimed => "REMOVED",
                State::Idle => "IDLE",
                State::Busy if working => "WORKING",
                State::Busy if available => "IDLE",
                State::Busy => "STUCK",
            };
            if display_state == "WORKING" { running += 1; }
            let settings = match seat.settings_json.as_deref() {
                Some(raw) => match Parser::parse(raw)? {
                    Json::Object(fields) => fields,
                    _ => return Err(OrchestrationError::Invalid("seat settings object")),
                },
                None => BTreeMap::new(),
            };
            let setting = |name: &str| match settings.get(&key(name)) {
                Some(Json::String(value)) => Json::String(value.clone()),
                _ => string(""),
            };
            let historical_reference;
            let reference = match refs.get(&seat.instance_id) {
                Some(reference) => reference,
                None if seat.state == State::Reclaimed && !seat.instance_id.is_empty() => {
                    // Instance tombstones retain registry/profile history.
                    // A removed seat must remain readable after its unused
                    // instance is deleted, without making that instance a choice.
                    let registered = self.read_registered_instance(&seat.instance_id)?
                        .ok_or(OrchestrationError::Invalid("historical seat instance"))?;
                    let name = Statement::prepare(self.connection.as_ptr(),
                        "SELECT display_name FROM main.gogoke_v37_instance_profiles WHERE instance_id=?1")?;
                    name.bind_text(1, &seat.instance_id)?;
                    let display_name = if name.step_row()? { name.column_text(0)? }
                        else { "未命名实例".to_owned() };
                    if name.step_row()? { return Err(OrchestrationError::Invalid("historical instance profile rows")); }
                    historical_reference = Json::Object(BTreeMap::from([
                        (key("id"), string(&seat.instance_id)),
                        (key("name"), string(&display_name)),
                        (key("vendor"), string(&registered.driver_id)),
                    ])).canonical();
                    &historical_reference
                },
                None => return Err(OrchestrationError::Invalid("seat instance presentation unavailable")),
            };
            let mut row = BTreeMap::from([
                (key("id"), string(&seat.seat_id)),
                // Native mutation tokens are held by the source adapter, never
                // rendered as product labels or substituted for display names.
                (key("_revision"), string(&seat.revision.to_string())),
                (key("_incarnation"), string(&seat.incarnation)),
                (key("_settings"), match seat.settings_json.as_deref() {
                    Some(raw) => Parser::parse(raw)?, None => Json::Object(BTreeMap::new()),
                }),
                (key("name"), string(fact.display_name.as_deref().unwrap_or("未命名席位"))),
                (key("layer"), string(if seat.layer == seat::Layer::User { "direct" } else { "sub" })),
                (key("isLead"), Json::Bool(fact.is_project_lead)),
                (key("term"), string(if seat.kind == Kind::Long { "long" } else { "short" })),
                (key("state"), string(display_state)),
                (key("allowed"), Json::Object(BTreeMap::from([
                    (key("tune"), Json::Bool(fact.allowed.tune)),
                    (key("changeInstance"), Json::Bool(fact.allowed.change_instance)),
                    (key("remove"), Json::Bool(fact.allowed.remove)),
                ]))),
                (key("instance"), Parser::parse(reference)?), (key("model"), setting("model")),
                (key("effort"), if settings.contains_key(&key("effort")) { setting("effort") }
                    else { setting("reasoningEffort") }),
                (key("permission"), setting("permissionTier")),
            ]);
            if let Some(reason) = fact.allowed.locked_reason {
                if !fact.allowed.change_instance { row.insert(key("instanceLockedReason"), string(reason)); }
            }
            if let Some(card) = fact.state_card.and_then(|card| card.card_json) {
                if let Json::Object(card) = Parser::parse(&card)? {
                    // The original state-card producer uses pendingQuestions.
                    // No card or missing field is not a claim of no questions.
                    for (source, display) in [("goal", "goal"), ("pendingQuestions", "pending")] {
                        if let Some(Json::String(value)) = card.get(&key(source)) {
                            row.insert(key(display), Json::String(value.clone()));
                        }
                    }
                }
            }
            if let Some(scope) = fact.orchestration_scope {
                efforts = scope.reasoning_efforts.iter().map(|effort| string(effort)).collect();
                let permission = match scope.max_permission_tier {
                    seat::PermissionTier::ReadOnly => "READ_ONLY",
                    seat::PermissionTier::NoNetwork => "NO_NETWORK",
                    seat::PermissionTier::IsolatedWrite => "ISOLATED_WRITE",
                    seat::PermissionTier::NetworkedWrite => "NETWORKED_WRITE",
                };
                range = Some(Json::Object(BTreeMap::from([
                    (key("instanceIds"), Json::Array(scope.instance_ids.iter().map(|id| string(id)).collect())),
                    (key("maxPermission"), string(permission)),
                    (key("maxConcurrent"), Json::Number(owner_cap.to_string())),
                ])));
            }
            rows.push(Json::Object(row));
        }
        let mut page = BTreeMap::from([
            (key("running"), Json::Number(running.to_string())),
            (key("limit"), Json::Number(limit.to_string())),
            (key("seats"), Json::Array(rows)), (key("instances"), Json::Array(choices)),
            (key("efforts"), Json::Array(efforts)),
            (key("permissions"), Json::Array(["READ_ONLY", "NO_NETWORK", "ISOLATED_WRITE", "NETWORKED_WRITE"]
                .iter().map(|value| string(value)).collect())),
            (key("templates"), Json::Array(seat::list_templates(&self.connection, domain)?
                .iter().map(|template| string(&template.template_id)).collect())),
        ]);
        if let Some(range) = range { page.insert(key("range"), range); }
        Ok(Json::Object(page))
    }
    /// H may project only the exact User answer which C already settled for
    /// this live turn. Neither model prose nor request payload becomes an
    /// answer; the source reference names C's original receipt.
    pub(crate) fn dispatch_native_takeover_answer(&mut self, request:&V37Request,
        caller:&seat::NativeSeatCall)->Result<Vec<u8>> {
        if request.family!="K-SEAT" || request.operation!="takeover-answers" {
            return Err(OrchestrationError::Invalid("takeover operation"));
        }
        if request.domain_id!=caller.domain_id() || request.target_id!=caller.seat_id() ||
            !exact_payload(request,&["cardId","cardAnswerRequestId","answerRevision"]) {
            return Ok(receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,BTreeMap::new()));
        }
        let card_id=string_field(&request.payload,"cardId")?;
        let answer_request=string_field(&request.payload,"cardAnswerRequestId")?;
        let answer_revision=string_field(&request.payload,"answerRevision")?.parse::<i64>()
            .ok().filter(|value|*value>=0).ok_or(OrchestrationError::Invalid("answerRevision"))?;
        let present=seat::get(&self.connection,&request.domain_id,&request.target_id)?;
        let Some(present)=present else {return Ok(receipt(request,V37Status::Conflict,0,0,BTreeMap::new()));};
        let seat_revision=seat_revision(&present)?;
        if seat_revision!=request.expected_revision {
            return Ok(receipt(request,V37Status::Stale,seat_revision,seat_revision,BTreeMap::new()));
        }
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT c.question_id,c.answer,c.seat_id,c.turn_id,c.generation,o.native_receipt_id,c.vendor_thread_id,c.session_id FROM main.gogoke_v37_qcard_native c JOIN main.gogoke_v37_qcard_native_operations o ON o.domain_id=c.domain_id AND o.card_id=c.card_id WHERE c.domain_id=?1 AND c.card_id=?2 AND o.request_id=?3 AND c.state='ANSWERED' AND o.state='ANSWERED' AND c.answer_kind='WIRE'")?;
        q.bind_text(1,&request.domain_id)?;q.bind_text(2,&card_id)?;
        q.bind_text(3,&answer_request)?;
        if !q.step_row()? {
            return Ok(receipt(request,V37Status::Denied,seat_revision,seat_revision,BTreeMap::new()));
        }
        let question_id=q.column_text(0)?;let answer_wire=q.column_text(1)?;
        let source_seat=q.column_text(2)?;let source_turn=q.column_text(3)?;
        let source_generation=q.column_text(4)?;let source_receipt=q.column_text(5)?;
        let source_thread=q.column_text(6)?;
        let source_session=q.column_text(7)?;
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(q);
        if source_seat!=caller.seat_id() || source_turn!=caller.turn_id() ||
            present.generation!=caller.generation() || source_receipt.is_empty()
            || caller.model_proof().map(|proof|proof.physical_generation())!=Some(source_generation.as_str())
            || caller.thread_id()!=Some(source_thread.as_str()) || caller.session_id()!=Some(source_session.as_str()) {
            return Ok(receipt(request,V37Status::Denied,seat_revision,seat_revision,BTreeMap::new()));
        }
        let Json::Object(wire)=Parser::parse(&answer_wire)? else {return Err(OrchestrationError::Invalid("C answer wire"));};
        let Some(Json::Object(result))=wire.get(&key("result")) else {return Err(OrchestrationError::Invalid("C answer result"));};
        let Some(Json::Object(answers))=result.get(&key("answers")) else {return Err(OrchestrationError::Invalid("C answers"));};
        let Some(Json::Object(answer))=answers.get(&key(&question_id)) else {return Err(OrchestrationError::Invalid("C question answer"));};
        let Some(Json::Array(values))=answer.get(&key("answers")) else {return Err(OrchestrationError::Invalid("C answer values"));};
        let [Json::String(value)]=values.as_slice() else {return Err(OrchestrationError::Invalid("C answer count"));};
        let text=value.to_well_formed_string().filter(|value|!value.is_empty())
            .ok_or(OrchestrationError::Invalid("C answer text"))?;
        let source_ref=format!("C-QCARD:{card_id}:{answer_request}:{source_receipt}");
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_seat_continuity_operations WHERE domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        let replayed=prior.step_row()?;
        if replayed && prior.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(prior);
        let outcome=seat::answer_takeover_from_written_source(&mut self.connection,caller,
            &question_id,&text,seat::AnswerBasis::Cited {source_ref},present.revision,
            answer_revision,&request.request_id,&request.raw_bytes,|db| {
                let session=caller.session_id().ok_or(SeatError::Denied)?;
                let q=Statement::prepare(db.as_ptr(),
                    "SELECT c.question_id,c.answer,c.seat_id,c.turn_id,c.generation,c.vendor_thread_id,o.native_receipt_id,o.request_hex FROM main.gogoke_v37_qcard_native c JOIN main.gogoke_v37_qcard_native_operations o ON o.domain_id=c.domain_id AND o.card_id=c.card_id WHERE c.domain_id=?1 AND c.card_id=?2 AND o.request_id=?3 AND c.state='ANSWERED' AND o.state='ANSWERED' AND c.answer_kind='WIRE'")?;
                q.bind_text(1,&request.domain_id)?;q.bind_text(2,&card_id)?;q.bind_text(3,&answer_request)?;
                if !q.step_row()? || q.column_text(0)?!=question_id || q.column_text(1)?!=answer_wire
                    || q.column_text(2)?!=caller.seat_id() || q.column_text(3)?!=caller.turn_id()
                    || caller.model_proof().map(|proof|proof.physical_generation())!=Some(q.column_text(4)?.as_str())
                    || q.column_text(5)?!=source_thread || q.column_text(6)?!=source_receipt {
                    return Err(SeatError::Denied);
                }
                let original_request=q.column_text(7)?;
                if q.step_row()? {return Err(SeatError::SchemaDrift)};
                drop(q);
                let mut answer_bytes=answer_wire.as_bytes().to_vec();answer_bytes.push(b'\n');
                let expected_hex:String=answer_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
                let written=Statement::prepare(db.as_ptr(),
                    "SELECT step_id,process_operation_id,custodian_nonce,command_hex FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND phase='WRITTEN' AND requires_response=0 AND command_hex=?4")?;
                for (index,value) in [request.domain_id.as_str(),session,source_generation.as_str(),expected_hex.as_str()]
                    .iter().enumerate() {written.bind_text((index+1) as i32,value)?;}
                let mut matches=0;
                while written.step_row()? {
                    let basis=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}",request.domain_id,session,
                        written.column_text(0)?,written.column_text(1)?,written.column_text(2)?,
                        written.column_text(3)?,original_request);
                    if source_receipt==format!("h-qanswer-{}",crate::store::digest::sha256_hex(basis.as_bytes())) {matches+=1;}
                }
                if matches!=1 {return Err(SeatError::Denied)};
                Ok(())
            });
        match outcome {
            Ok(revision)=>Ok(receipt(request,if replayed {V37Status::Replayed} else {V37Status::Applied},seat_revision,seat_revision,
                BTreeMap::from([
                    (key("questionId"),Json::String(JsonString::from_str(&question_id))),
                    (key("answerRevision"),Json::String(JsonString::from_str(&revision.to_string()))),
                ]))),
            Err(error)=>{
                let status=status_for(&error,request,Some(&present));
                let mut result=BTreeMap::new();
                if status==V37Status::Unknown {
                    result.insert(key("reason"),Json::String(JsonString::from_str(&format!("native takeover store: {error:?}"))));
                }
                Ok(receipt(request,status,seat_revision,seat_revision,result))
            }
        }
    }

    fn dispatch_user_state_card(&mut self,request:&V37Request)->Result<Vec<u8>> {
        self.dispatch_state_card(request, None)
    }

    fn dispatch_state_card(&mut self,request:&V37Request,
        caller:Option<&seat::NativeSeatCall>)->Result<Vec<u8>> {
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(||->Result<Vec<u8>> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            if let Some(caller)=caller {
                crate::store::session_transport::model_call::revalidate_model_call_in_transaction(
                    &self.connection,caller).map_err(|error|OrchestrationError::V37StoreFailure(
                        format!("native state card caller: {error:?}")))?;
                if request.target_id!=caller.seat_id() {
                    let child=seat::get(&self.connection,&request.domain_id,&request.target_id)?
                        .ok_or(OrchestrationError::AccessDenied)?;
                    seat::current_child_dispatch_context(&self.connection,caller,&child)
                        .map_err(|error|OrchestrationError::V37StoreFailure(
                            format!("native state card scope: {error:?}")))?;
                }
            }
            let identity=Statement::prepare(self.connection.as_ptr(),
                "SELECT lower(hex(request_bytes)) FROM main.v37_ledger_receipt WHERE family='K-SEAT' AND domain_id=?1 AND request_id=?2")?;
            identity.bind_text(1,&request.domain_id)?;identity.bind_text(2,&request.request_id)?;
            if identity.step_row()? {
                let raw:String=request.raw_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
                let same=identity.column_text(0)?==raw;
                if identity.step_row()? {return Err(OrchestrationError::OperationConflict);}
                if !same {return Ok(receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,BTreeMap::new()));}
            }
            drop(identity);
            let present=seat::get(&self.connection,&request.domain_id,&request.target_id)?;
            let revision=present.as_ref().map(seat_revision).transpose()?.unwrap_or(0);
            if !exact_payload(request,&[]) {
                return Ok(receipt(request,V37Status::Denied,revision,revision,BTreeMap::new()));
            }
            let Some(seat)=present else {
                return Ok(receipt(request,V37Status::Conflict,0,0,BTreeMap::new()));
            };
            if request.expected_revision!=revision {
                return Ok(receipt(request,V37Status::Stale,revision,revision,BTreeMap::new()));
            }
            let card=seat::read_state_card(&self.connection,&seat)?;
            // Missing copied questions are a readable configuration fact,
            // not a completed takeover. The write checks remain strict.
            let questions=match seat::takeover_questions(&seat) {
                Ok(questions)=>Some(questions),
                Err(SeatError::Denied)=>None,
                Err(error)=>return Err(error.into()),
            };
            let mut result=seat_result(&seat)?;
            result.insert(key("takeoverReady"),Json::Bool(card.takeover_ready));
            result.insert(key("takeoverQuestionsConfigured"),Json::Bool(questions.is_some()));
            // The CLI already received these User answers. Its model needs
            // their native C references to consume them without a human
            // copying opaque request IDs. These are not authority tokens;
            // takeover-answers still verifies the original H write in E.
            if let Some(caller)=caller.filter(|caller|caller.seat_id()==request.target_id) {
                let sources=Statement::prepare(self.connection.as_ptr(),
                    "SELECT c.question_id,c.card_id,o.request_id,COALESCE(a.revision,0)
                       FROM main.gogoke_v37_qcard_native c
                       JOIN main.gogoke_v37_qcard_native_operations o
                         ON o.domain_id=c.domain_id AND o.card_id=c.card_id
                       LEFT JOIN main.gogoke_v37_seat_takeover_answers a
                         ON a.domain_id=c.domain_id AND a.seat_id=c.seat_id AND a.question_id=c.question_id
                      WHERE c.domain_id=?1 AND c.seat_id=?2 AND c.turn_id=?3 AND c.generation=?4
                        AND c.vendor_thread_id=?5 AND c.state='ANSWERED' AND o.state='ANSWERED'
                        AND c.answer_kind='WIRE' AND o.native_receipt_id!=''
                      ORDER BY c.card_id,o.request_id")?;
                let generation=caller.model_proof().ok_or(OrchestrationError::AccessDenied)?.physical_generation();
                for (index,value) in [caller.domain_id(),caller.seat_id(),caller.turn_id(),generation,
                    caller.thread_id().ok_or(OrchestrationError::AccessDenied)?].iter().enumerate() {
                    sources.bind_text((index+1) as i32,value)?;
                }
                let mut refs=Vec::new();
                while sources.step_row()? {refs.push(Json::Object(BTreeMap::from([
                    (key("questionId"),Json::String(JsonString::from_str(&sources.column_text(0)?))),
                    (key("cardId"),Json::String(JsonString::from_str(&sources.column_text(1)?))),
                    (key("cardAnswerRequestId"),Json::String(JsonString::from_str(&sources.column_text(2)?))),
                    (key("answerRevision"),Json::String(JsonString::from_str(&sources.column_text(3)?))),
                ])));}
                result.insert(key("nativeAnswerSources"),Json::Array(refs));
            }
            result.insert(key("takeoverQuestions"),Json::Array(questions.into_iter().flatten().map(|question|
                Json::Object(BTreeMap::from([
                    (key("id"),Json::String(JsonString::from_str(&question.id))),
                    (key("prompt"),Json::String(JsonString::from_str(&question.prompt))),
                ]))).collect()));
            result.insert(key("takeoverAnswers"),Json::Array(card.takeover_answers.into_iter().map(|answer|
                Json::Object(BTreeMap::from([
                    (key("questionId"),Json::String(JsonString::from_str(&answer.question_id))),
                    (key("answer"),Json::String(JsonString::from_str(&answer.answer))),
                    (key("basis"),Json::String(JsonString::from_str(&answer.basis))),
                    (key("sourceRef"),Json::String(JsonString::from_str(&answer.source_ref))),
                    (key("howToFind"),Json::String(JsonString::from_str(&answer.how_to_find))),
                ]))).collect()));
            result.insert(key("stateCardRevision"),Json::String(JsonString::from_str(&card.revision.to_string())));
            result.insert(key("stateCard"),match card.card_json {
                Some(json)=>Parser::parse(&json)?,None=>Json::Null,
            });
            let prior=Statement::prepare(self.connection.as_ptr(),
                "SELECT lower(hex(request_bytes)),receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-SEAT' AND domain_id=?1 AND request_id=?2")?;
            prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
            if prior.step_row()? {
                let raw:String=request.raw_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
                let saved_raw=prior.column_text(0)?;
                let saved_receipt=prior.column_text(1)?;
                if prior.step_row()? {return Err(OrchestrationError::OperationConflict);}
                if saved_raw!=raw {return Ok(receipt(request,V37Status::Conflict,revision,revision,BTreeMap::new()));}
                let saved=crate::store::session_transport::decode_receipt(saved_receipt.as_bytes())
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("seat card receipt: {error:?}")))?;
                if saved.family!="K-SEAT"||saved.operation!="state-card"||
                    saved.target_id!=request.target_id {return Err(OrchestrationError::OperationConflict);}
                let old=saved.into_result();
                let same=old.len()==result.len()&&old.iter().all(|(key,value)|
                    result.get(key).is_some_and(|current|current.canonical()==value.canonical()));
                if !same {return Ok(receipt(request,V37Status::Stale,revision,revision,BTreeMap::new()));}
                return Ok(receipt(request,V37Status::Replayed,revision,revision,old));
            }
            drop(prior);
            let bytes=receipt(request,V37Status::Applied,revision,revision,result);
            if bytes.len()>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("state card receipt bound"));}
            let insert=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-SEAT',?1,?2,?3,?4)")?;
            insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
            insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(bytes)
        })();
        match read {
            Ok(bytes)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=>{
                if let Err(error)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!("seat state card: {primary:?}; rollback: {error:?}")));
                }
                Err(primary)
            }
        }
    }

    /// Parent integration: dispatch K-SEAT only after the UserOriginProof check.
    /// The native Owner issuer, never request JSON, establishes the user layer.
    pub(super) fn dispatch_user_seat(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        self.dispatch_seat_request(request, None)
    }

    pub(super) fn dispatch_native_seat(&mut self,request:&V37Request,
        caller:&seat::NativeSeatCall)->Result<Vec<u8>> {
        if request.domain_id!=caller.domain_id() {
            return Err(OrchestrationError::AccessDenied);
        }
        self.dispatch_seat_request(request, Some(caller))
    }

    fn dispatch_seat_request(&mut self,request:&V37Request,
        caller:Option<&seat::NativeSeatCall>)->Result<Vec<u8>> {
        if request.family != "K-SEAT" { return Err(OrchestrationError::Invalid("family")); }
        if request.operation == "takeover-answers" {
            return Ok(receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, BTreeMap::new()));
        }
        if request.operation == "state-card" {return self.dispatch_state_card(request,caller);}
        let prior = current(self, request)?;
        let prior_revision = prior.as_ref().map(seat_revision).transpose()?.unwrap_or(0);
        let admission=caller.map(seat::NativeLeadAdmission::from_model_call).transpose()?;
        let native = match admission.as_ref() {
            Some(admission)=>NativeOrigin::lead(admission),
            None=>NativeOrigin::user(&self.owner),
        };
        let outcome: std::result::Result<SeatReceipt, SeatError> = match request.operation.as_str() {
            "create-from-template" => {
                if request.expected_revision != 0 {
                    return Ok(receipt(request, V37Status::Stale, prior_revision, prior_revision, BTreeMap::new()));
                }
                let expected_fields:&[&str]=if caller.is_some() {&["layer","templateId","instanceId"]}
                    else {&["layer","templateId"]};
                if !exact_payload(request, expected_fields)
                    || string_field(&request.payload, "layer").ok().as_deref()
                        != Some(if caller.is_some() {"LEAD"} else {"USER"}) {
                    return Ok(receipt(request, V37Status::Unsupported, 0, 0, BTreeMap::new()));
                }
                let template_id = match string_field(&request.payload, "templateId") {
                    Ok(value) => value,
                    Err(_) => return Ok(receipt(request, V37Status::Denied, 0, 0, BTreeMap::new())),
                };
                // The private model tool selects an existing logical instance.
                // E checks it against the copied Owner scope in the create
                // transaction. The User wire remains the frozen two fields.
                let instance_id=if caller.is_some() {Some(string_field(&request.payload,"instanceId")?)} else {None};
                let input=CreateSeat {
                    domain_id: &request.domain_id, seat_id: &request.target_id,
                    template_id: &template_id, instance_id: instance_id.as_deref(), kind: Kind::Long,
                    request_id: &request.request_id, request_bytes: &request.raw_bytes,
                };
                match caller {
                    Some(caller)=>seat::create_native_child(&mut self.connection,caller,input),
                    None=>seat::create(&mut self.connection,native,input),
                }
            }
            "tune" | "bind-instance" | "change-instance" | "reclaim" | "short-to-long" => {
                let expected_fields: &[&str] = if request.operation == "change-instance" {
                    &["instanceId", "model", "effort", "permissionTier"]
                } else if request.operation == "bind-instance" {
                    &["instanceId"]
                } else if request.operation == "tune" {
                    &["setting", "value"]
                } else { &[] };
                if !exact_payload(request, expected_fields) {
                    return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new()));
                }
                let expected_revision = match i64::try_from(request.expected_revision) {
                    Ok(value) if value > 0 => value,
                    _ => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                };
                let change = SeatChange { domain_id: &request.domain_id, seat_id: &request.target_id,
                    expected_generation: prior.as_ref().map(|seat| seat.generation).unwrap_or(1),
                    expected_revision, request_id: &request.request_id, request_bytes: &request.raw_bytes };
                match request.operation.as_str() {
                    "tune" => {
                        let setting = match string_field(&request.payload, "setting") {
                            Ok(value) => value,
                            Err(_) => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                        };
                        let value = request.payload.get(&key("value")).expect("exact payload").canonical();
                        seat::tune(&mut self.connection, native, change, &setting, &value)
                    }
                    "bind-instance" => {
                        let instance_id = match string_field(&request.payload, "instanceId") {
                            Ok(value) => value,
                            Err(_) => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                        };
                        seat::bind_instance(&mut self.connection, native, change, &instance_id)
                    }
                    "change-instance" => {
                        let instance_id = match string_field(&request.payload, "instanceId") {
                            Ok(value) => value,
                            Err(_) => return Ok(receipt(request, V37Status::Denied, prior_revision, prior_revision, BTreeMap::new())),
                        };
                        let model = string_field(&request.payload, "model")?;
                        let effort = string_field(&request.payload, "effort")?;
                        let permission = request.payload.get(&key("permissionTier"))
                            .expect("exact payload").canonical();
                        seat::configure_instance(&mut self.connection, native, change,
                            &instance_id, &model, &effort, &permission)
                    }
                    "reclaim" => seat::reclaim(&mut self.connection, native, change),
                    _ => seat::promote(&mut self.connection, native, change),
                }
            }
            _ => return Ok(receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, BTreeMap::new())),
        };
        match outcome {
            Ok(value) => {
                let revision = seat_revision(&value.seat)?;
                let previous = revision.saturating_sub(1);
                Ok(receipt(request, if value.replayed { V37Status::Replayed } else { V37Status::Applied },
                    previous, revision, seat_result(&value.seat)?))
            }
            Err(error) => {
                let after = current(self, request)?;
                let revision = after.as_ref().map(seat_revision).transpose()?.unwrap_or(0);
                let status = status_for(&error, request, after.as_ref());
                let mut result = BTreeMap::new();
                if status == V37Status::Unknown {
                    result.insert(key("reason"), Json::String(JsonString::from_str(&format!("native seat store: {error:?}"))));
                }
                Ok(receipt(request, status, revision, revision, result))
            }
        }
    }

    /// Separate Owner configuration plane. Parent must verify UserOriginProof
    /// before calling; service and seat ingress must never route here.
    pub(super) fn configure_user_v37(&mut self, frame: &[u8]) -> Result<Vec<u8>> {
        if frame.is_empty() || frame.len() > crate::ipc::MAX_FRAME_BYTES {
            return Err(OrchestrationError::Invalid("configuration frame"));
        }
        if !configuration_depth_ok(frame) {
            return Err(OrchestrationError::Invalid("configuration depth"));
        }
        let text = std::str::from_utf8(frame).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("configuration UTF-8: {error}")))?;
        let Json::Object(fields) = Parser::parse(text).map_err(OrchestrationError::Atomic)? else {
            return Err(OrchestrationError::Invalid("configuration object"));
        };
        if string_field(&fields, "schema")?.as_str() != "gogoke.37.owner-configuration.v1" {
            return Err(OrchestrationError::Invalid("configuration schema"));
        }
        let command = string_field(&fields, "command")?;
        if matches!(command.as_str(), "secretary-routines-read" | "secretary-routine-pause"
            | "secretary-routine-resume" | "secretary-routine-delete") {
            return self.dispatch_user_secretary_routine_configuration(&command, &fields, frame);
        }
        if command == "secretary-configuration-read" && (fields.len() == 2
            || (fields.len()==3 && fields.contains_key(&key("inputHistory")))) {
            let history=if fields.len()==3 {Some(secretary_input_history_read(&fields)?)}
                else {None};
            self.connection.execute("BEGIN").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let observed=(|| -> Result<_> {
                let configuration=seat::read_secretary_configuration_in_transaction(
                    &self.connection,&self.owner)?;
                let conversation=match &configuration {
                    seat::SecretaryConfiguration::Designated {seat_id,incarnation,generation,..}=>
                        self.read_secretary_conversation_in_transaction(seat_id,incarnation,*generation)?,
                    _=>Json::Object(BTreeMap::from([(key("state"),
                        Json::String(JsonString::from_str("NONE")))])),
                };
                let text=|value:&str| Json::String(JsonString::from_str(value));
                let optional=|value:Option<String>| value.map(|value|text(&value)).unwrap_or(Json::Null);
                let mut result=BTreeMap::from([
                    (key("schema"),text("gogoke.37.secretary-configuration.v1")),
                ]);
                match configuration {
                    seat::SecretaryConfiguration::Unset=>{result.insert(key("state"),text("UNSET"));},
                    seat::SecretaryConfiguration::Revoked=>{result.insert(key("state"),text("REVOKED"));},
                    seat::SecretaryConfiguration::Designated {seat_id,incarnation,generation,revision,
                        instance_id,model,effort,permission,state}=>{
                        if let Some(read)=&history {
                            if read.seat_id!=seat_id || read.incarnation!=incarnation
                                || read.authorization_generation!=generation.to_string() {
                                return Err(OrchestrationError::AccessDenied);
                            }
                        }
                        result.insert(key("state"),text("DESIGNATED"));
                        result.insert(key("seatId"),text(&seat_id));
                        result.insert(key("incarnation"),text(&incarnation));
                        result.insert(key("generation"),text(&generation.to_string()));
                        result.insert(key("revision"),text(&revision.to_string()));
                        result.insert(key("instanceId"),optional(instance_id));
                        result.insert(key("model"),optional(model));
                        result.insert(key("effort"),optional(effort));
                        let permission=permission.map(|tier|match tier {
                            seat::PermissionTier::ReadOnly=>"READ_ONLY",
                            seat::PermissionTier::NoNetwork=>"NO_NETWORK",
                            seat::PermissionTier::IsolatedWrite=>"ISOLATED_WRITE",
                            seat::PermissionTier::NetworkedWrite=>"NETWORKED_WRITE",
                        }.to_owned());
                        result.insert(key("permissionTier"),optional(permission));
                        result.insert(key("seatState"),text(match state {
                            State::Idle=>"IDLE",State::Busy=>"BUSY",State::Reclaimed=>"RECLAIMED",
                        }));
                    },
                }
                if let Some(read)=&history {
                    if !matches!(result.get(&key("state")),Some(Json::String(value))
                        if value.to_well_formed_string().as_deref()==Some("DESIGNATED")) {
                        return Err(OrchestrationError::AccessDenied);
                    }
                    self.read_secretary_input_history_in_transaction(read,&conversation,&mut result)?;
                }
                result.insert(key("conversation"),conversation);
                let bytes=Json::Object(result).canonical().into_bytes();
                if bytes.len()>crate::ipc::MAX_FRAME_BYTES {
                    return Err(OrchestrationError::Invalid("secretary configuration frame bound"));
                }
                Ok(bytes)
            })();
            return match observed {
                Ok(bytes)=>{
                    self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    Ok(bytes)
                },
                Err(error)=>{
                    self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    Err(error)
                },
            };
        }
        if command == "seats-page-read" && fields.len() == 3 {
            let domain = string_field(&fields, "domainId")?;
            self.connection.execute("BEGIN").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let result = self.read_user_seats_page(&domain);
            match result {
                Ok(page) => {
                    self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    return Ok(page.canonical().into_bytes());
                },
                Err(error) => {
                    self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    return Err(error);
                },
            }
        }
        if command == "instance-management-read" && fields.len() == 2 {
            let optional = |value: Option<String>| value.map(|value|
                Json::String(JsonString::from_str(&value))).unwrap_or(Json::Null);
            let fail = |error| OrchestrationError::V37StoreFailure(format!("instance management read: {error:?}"));
            let mut profiles = Vec::new();
            for profile in instance::read_instance_profiles(&self.connection).map_err(fail)? {
                let evidence = instance::read_instance_evidence(&self.connection, &profile.instance_id).map_err(fail)?;
                let mut row = BTreeMap::from([
                    (key("instanceId"), Json::String(JsonString::from_str(&profile.instance_id))),
                    (key("driverId"), Json::String(JsonString::from_str(&profile.driver_id))),
                    (key("name"), optional(profile.display_name)),
                    (key("enabled"), profile.enabled.map(Json::Bool).unwrap_or(Json::Null)),
                    (key("provider"), optional(profile.connected_model_source)),
                    (key("profileRevision"), optional(profile.revision.map(|value| value.to_string()))),
                ]);
                let cap = Statement::prepare(self.connection.as_ptr(),
                    "SELECT concurrency_cap FROM main.gogoke_v37_instance_caps WHERE instance_id=?1")?;
                cap.bind_text(1, &profile.instance_id)?;
                if cap.step_row()? {
                    row.insert(key("cap"), Json::Number(instance::read_instance_concurrency_cap(
                        &self.connection, &profile.instance_id)?.to_string()));
                    if cap.step_row()? { return Err(OrchestrationError::Invalid("instance cap rows")); }
                }
                if let Some(evidence) = evidence {
                    row.insert(key("account"), optional(evidence.masked_account));
                    row.insert(key("plan"), optional(evidence.subscription));
                    row.insert(key("lastConfirmed"), optional(evidence.account_confirmed_at));
                    row.insert(key("checkFailed"), optional(evidence.detect_error));
                    row.insert(key("modelsSource"), optional(evidence.models_source));
                    row.insert(key("modelsObservedAt"), optional(evidence.models_observed_at));
                    if let Some(models) = evidence.available_models_json {
                        let Json::Array(models) = Parser::parse(&models)? else {
                            return Err(OrchestrationError::Invalid("verified instance models"));
                        };
                        row.insert(key("models"), Json::Array(models));
                    }
                }
                let (seats, running) = self.read_instance_seat_occupancy(&profile.instance_id)?;
                row.insert(key("seats"), Json::Array(seats));
                row.insert(key("runningSessions"), running.map(|value|
                    Json::Number(value.to_string())).unwrap_or(Json::Null));
                profiles.push(Json::Object(row));
            }
            let mut cli = Vec::new();
            for driver in ["codex", "claude", "opencode", "grok"] {
                let copy = instance::read_managed_cli(&self.connection, self.root, driver)
                    .map_err(|error| OrchestrationError::V37StoreFailure(format!("managed CLI read: {error:?}")))?;
                let Some(copy) = copy else { continue; };
                let mut row = BTreeMap::from([
                    (key("driverId"), Json::String(JsonString::from_str(driver))),
                    (key("state"), Json::String(JsonString::from_str(&copy.state))),
                    (key("version"), optional(copy.version)),
                    (key("previousVersion"), optional(copy.previous_version)),
                    (key("checkedAt"), optional(copy.checked_at)),
                    (key("officialVersion"), optional(copy.official_notice)),
                    (key("raw"), optional(copy.raw_error)),
                    (key("progressBytes"), Json::Number(copy.progress_bytes.to_string())),
                ]);
                // A compiled pin is a qualification constraint, never an observation
                // of a newer usable installation. No verified upgrade is invented.
                row.insert(key("verifiedVersion"), Json::Null);
                cli.push(Json::Object(row));
            }
            return Ok(Json::Object(BTreeMap::from([
                (key("schema"), Json::String(JsonString::from_str("gogoke.37.instance-management.v1"))),
                (key("profiles"), Json::Array(profiles)), (key("cli"), Json::Array(cli)),
            ])).canonical().into_bytes());
        }
        let cap = |name: &'static str| -> Result<i64> {
            match fields.get(&key(name)) {
                Some(Json::Number(value)) => value.parse::<i64>().ok().filter(|value| *value > 0)
                    .ok_or(OrchestrationError::Invalid(name)),
                _ => Err(OrchestrationError::Invalid(name)),
            }
        };
        let revision = |name: &'static str| -> Result<i64> {
            let value = string_field(&fields, name)?;
            value.parse::<i64>().ok().filter(|value| *value >= 0)
                .ok_or(OrchestrationError::Invalid(name))
        };
        let policy_domain = if command.starts_with("policy-") || command=="seat-template" {
            Some((string_field(&fields, "domainId")?, string_field(&fields, "requestId")?))
        } else { None };
        let policy = match command.as_str() {
            "seat-template" if fields.len() == 6 => {
                let template_id=string_field(&fields,"templateId")?;
                let settings=match fields.get(&key("settings")) {
                    Some(value @ Json::Object(_))=>value.canonical(),
                    _=>return Err(OrchestrationError::Invalid("settings")),
                };
                let (domain,request_id)=policy_domain.as_ref().expect("template command");
                Some(seat::apply_owner_policy_configuration(&mut self.connection,&self.owner,
                    domain,request_id,frame,seat::OwnerPolicyCommand::Template {
                        template_id:&template_id,settings_json:settings.as_bytes()})?)
            }
            "policy-initialize" if fields.len() == 6 => {
                let stage = string_field(&fields, "stage")?;
                if revision("expectedRevision")? != 0 {
                    return Err(OrchestrationError::Invalid("expectedRevision"));
                }
                let (domain, request_id) = policy_domain.as_ref().expect("policy command");
                Some(seat::apply_owner_policy_configuration(&mut self.connection, &self.owner,
                    domain, request_id, frame, seat::OwnerPolicyCommand::Initialize {stage:&stage})?)
            }
            "policy-call-grant" if fields.len() == 9 => {
                let caller = string_field(&fields, "callerSeatId")?;
                let target = string_field(&fields, "targetId")?;
                let action = match string_field(&fields, "action")?.as_str() {
                    "DISPATCH" => seat::CallAction::Dispatch,
                    "REVIEW" => seat::CallAction::Review,
                    "MESSAGE" => seat::CallAction::Message,
                    "MERGE" => seat::CallAction::Merge,
                    _ => return Err(OrchestrationError::Invalid("action")),
                };
                let expires_at_ms = match fields.get(&key("expiresAtMs")) {
                    Some(Json::Null) => None,
                    Some(Json::String(value)) => Some(value.to_well_formed_string()
                        .and_then(|text| text.parse::<i64>().ok())
                        .filter(|value| *value > 0)
                        .ok_or(OrchestrationError::Invalid("expiresAtMs"))?),
                    _ => return Err(OrchestrationError::Invalid("expiresAtMs")),
                };
                let (domain, request_id) = policy_domain.as_ref().expect("policy command");
                Some(seat::apply_owner_policy_configuration(&mut self.connection, &self.owner,
                    domain, request_id, frame, seat::OwnerPolicyCommand::Grant { caller: &caller,
                    target: &target, action, expires_at_ms,
                    expected_revision: revision("expectedRevision")? })?)
            }
            "policy-gate" if fields.len() == 11 => {
                let gate_id = string_field(&fields, "gateId")?;
                let submitter = string_field(&fields, "submitterSeatId")?;
                let reviewer = string_field(&fields, "reviewerSeatId")?;
                let from_stage = string_field(&fields, "fromStage")?;
                let to_stage = string_field(&fields, "toStage")?;
                let (domain, request_id) = policy_domain.as_ref().expect("policy command");
                Some(seat::apply_owner_policy_configuration(&mut self.connection, &self.owner,
                    domain, request_id, frame, seat::OwnerPolicyCommand::Gate {gate_id:&gate_id,submitter:&submitter,
                    reviewer:&reviewer,from_stage:&from_stage,to_stage:&to_stage,
                    reject_cap:cap("rejectCap")?,expected_revision:revision("expectedRevision")?})?)
            }
            "policy-escalation-route" if fields.len() == 8 => {
                let from_seat = string_field(&fields, "fromSeatId")?;
                let reason = string_field(&fields, "reason")?;
                let to_seat = string_field(&fields, "toSeatId")?;
                let (domain, request_id) = policy_domain.as_ref().expect("policy command");
                Some(seat::apply_owner_policy_configuration(&mut self.connection, &self.owner,
                    domain, request_id, frame, seat::OwnerPolicyCommand::Route {from_seat:&from_seat,
                    reason:&reason,to_seat:&to_seat,expected_revision:revision("expectedRevision")?})?)
            }
            _ => None,
        };
        if let Some((revision, replayed)) = policy {
            let (_, request_id) = policy_domain.as_ref().expect("policy command");
            return Ok(Json::Object(BTreeMap::from([
                (key("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
                (key("command"), Json::String(JsonString::from_str(&command))),
                (key("requestId"), Json::String(JsonString::from_str(&request_id))),
                (key("status"), Json::String(JsonString::from_str(if replayed {"REPLAYED"} else {"APPLIED"}))),
                (key("revision"), Json::String(JsonString::from_str(&revision.to_string()))),
            ])).canonical().into_bytes());
        }
        match command.as_str() {
            "secretary-designate" if fields.len() == 5 => {
                let designated=seat::designate_secretary(&mut self.connection,&self.owner,
                    &string_field(&fields,"seatId")?,&string_field(&fields,"incarnation")?,
                    &string_field(&fields,"requestId")?,frame)?;
                return Ok(Json::Object(BTreeMap::from([
                    (key("schema"),Json::String(JsonString::from_str("gogoke.37.secretary-configuration.v1"))),
                    (key("status"),Json::String(JsonString::from_str(if designated.replayed {"REPLAYED"} else {"APPLIED"}))),
                    (key("seatId"),Json::String(JsonString::from_str(&designated.seat_id))),
                    (key("incarnation"),Json::String(JsonString::from_str(&designated.incarnation))),
                ])).canonical().into_bytes());
            },
            "secretary-configure" if fields.len() == 9 => {
                let permission=fields.get(&key("permissionTier"))
                    .ok_or(OrchestrationError::Invalid("permissionTier"))?.canonical();
                let configured=seat::configure_secretary(&mut self.connection,&self.owner,
                    revision("expectedGeneration")?,revision("expectedRevision")?,
                    &string_field(&fields,"requestId")?,frame,&string_field(&fields,"instanceId")?,
                    &string_field(&fields,"model")?,&string_field(&fields,"effort")?,&permission)?;
                let mut result=seat_result(&configured.seat)?;
                result.insert(key("schema"),Json::String(JsonString::from_str("gogoke.37.secretary-configuration.v1")));
                result.insert(key("status"),Json::String(JsonString::from_str(if configured.replayed {"REPLAYED"} else {"APPLIED"})));
                result.insert(key("revision"),Json::String(JsonString::from_str(&configured.seat.revision.to_string())));
                return Ok(Json::Object(result).canonical().into_bytes());
            },
            "seat-rename" | "seat-designate-lead" => {
                let expected = if command == "seat-rename" { 6 } else { 5 };
                if fields.len() != expected { return Err(OrchestrationError::Invalid("seat metadata fields")); }
                let domain = string_field(&fields, "domainId")?;
                let seat_id = string_field(&fields, "seatId")?;
                let incarnation = string_field(&fields, "incarnation")?;
                if command == "seat-rename" {
                    seat::rename_seat(&mut self.connection, &self.owner, &domain, &seat_id,
                        &incarnation, &string_field(&fields, "name")?)?;
                } else {
                    seat::designate_project_lead(&mut self.connection, &self.owner, &domain,
                        &seat_id, &incarnation)?;
                }
            }
            "instance-profile" if fields.len() == 8 => {
                let id = string_field(&fields, "instanceId")?;
                let name = string_field(&fields, "name")?;
                let enabled = match fields.get(&key("enabled")) {
                    Some(Json::Bool(value)) => *value,
                    _ => return Err(OrchestrationError::Invalid("enabled")),
                };
                let provider = match fields.get(&key("provider")) {
                    Some(Json::Null) => None,
                    Some(Json::String(value)) => Some(value.to_well_formed_string()
                        .ok_or(OrchestrationError::Invalid("provider"))?),
                    _ => return Err(OrchestrationError::Invalid("provider")),
                };
                let expected = match fields.get(&key("expectedProfileRevision")) {
                    Some(Json::Null) => None,
                    Some(Json::String(_)) => Some(revision("expectedProfileRevision")?),
                    _ => return Err(OrchestrationError::Invalid("expectedProfileRevision")),
                };
                // requestId binds the UI write intent; native CAS remains authoritative.
                string_field(&fields, "requestId")?;
                instance::set_instance_profile(&mut self.connection, &self.owner, &id, &name,
                    enabled, provider.as_deref(), expected).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("instance profile write: {error:?}")))?;
            }
            "instance-remove" if fields.len() == 4 => {
                let id = string_field(&fields, "instanceId")?;
                instance::tombstone_unused_instance(&mut self.connection, &self.owner,
                    &id, revision("expectedProfileRevision")?).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("instance remove: {error:?}")))?;
            }
            "project-parallel-cap" if fields.len() == 4 => {
                let domain = string_field(&fields, "domainId")?;
                seat::set_project_parallel_cap(&mut self.connection, &self.owner, &domain, cap("value")?)?;
            }
            "instance-concurrency-cap" if fields.len() == 4 => {
                let instance = string_field(&fields, "instanceId")?;
                instance::set_instance_concurrency_cap(&mut self.connection, &self.owner, &instance, cap("value")?)?;
            }
            "worktree-source" if fields.len() == 5 => {
                let repository = string_field(&fields, "repositoryId")?;
                let source = string_field(&fields, "sourcePath")?;
                let program = string_field(&fields, "gitPath")?;
                let pin = super::super::worktree::GitProgramPin::observe(
                    &mut self.connection, &self.owner, self.root,
                    Path::new(&program), &mut self.process_custodian)
                    .map_err(|error| OrchestrationError::V37StoreFailure(format!("native Git pin: {error:?}")))?;
                super::super::worktree::register_source(&mut self.connection, self.root,
                    &self.owner, &pin, &mut self.process_custodian,
                    super::super::worktree::SourceRegistration {
                        repository_id: &repository, source_path: Path::new(&source),
                    }).map_err(|error| OrchestrationError::V37StoreFailure(format!("native worktree source: {error:?}")))?;
            }
            _ => return Err(OrchestrationError::Invalid("configuration command or fields")),
        }
        Ok(Json::Object(BTreeMap::from([
            (key("schema"), Json::String(JsonString::from_str("gogoke.37.owner-configuration.v1"))),
            (key("command"), Json::String(JsonString::from_str(&command))),
            (key("status"), Json::String(JsonString::from_str("APPLIED"))),
        ])).canonical().into_bytes())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::store::ledger;
    use crate::store::session_transport::codex_rpc;
    use crate::store::session_transport::decode_receipt;
    use crate::store::same_open::route_b_test_guard;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn request(operation: &str, request_id: &str, target_id: &str,
        expected: u64, payload: &str) -> V37Request {
        let frame = format!(
            "{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SEAT\",\"operation\":\"{operation}\",\"requestId\":\"{request_id}\",\"targetId\":\"{target_id}\",\"domainId\":\"projectA\",\"expectedRevision\":\"{expected}\",\"payload\":{payload}}}"
        );
        decode_request(frame.as_bytes()).unwrap()
    }

    fn status(product: &mut ProductDatabase<'_>, request: &V37Request) -> V37Status {
        decode_receipt(&product.dispatch_user_seat(request).unwrap()).unwrap().status
    }

    fn fixture(run: impl FnOnce(&mut ProductDatabase<'_>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-user-seat-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut product = ProductDatabase::open(&root, &database).unwrap();
        run(&mut product);
        product.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_file(path.join(".gogoke-state.sqlite.custody-v1")).unwrap();
        if let Err(error) = std::fs::remove_dir(&path) {
            eprintln!("owned fixture retained: {error}");
        }
    }

    #[test]
    fn secretary_configuration_ingress_preserves_unset_and_exact_owner_bytes() {
        fixture(|product| {
            let read=br#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-configuration-read"}"#;
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"state\":\"UNSET\""));
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"conversation\":{\"state\":\"NONE\"}"));
            seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),
                seat::StoreTemplate {domain_id:"global",template_id:"secretaryBase",settings_json:br#"{}"#}).unwrap();
            let created=seat::create(&mut product.connection,NativeOrigin::user(&product.owner),
                CreateSeat {domain_id:"global",seat_id:"globalSeatA",template_id:"secretaryBase",instance_id:None,
                    kind:Kind::Long,request_id:"createGlobalA",request_bytes:b"original global configuration seat"}).unwrap().seat;
            let designate=format!(r#"{{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-designate","requestId":"designateGlobalA","seatId":"globalSeatA","incarnation":"{}"}}"#,created.incarnation);
            assert!(String::from_utf8(product.configure_user_v37(designate.as_bytes()).unwrap()).unwrap()
                .contains("\"status\":\"APPLIED\""));
            assert!(String::from_utf8(product.configure_user_v37(designate.as_bytes()).unwrap()).unwrap()
                .contains("\"status\":\"REPLAYED\""));
            assert!(product.configure_user_v37(format!("{designate} ").as_bytes()).is_err(),
                "same logical request with different original bytes is not replay");
            let snapshot=String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap();
            for field in ["instanceId","model","effort","permissionTier"] {
                assert!(snapshot.contains(&format!("\"{field}\":null")),"no guessed setting: {snapshot}");
            }
            let configure=format!(r#"{{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-configure","requestId":"configureGlobalA","expectedGeneration":"{}","expectedRevision":"{}","instanceId":"absentInstance","model":"unverifiedModel","effort":"high","permissionTier":"READ_ONLY"}}"#,
                created.generation,created.revision);
            assert!(product.configure_user_v37(configure.as_bytes()).is_err(),"unverified F target refused");
            assert_eq!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap(),snapshot,
                "failed configuration leaves original unset fields and revisions unchanged");
            // Synthetic F metadata exercises this Owner configuration boundary;
            // it is not a real CLI/model-list or secretary launch observation.
            product.connection.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('configuredInstance','codex','fixtureHome','fixtureIdentity','sha256:fixture','fixture','INSTALLED','LOGGED_IN',1)").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,tombstoned,revision) VALUES('configuredInstance','fixture',1,0,1)").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_v37_instance_evidence(instance_id,available_models_json,models_source,models_observed_at,models_program_digest) VALUES('configuredInstance','[\"verifiedModel\"]','codex-model/list:OBSERVED:fixture','100','sha256:fixture')").unwrap();
            let configure=configure.replace("configureGlobalA","configureGlobalB")
                .replace("absentInstance","configuredInstance").replace("unverifiedModel","verifiedModel");
            assert!(String::from_utf8(product.configure_user_v37(configure.as_bytes()).unwrap()).unwrap()
                .contains("\"status\":\"APPLIED\""));
            assert!(String::from_utf8(product.configure_user_v37(configure.as_bytes()).unwrap()).unwrap()
                .contains("\"status\":\"REPLAYED\""));
            assert!(product.configure_user_v37(format!("{configure} ").as_bytes()).is_err(),
                "configuration replay requires original bytes at the Root ingress");
            let configured=String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap();
            for value in ["\"instanceId\":\"configuredInstance\"","\"model\":\"verifiedModel\"",
                "\"effort\":\"high\"","\"permissionTier\":\"READ_ONLY\""] {
                assert!(configured.contains(value),"Root readback matches original E selection: {configured}");
            }
            let current=seat::get(&product.connection,"global",&created.seat_id).unwrap().unwrap();
            assert!(current.generation>created.generation,"configuration advances E authorization generation");
            let history_read=format!(r#"{{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-configuration-read","inputHistory":{{"seatId":"{}","incarnation":"{}","authorizationGeneration":"{}","sessionId":"foreignSession","hGeneration":"1"}}}}"#,
                created.seat_id,created.incarnation,current.generation);
            assert!(String::from_utf8(product.configure_user_v37(history_read.as_bytes()).unwrap()).unwrap()
                .contains("\"inputHistory\":{\"items\":[],\"nextContinuation\":null,\"state\":\"UNKNOWN\"}"),
                "an unproven H/A conversation cannot disclose input history");
            assert!(product.configure_user_v37(history_read.replace(&created.seat_id,"otherSeat")
                .as_bytes()).is_err(),"the read is bound to the current E designation");
            // A foreign project cannot masquerade as the singleton's global
            // conversation. An old native selection alone has no H/A proof.
            product.connection.execute("INSERT INTO main.gogoke_v37_native_selection(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id) VALUES('projectA','foreignSession','globalSeatA','foreignIncarnation',1,'configuredInstance')").unwrap();
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"conversation\":{\"state\":\"NONE\"}"));
            let old=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_native_selection(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id) VALUES('global',?1,?2,?3,?4,'configuredInstance')").unwrap();
            old.bind_text(1,"oldSecretary").unwrap();
            old.bind_text(2,&created.seat_id).unwrap();
            old.bind_text(3,&created.incarnation).unwrap();
            old.bind_i64(4,created.generation).unwrap();
            old.step_done().unwrap();drop(old);
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"conversation\":{\"state\":\"UNKNOWN\"}"),
                "old selection without original H/A reader proof must not become FOUND or empty");
            // One incomplete current selection is unknown, even when old
            // history is retained. Two current selections are a conflict.
            let pending=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_native_selection(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id) VALUES('global',?1,?2,?3,?4,'configuredInstance')").unwrap();
            pending.bind_text(1,"pendingSecretaryA").unwrap();
            pending.bind_text(2,&created.seat_id).unwrap();
            pending.bind_text(3,&created.incarnation).unwrap();
            pending.bind_i64(4,current.generation).unwrap();
            pending.step_done().unwrap();drop(pending);
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"conversation\":{\"state\":\"UNKNOWN\"}"));
            let second=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_native_selection(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id) VALUES('global',?1,?2,?3,?4,'configuredInstance')").unwrap();
            second.bind_text(1,"pendingSecretaryB").unwrap();
            second.bind_text(2,&created.seat_id).unwrap();
            second.bind_text(3,&created.incarnation).unwrap();
            second.bind_i64(4,current.generation).unwrap();
            second.step_done().unwrap();drop(second);
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"conversation\":{\"state\":\"CONFLICT\"}"));
            product.connection.execute("DELETE FROM main.gogoke_v37_native_selection").unwrap();
            seat::reclaim(&mut product.connection,NativeOrigin::user(&product.owner),SeatChange {
                domain_id:"global",seat_id:&created.seat_id,expected_generation:current.generation,
                expected_revision:current.revision,request_id:"revokeGlobalA",request_bytes:b"original revoke"}).unwrap();
            assert!(String::from_utf8(product.configure_user_v37(read).unwrap()).unwrap()
                .contains("\"state\":\"REVOKED\""));
            assert!(product.configure_user_v37(designate.as_bytes()).is_err(),"old designation cannot undo revocation");
        });
    }

    #[test]
    fn secretary_original_user_pages_preserve_historical_provenance_and_gaps() {
        fixture(|product| {
            let hex=|bytes:&[u8]| bytes.iter().map(|byte|format!("{byte:02x}")).collect::<String>();
            // Qualify the public read with the same E designation, NATIVE_V2
            // H binding, A Secretary registration and original H/A thread
            // evidence that production requires. No model is launched here.
            product.connection.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceS','codex','fixtureHome','fixtureIdentity','sha256:fixture','0.160.0','INSTALLED','LOGGED_OUT',1)").unwrap();
            seat::store_template(&mut product.connection,NativeOrigin::user(&product.owner),
                seat::StoreTemplate {domain_id:"global",template_id:"secretaryHistory",
                    settings_json:br#"{}"#}).unwrap();
            let designated=seat::create(&mut product.connection,NativeOrigin::user(&product.owner),
                CreateSeat {domain_id:"global",seat_id:"seatS",template_id:"secretaryHistory",
                    instance_id:None,kind:Kind::Long,request_id:"createHistorySeat",
                    request_bytes:b"create history seat"}).unwrap().seat;
            seat::designate_secretary(&mut product.connection,&product.owner,"seatS",
                &designated.incarnation,"designateHistorySeat",b"designate history seat").unwrap();
            product.connection.execute("UPDATE main.gogoke_v37_seats SET state='BUSY',instance_id='instanceS' WHERE domain_id='global' AND seat_id='seatS'").unwrap();
            ledger::register_session(&mut product.connection,&ledger::SessionRegistration {
                domain_id:"global".into(),seat_id:"seatS".into(),
                session_id:"secretarySession".into(),purpose:ledger::SessionPurpose::Secretary,
                side_id:None,
            }).unwrap();
            let selection=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_native_selection VALUES('global','secretarySession','seatS',?1,?2,'instanceS')").unwrap();
            selection.bind_text(1,&designated.incarnation).unwrap();
            selection.bind_i64(2,designated.generation).unwrap();
            selection.step_done().unwrap();drop(selection);
            let binding=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_session_binding_v2 VALUES('global','secretarySession','seatS',?1,?2,'instanceS','NATIVE_V2')").unwrap();
            binding.bind_text(1,&designated.incarnation).unwrap();
            binding.bind_i64(2,designated.generation).unwrap();
            binding.step_done().unwrap();drop(binding);
            product.connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingS','instanceS','global','SESSION','secretarySession','2','ACTIVE')").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES('homeS','instanceS','global','SESSION','secretarySession','2','ACTIVE',1)").unwrap();
            product.connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('global','secretarySession','instanceS','homeS','bindingS','2','COMMITTED',7,'newProcess')").unwrap();
            let original_open=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"open1","targetId":"secretarySession","domainId":"global","expectedRevision":"0","payload":{}}"#;
            for (generation,operation,ticket,nonce,phase,custody,stop) in [
                ("1","oldProcess","oldTicket","oldNonce","STOPPED","STOPPED",Some("oldProof")),
                ("2","newProcess","newTicket","newNonce","ACTIVE","ACTIVE",None),
            ] {
                let episode=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,
                     generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,
                     instance_id,home_id,binding_id,phase,stop_fact_id)
                     VALUES('global',?1,'secretarySession',?2,?3,?4,?5,?6,?7,
                     'instanceS','homeS','bindingS',?8,?9)").unwrap();
                let open=format!("open{generation}");
                let old_generation=if generation=="1" {None} else {Some("1")};
                let raw=if generation=="1" {hex(original_open)} else {hex(br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"resume","requestId":"open2","targetId":"secretarySession","domainId":"global","expectedRevision":"1","payload":{"generation":"1"}}"#)};
                episode.bind_text(1,&open).unwrap();
                episode.bind_text(2,generation).unwrap();
                if let Some(old)=old_generation {episode.bind_text(3,old).unwrap();}
                episode.bind_text(4,&raw).unwrap();
                episode.bind_i64(5,if generation=="1" {0} else {2}).unwrap();
                episode.bind_i64(6,if generation=="1" {1} else {3}).unwrap();
                episode.bind_text(7,operation).unwrap();
                episode.bind_text(8,phase).unwrap();
                if let Some(stop)=stop {episode.bind_text(9,stop).unwrap();}
                episode.step_done().unwrap();drop(episode);
                let generation_row=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_generation VALUES('global','secretarySession',?1,?2,?3)").unwrap();
                for (index,value) in [generation,open.as_str(),operation].iter().enumerate() {
                    generation_row.bind_text((index+1) as i32,value).unwrap();
                }
                generation_row.step_done().unwrap();drop(generation_row);
                let process=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,
                     custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,
                     profile_id,domain_id,generation,state,stop_proof_hash)
                     VALUES(?1,?2,?3,'11','1','fixture','sha256:fixture','profileS','global',?4,?5,?6)").unwrap();
                for (index,value) in [operation,ticket,nonce,generation,custody].iter().enumerate() {
                    process.bind_text((index+1) as i32,value).unwrap();
                }
                if let Some(stop)=stop {process.bind_text(6,stop).unwrap();}
                process.step_done().unwrap();drop(process);
            }
            let command=codex_rpc::Command::ThreadStart {
                cwd:"fixture-directory".into(),model:"m".into(),
            }.encode(Some(&codex_rpc::RpcId::Number(3))).unwrap();
            let original_response=b"{\"id\":3,\"result\":{\"thread\":{\"id\":\"threadS\",\"cwd\":\"fixture-directory\"}}}\n";
            let source=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,
                 custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,
                 raw_bytes,state,no_event_reason) VALUES('oldProcess','oldTicket','oldNonce',
                 'global','secretarySession','1','oldEpoch','1',?1,'NO_EVENT','CODEX_RPC_RESPONSE')").unwrap();
            source.bind_blob(1,original_response).unwrap();
            source.step_done().unwrap();drop(source);
            let step=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,
                 step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,
                 image_path,binary_digest,profile_id,generation,command_hex,requires_response,
                 phase,source_epoch,source_cursor) VALUES('global','secretarySession','open1',
                 'thread-start','oldProcess','oldTicket','oldNonce','11','1','fixture',
                 'sha256:fixture','profileS','1',?1,1,'OBSERVED','oldEpoch','1')").unwrap();
            step.bind_text(1,&hex(&command)).unwrap();step.step_done().unwrap();drop(step);
            let long="a".repeat(900_000);
            for (id,generation,operation,ticket,nonce,revision,body,phase,marked) in [
                ("oldUser","1","oldProcess","oldTicket","oldNonce",1,&long,"RECEIPTED",true),
                ("newUser","2","newProcess","newTicket","newNonce",3,&long,"RECEIPTED",true),
                ("moreUser","2","newProcess","newTicket","newNonce",4,&long,"RECEIPTED",true),
                ("tailUser","2","newProcess","newTicket","newNonce",5,&long,"RECEIPTED",true),
                ("lastUser","2","newProcess","newTicket","newNonce",6,&long,"RECEIPTED",true),
                ("unmarkedUser","2","newProcess","newTicket","newNonce",7,&long,"UNKNOWN",false),
            ] {
                let original=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"send\",\"requestId\":\"{id}\",\"targetId\":\"secretarySession\",\"domainId\":\"global\",\"expectedRevision\":\"{revision}\",\"payload\":{{\"body\":\"{body}\",\"generation\":\"{generation}\"}}}}");
                assert!(original.len()<=1024*1024,"every admitted fixture request fits H's original frame bound");
                let request=decode_request(original.as_bytes()).unwrap();
                let original_hex=hex(original.as_bytes());
                let receipt=if phase=="RECEIPTED" {Some(encode_receipt(&request,
                    V37Status::Applied,revision,revision+1,BTreeMap::new()))} else {None};
                let row=Statement::prepare(product.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_stdin_journal(domain_id,request_id,operation,
                     ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,
                     phase,receipt_hex,receipt_status,expected_revision,
                     receipt_previous_revision,receipt_revision)
                     VALUES('global',?1,'send',?2,?3,?4,'secretarySession',?5,?6,?7,?8,?9,?10,?11,?12)").unwrap();
                for (index,value) in [id,ticket,operation,nonce,generation,
                    original_hex.as_str(),phase].iter().enumerate() {
                    row.bind_text((index+1) as i32,value).unwrap();
                }
                if let Some(receipt)=receipt {
                    row.bind_text(8,&hex(&receipt)).unwrap();
                    row.bind_text(9,"APPLIED").unwrap();
                    row.bind_text(11,&revision.to_string()).unwrap();
                    row.bind_text(12,&(revision+1).to_string()).unwrap();
                }
                row.bind_text(10,&revision.to_string()).unwrap();
                row.step_done().unwrap();drop(row);
                if marked {
                    let marker=Statement::prepare(product.connection.as_ptr(),
                        "INSERT INTO main.gogoke_v37_seat_secretary_presence
                         (source_id,kind,source_operation_id,source_epoch,source_cursor,
                          occurred_at_ms,observed_at_ms)
                         VALUES(?1,'INPUT',?2,?3,?4,100,100)").unwrap();
                    marker.bind_text(1,&format!("H-USER:global:secretarySession:{id}")).unwrap();
                    marker.bind_text(2,operation).unwrap();
                    marker.bind_text(3,nonce).unwrap();
                    marker.bind_text(4,id).unwrap();
                    marker.step_done().unwrap();
                }
            }
            let foreign=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"foreignUser","targetId":"secretarySession","domainId":"projectA","expectedRevision":"1","payload":{"body":"foreign project secret","generation":"1"}}"#;
            let foreign_row=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_stdin_journal(domain_id,request_id,operation,
                 ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,
                 phase,expected_revision) VALUES('projectA','foreignUser','send','oldTicket',
                 'oldProcess','oldNonce','secretarySession','1',?1,'PREPARED','1')").unwrap();
            foreign_row.bind_text(1,&hex(foreign)).unwrap();
            foreign_row.step_done().unwrap();drop(foreign_row);
            let changes=|product:&ProductDatabase<'_>| {
                let q=Statement::prepare(product.connection.as_ptr(),"SELECT CAST(total_changes() AS TEXT)").unwrap();
                assert!(q.step_row().unwrap());q.column_text(0).unwrap()
            };
            let before_reads=changes(product);
            let old_read=br#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-configuration-read"}"#;
            let original_configuration=product.configure_user_v37(old_read).unwrap();
            assert!(String::from_utf8_lossy(&original_configuration).contains("\"state\":\"FOUND\""),
                "the original E/H/A selection must qualify before testing its history");
            let frame=|incarnation:&str,authorization:i64,session:&str,cursor:Option<&str>| {
                let continuation=cursor.map(|cursor|format!(",\"continuation\":\"{cursor}\""))
                    .unwrap_or_default();
                format!(r#"{{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-configuration-read","inputHistory":{{"seatId":"seatS","incarnation":"{incarnation}","authorizationGeneration":"{authorization}","sessionId":"{session}","hGeneration":"2"{continuation}}}}}"#)
            };
            let parse=|bytes:&[u8]| {
                assert!(bytes.len()<=crate::ipc::MAX_FRAME_BYTES,"actual public response frame bound");
                let Json::Object(fields)=Parser::parse(std::str::from_utf8(bytes).unwrap()).unwrap()
                    else {panic!("configuration object");};
                fields
            };
            let first_bytes=product.configure_user_v37(frame(&designated.incarnation,
                designated.generation,"secretarySession",None).as_bytes()).unwrap();
            assert!(!String::from_utf8_lossy(&first_bytes).contains("foreignUser"));
            let mut result=parse(&first_bytes);
            assert_eq!(result.get(&key("state")).unwrap().canonical(),"\"DESIGNATED\"");
            let Json::Object(conversation)=result.get(&key("conversation")).unwrap() else {panic!("conversation");};
            assert_eq!(conversation.get(&key("state")).unwrap().canonical(),"\"FOUND\"");
            assert_eq!(conversation.get(&key("generation")).unwrap().canonical(),"\"2\"");
            assert_eq!(conversation.get(&key("historical")).unwrap().canonical(),"false");
            let Json::Object(first)=result.remove(&key("inputHistory")).unwrap() else {panic!("page");};
            let Json::Array(first_rows)=first.get(&key("items")).unwrap() else {panic!("items");};
            assert_eq!(first_rows.len(),4,"page stops before the next valid H body exceeds the IPC frame");
            let Json::Object(old)=&first_rows[0] else {panic!("old");};
            assert_eq!(old.get(&key("bodyState")).unwrap().canonical(),"\"VERIFIED\"");
            assert_eq!(old.get(&key("generation")).unwrap().canonical(),"\"1\"");
            assert_eq!(old.get(&key("receiptStatus")).unwrap().canonical(),"\"APPLIED\"");
            let Json::String(cursor)=first.get(&key("nextContinuation")).unwrap() else {panic!("cursor");};
            let cursor=cursor.to_well_formed_string().unwrap();
            assert!(product.configure_user_v37(frame(&designated.incarnation,
                designated.generation,"otherSession",Some(&cursor)).as_bytes()).is_err(),
                "continuation cannot switch the selected session");
            assert!(product.configure_user_v37(frame("otherIncarnation",
                designated.generation,"secretarySession",Some(&cursor)).as_bytes()).is_err(),
                "continuation cannot change the E incarnation");
            assert!(product.configure_user_v37(frame(&designated.incarnation,
                designated.generation+1,"secretarySession",Some(&cursor)).as_bytes()).is_err(),
                "continuation remains bound to the original E generation");
            let second_bytes=product.configure_user_v37(frame(&designated.incarnation,
                designated.generation,"secretarySession",Some(&cursor)).as_bytes()).unwrap();
            assert!(!String::from_utf8_lossy(&second_bytes).contains("foreignUser"));
            let mut result=parse(&second_bytes);
            let Json::Object(second)=result.remove(&key("inputHistory")).unwrap() else {panic!("second");};
            let Json::Array(second_rows)=second.get(&key("items")).unwrap() else {panic!("items");};
            assert_eq!(second_rows.len(),2);
            let Json::Object(last)=&second_rows[0] else {panic!("last original");};
            assert_eq!(last.get(&key("requestId")).unwrap().canonical(),"\"lastUser\"");
            assert_eq!(last.get(&key("bodyState")).unwrap().canonical(),"\"VERIFIED\"");
            let Json::Object(gap)=&second_rows[1] else {panic!("gap");};
            assert_eq!(gap.get(&key("requestId")).unwrap().canonical(),"\"unmarkedUser\"");
            assert_eq!(gap.get(&key("bodyState")).unwrap().canonical(),"\"UNKNOWN\"");
            assert_eq!(gap.get(&key("body")).unwrap().canonical(),"null");
            assert_eq!(gap.get(&key("phase")).unwrap().canonical(),"\"UNKNOWN\"");
            assert_eq!(second.get(&key("nextContinuation")).unwrap().canonical(),"null");
            assert_eq!(product.configure_user_v37(old_read).unwrap(),original_configuration,
                "scoped read never mutates the old two-field configuration response");
            assert_eq!(changes(product),before_reads,"USER history reads never write product facts");
            let invalid=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"send\",\"requestId\":\"invalidUser\",\"targetId\":\"secretarySession\",\"domainId\":\"global\",\"expectedRevision\":\"8\",\"payload\":{{\"body\":\"{}\",\"generation\":\"2\"}}}}",
                "x".repeat(1_100_000));
            assert!(invalid.len()>1024*1024 && invalid.len()<crate::ipc::MAX_FRAME_BYTES);
            let oversized_row=Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_stdin_journal(domain_id,request_id,operation,
                 ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,
                 phase,expected_revision) VALUES('global','invalidUser','send','newTicket',
                 'newProcess','newNonce','secretarySession','2',?1,'PREPARED','8')").unwrap();
            oversized_row.bind_text(1,&hex(invalid.as_bytes())).unwrap();
            oversized_row.step_done().unwrap();drop(oversized_row);
            let before_refusal=changes(product);
            let error=product.configure_user_v37(frame(&designated.incarnation,
                designated.generation,"secretarySession",Some(&cursor)).as_bytes()).unwrap_err();
            assert!(format!("{error:?}").contains("stored original operation size"),
                "H rejects an impossible original request before it can become history");
            assert_eq!(changes(product),before_refusal,"refused read has no product write");
            product.connection.execute("DELETE FROM main.gogoke_v37_h_stdin_journal WHERE domain_id='global' AND request_id='invalidUser'").unwrap();
            for (field,wrong,correct) in [
                ("source_operation_id","otherProcess","oldProcess"),
                ("source_epoch","otherNonce","oldNonce"),
                ("source_cursor","otherCursor","oldUser"),
            ] {
                let change=format!("UPDATE main.gogoke_v37_seat_secretary_presence SET {field}='{wrong}' WHERE source_id='H-USER:global:secretarySession:oldUser'");
                product.connection.execute(&change).unwrap();
                let bytes=product.configure_user_v37(frame(&designated.incarnation,
                    designated.generation,"secretarySession",None).as_bytes()).unwrap();
                let mut result=parse(&bytes);
                let Json::Object(page)=result.remove(&key("inputHistory")).unwrap() else {panic!("page");};
                let Json::Array(rows)=page.get(&key("items")).unwrap() else {panic!("items");};
                let Json::Object(old)=&rows[0] else {panic!("old row");};
                assert_eq!(old.get(&key("requestId")).unwrap().canonical(),"\"oldUser\"");
                assert_eq!(old.get(&key("bodyState")).unwrap().canonical(),"\"UNKNOWN\"");
                assert_eq!(old.get(&key("body")).unwrap().canonical(),"null");
                let restore=format!("UPDATE main.gogoke_v37_seat_secretary_presence SET {field}='{correct}' WHERE source_id='H-USER:global:secretarySession:oldUser'");
                product.connection.execute(&restore).unwrap();
            }
            product.connection.execute("UPDATE main.gogoke_v37_h_process_episode SET phase='STOPPED',stop_fact_id='newProof' WHERE domain_id='global' AND request_id='open2'").unwrap();
            product.connection.execute("UPDATE main.gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='newProof' WHERE operation_id='newProcess'").unwrap();
            product.connection.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED',stop_fact_id='newProof' WHERE domain_id='global' AND session_id='secretarySession'").unwrap();
            product.connection.execute("UPDATE main.gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='bindingS'").unwrap();
            product.connection.execute("UPDATE main.gogoke_v37_seats SET state='IDLE',generation=generation+1 WHERE domain_id='global' AND seat_id='seatS'").unwrap();
            let released=product.configure_user_v37(frame(&designated.incarnation,
                designated.generation+1,"secretarySession",None).as_bytes()).unwrap();
            let mut result=parse(&released);
            let Json::Object(conversation)=result.get(&key("conversation")).unwrap() else {panic!("conversation");};
            assert_eq!(conversation.get(&key("historical")).unwrap().canonical(),"true");
            assert_eq!(conversation.get(&key("claimState")).unwrap().canonical(),"\"RELEASED\"");
            let Json::Object(page)=result.remove(&key("inputHistory")).unwrap() else {panic!("page");};
            let Json::Array(rows)=page.get(&key("items")).unwrap() else {panic!("items");};
            let Json::Object(old)=&rows[0] else {panic!("old row");};
            assert_eq!(old.get(&key("bodyState")).unwrap().canonical(),"\"VERIFIED\"");
            assert_eq!(old.get(&key("generation")).unwrap().canonical(),"\"1\"");
        });
    }

    #[test]
    fn owner_configuration_and_user_seat_share_the_verified_product_store() {
        fixture(|product| {
            assert!(is_user_v37_configuration_frame(br#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":2}"#));
            assert!(!is_user_v37_configuration_frame(br#"{"schema":"gogoke.37.operations.v1","family":"K-SEAT"}"#));
            let config = |product: &mut ProductDatabase<'_>, frame: &str| {
                product.configure_user_v37(frame.as_bytes()).unwrap()
            };
            config(product, r#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":2}"#);
            assert_eq!(seat::read_project_parallel_cap(&product.connection, "projectA").unwrap(), 2);
            assert!(product.configure_user_v37(br#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":0}"#).is_err());
            assert!(product.configure_user_v37(br#"{"schema":"gogoke.37.owner-configuration.v1","command":"project-parallel-cap","domainId":"projectA","value":2,"sql":"DROP TABLE"}"#).is_err());
            let template=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"seat-template","domainId":"projectA","requestId":"templateAConfig","templateId":"templateA","settings":{"instruction":"default"}}"#;
            config(product, template);
            assert!(String::from_utf8(config(product,template)).unwrap().contains("\"REPLAYED\""));
            assert!(product.configure_user_v37(template.replace("default","changed").as_bytes()).is_err());
            let policy=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"policy-initialize","domainId":"projectA","requestId":"policyInit","stage":"OPEN","expectedRevision":"0"}"#;
            config(product,policy);
            assert!(String::from_utf8(config(product,policy)).unwrap().contains("\"REPLAYED\""));
            assert!(product.configure_user_v37(policy.replace("OPEN","DONE").as_bytes()).is_err());
            let insert = Statement::prepare(product.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();
            insert.step_done().unwrap();
            config(product, r#"{"schema":"gogoke.37.owner-configuration.v1","command":"instance-concurrency-cap","instanceId":"instanceA","value":2}"#);
            assert_eq!(instance::read_instance_concurrency_cap(&product.connection, "instanceA").unwrap(), 2);
            let create = request("create-from-template", "createA", "seatA", 0,
                r#"{"layer":"USER","templateId":"templateA"}"#);
            assert_eq!(status(product, &create), V37Status::Applied);
            assert_eq!(status(product, &create), V37Status::Replayed);
            let grant=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"policy-call-grant","domainId":"projectA","requestId":"grantA","callerSeatId":"seatA","targetId":"OWNER","action":"MESSAGE","expiresAtMs":null,"expectedRevision":"1"}"#;
            assert!(String::from_utf8(config(product,grant)).unwrap().contains("\"revision\":\"2\""));
            assert!(String::from_utf8(config(product,grant)).unwrap().contains("\"REPLAYED\""));
            assert!(product.configure_user_v37(grant.replace("grantA","grantB").as_bytes()).is_err());
            let card = request("state-card", "cardA", "seatA", 1, "{}");
            let card_receipt = product.dispatch_user_seat(&card).unwrap();
            assert_eq!(decode_receipt(&card_receipt).unwrap().status, V37Status::Applied);
            let card_text = std::str::from_utf8(&card_receipt).unwrap();
            assert!(card_text.contains("\"takeoverQuestionsConfigured\":false"));
            assert!(card_text.contains("\"instruction\":\"default\""));
            assert!(card_text.contains("\"takeoverReady\":false"));
            assert_eq!(status(product,&card),V37Status::Replayed);
            assert_eq!(status(product,&request("state-card","cardA","seatA",1,
                r#"{"forged":true}"#)),V37Status::Conflict);
            assert_eq!(status(product, &request("tune", "tuneA", "seatA", 1,
                r#"{"setting":"instruction","value":"changed"}"#)), V37Status::Applied);
            assert_eq!(status(product, &card), V37Status::Stale);
            assert_eq!(status(product, &request("bind-instance", "bindA", "seatA", 2,
                r#"{"instanceId":"instanceA"}"#)), V37Status::Applied);
            assert_eq!(status(product, &request("change-instance", "changeBusy", "seatA", 1,
                r#"{"instanceId":"instanceA","model":"modelA","effort":"high","permissionTier":"READ_ONLY"}"#)), V37Status::Stale);
            assert_eq!(status(product, &request("change-instance", "incompleteChange", "seatA", 3,
                r#"{"instanceId":"instanceA"}"#)), V37Status::Denied);
            assert_eq!(status(product, &request("reclaim", "reclaimA", "seatA", 3, "{}")), V37Status::Applied);
            assert_eq!(status(product, &request("short-to-long", "latePromote", "seatA", 4, "{}")), V37Status::Conflict);
            assert_eq!(status(product, &request("takeover-answers", "takeoverA", "seatA", 4, "{}")), V37Status::Unsupported);
        });
    }

    #[test]
    fn secretary_routine_user_commands_read_original_rows_and_preserve_history() {
        fixture(|product| {
            let list=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-routines-read"}"#;
            assert!(product.configure_user_v37(list.as_bytes()).is_err(),
                "an unset Secretary is not an empty routine list");
            seat::store_template(&mut product.connection, NativeOrigin::user(&product.owner),
                seat::StoreTemplate {domain_id:"global",template_id:"routineBase",settings_json:br#"{}"#}).unwrap();
            let created=seat::create(&mut product.connection, NativeOrigin::user(&product.owner),
                CreateSeat {domain_id:"global",seat_id:"routineSecretary",template_id:"routineBase",
                    instance_id:None,kind:Kind::Long,request_id:"createRoutineSeat",
                    request_bytes:b"create routine seat"}).unwrap().seat;
            seat::designate_secretary(&mut product.connection,&product.owner,&created.seat_id,
                &created.incarnation,"designateRoutine",b"designate routine").unwrap();
            product.connection.execute("BEGIN").unwrap();
            let (initial_presence,initial_policy)=seat::read_secretary_presence_in_transaction(
                &product.connection,&product.owner).unwrap();
            product.connection.execute("COMMIT").unwrap();
            assert!(initial_presence.is_none(),"designation does not invent Owner presence");
            assert_eq!(initial_policy,Some(seat::SecretaryAbsencePolicyFact {
                revision:1,max_absent_ms:86_400_000,
                source_id:"PRODUCT_DEFAULT_V1:ABSENCE_24_HOURS".into(),
            }));
            // Storage fixture only: production create requires an independently
            // authenticated User input locator and schedule parser.
            seat::create_secretary_routine(&mut product.connection,&product.owner,
                seat::SecretaryRoutineCreate {routine_id:"routineA",request_id:"createRoutine",
                    request_bytes:b"fixture create",original_text:"明天提醒我",source_operation_id:"inputA",
                    source_epoch:"epochA",source_cursor:"1",schedule_raw:"明天",timezone:"Asia/Shanghai",
                    next_due_ms:100,now_ms:50}).unwrap();
            let call=|product:&mut ProductDatabase<'_>,frame:&str| {
                String::from_utf8(product.configure_user_v37(frame.as_bytes()).unwrap()).unwrap()
            };
            let original=call(product,list);
            assert!(original.contains("\"originalText\":\"明天提醒我\""));
            assert!(original.contains("\"sourceOperationId\":\"inputA\""));
            assert!(original.contains("\"lastResult\":\"NONE\""));
            let missing=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-routines-read","routineId":"missing"}"#;
            assert!(product.configure_user_v37(missing.as_bytes()).is_err());
            let pause=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-routine-pause","routineId":"routineA","requestId":"pauseA","expectedRevision":"1"}"#;
            assert!(call(product,pause).contains("\"status\":\"APPLIED\""));
            assert!(call(product,pause).contains("\"status\":\"REPLAYED\""));
            assert!(call(product,list).contains("\"state\":\"PAUSED\""));
            let future=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()+86_400_000;
            let resume=format!(r#"{{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-routine-resume","routineId":"routineA","requestId":"resumeA","expectedRevision":"2","nextDueMs":"{future}"}}"#);
            assert!(call(product,&resume).contains("\"state\":\"ACTIVE\""));
            let delete=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-routine-delete","routineId":"routineA","requestId":"deleteA","expectedRevision":"3"}"#;
            assert!(call(product,delete).contains("\"state\":\"DELETED\""));
            let history=r#"{"schema":"gogoke.37.owner-configuration.v1","command":"secretary-routines-read","routineId":"routineA"}"#;
            let final_row=call(product,history);
            assert!(final_row.contains("\"state\":\"DELETED\""));
            assert!(final_row.contains("\"originalText\":\"明天提醒我\""));
            assert!(final_row.contains("\"occurrences\":[]"));
            product.connection.execute("BEGIN").unwrap();
            let (presence,policy)=seat::read_secretary_presence_in_transaction(
                &product.connection,&product.owner).unwrap();
            product.connection.execute("COMMIT").unwrap();
            assert_eq!(presence,initial_presence,"reads and changes never invent presence");
            assert_eq!(policy,initial_policy,"reads and changes preserve the designated product policy");
        });
    }
}
