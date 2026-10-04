//! C M1 native Inbox. Only the User ingress may call this dispatcher.
//! Durable C intent grants one H command; H's OBSERVED response and A's
//! original frame are checked by C before a DELIVERED receipt exists.
use super::*;
use crate::store::inbox::{self, InboxEdit, InboxEnvelope, InboxError, Message,
    NativeDeliveryKind, StoredOperation};
use crate::store::seat::NativeOrigin;
use crate::store::session_transport::{codex_rpc::{self,Command,Reply,RpcId,RpcError}, runtime};
use crate::store::digest::sha256_hex;

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}

fn inbox_error(scope:&str,error:impl std::fmt::Debug)->OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("native inbox {scope}: {error:?}"))
}

fn status(error:&InboxError)->V37Status {
    match error {
        InboxError::Invalid(_) | InboxError::Denied | InboxError::Authority(_) => V37Status::Denied,
        InboxError::Stale=>V37Status::Stale,
        InboxError::Conflict=>V37Status::Conflict,
        InboxError::Unknown | InboxError::CommitUnknown(_) | InboxError::RollbackUnknown(_)=>V37Status::Unknown,
        _=>V37Status::Unknown,
    }
}

fn outcome(request:&V37Request,operation:&StoredOperation,replayed:bool)->Vec<u8> {
    let status=match operation.phase.as_str() {
        "APPLIED"=>if replayed {V37Status::Replayed} else {V37Status::Applied},
        "FAILED"=>V37Status::Failed,
        "UNKNOWN" | "PREPARED"=>V37Status::Unknown,
        "DENIED"=>V37Status::Denied,
        "CONFLICT"=>V37Status::Conflict,
        _=>V37Status::Unknown,
    };
    let mut result=BTreeMap::from([(JsonString::from_str("state"),text(&operation.result_state))]);
    if !operation.reason.is_empty() {
        result.insert(JsonString::from_str("reason"),text(&operation.reason));
        if matches!(operation.phase.as_str(),"FAILED"|"UNKNOWN") {
            result.insert(JsonString::from_str("error"),text(&operation.reason));
        }
    }
    if !operation.native_receipt_id.is_empty() {
        result.insert(JsonString::from_str("nativeReceiptId"),text(&operation.native_receipt_id));
    }
    if request.operation=="steer" && operation.result_state=="DELIVERED" {
        result.insert(JsonString::from_str("mode"),text("NATIVE"));
    }
    if request.operation=="requeue" && operation.phase=="APPLIED" {
        if let Some(Json::String(new_id))=request.payload.get(&JsonString::from_str("newMessageId")) {
            result.insert(JsonString::from_str("newMessageId"),Json::String(new_id.clone()));
            result.insert(JsonString::from_str("newState"),text("PENDING"));
            result.insert(JsonString::from_str("newRevision"),text("1"));
        }
    }
    encode_receipt(request,status,operation.previous_revision,operation.revision,result)
}

fn message_result(message:Option<&Message>)->BTreeMap<JsonString,Json> {
    let mut result=BTreeMap::from([(JsonString::from_str("state"),
        text(message.map_or("ABSENT",|row|&row.state)))]);
    result.insert(JsonString::from_str("requeuedAs"),message.and_then(|row|row.requeued_as.as_deref())
        .map_or(Json::Null,text));
    result
}

fn payload(request:&V37Request,field:&'static str)->Result<String> {
    user_payload_string(request,field)
}

fn exact_fields(request:&V37Request,expected:&[&str])->bool {
    request.payload.len()==expected.len() && expected.iter()
        .all(|field|request.payload.contains_key(&JsonString::from_str(field)))
}

struct Target {
    key:(String,String), seat:String, generation:String, turn:String, thread:String,
    operation:String, ticket:String, nonce:String, turn_is_current:bool,
}

fn observed_turn(db:&VerifiedDatabaseConnection<'_>,target:&Target)->Result<bool> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes) FROM main.gogoke_v37_rpc_steps s
         JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
           AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
           AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
           AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
         WHERE s.domain_id=?1 AND s.session_id=?2 AND s.generation=?3
           AND s.process_operation_id=?4 AND s.ticket=?5 AND s.custodian_nonce=?6
           AND s.phase='OBSERVED' AND s.requires_response=1
           AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
    for (index,value) in [target.key.0.as_str(),target.key.1.as_str(),
        target.generation.as_str(),target.operation.as_str(),target.ticket.as_str(),
        target.nonce.as_str()].iter().enumerate(){q.bind_text((index+1) as i32,value)?;}
    let mut found=0usize;
    while q.step_row()? {
        let command=unhex(&q.column_text(0)?)?;
        let response=unhex(&q.column_text(1)?)?;
        let Ok((id,command))=codex_rpc::decode_stored_turn_start(&command) else {continue;};
        let Command::TurnStart {thread_id,..}=&command else {continue;};
        if thread_id!=&target.thread {continue;}
        if let Ok(Reply::Turn {turn_id,..})=codex_rpc::decode(&response,Some((&id,&command))) {
            if turn_id==target.turn {found+=1;}
        }
    }
    if found>1 {return Err(OrchestrationError::OperationConflict);}
    Ok(found==1)
}

fn unhex(value:&str)->Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(OrchestrationError::OperationConflict);}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let digit=|byte:u8|match byte {b'0'..=b'9'=>Some(byte-b'0'),b'A'..=b'F'=>Some(byte-b'A'+10),
            b'a'..=b'f'=>Some(byte-b'a'+10),_=>None};
        let high=digit(pair[0]).ok_or(OrchestrationError::OperationConflict)?;
        let low=digit(pair[1]).ok_or(OrchestrationError::OperationConflict)?;
        Ok((high<<4)|low)
    }).collect()
}

/// Use the largest Codex-supported numeric RPC ID, so every actual ID H may
/// allocate has an encoding no larger than this native command. The codec's
/// own 1 MiB frame rule is the only size rule here.
fn command_encodes(kind:NativeDeliveryKind,thread:&str,turn:&str,body:&str)->Result<bool> {
    let id=RpcId::client(9_007_199_254_740_991)
        .map_err(|error|inbox_error("Codex maximum RPC ID",error))?;
    let command=match kind {
        NativeDeliveryKind::Deliver=>Command::AppendWithoutTurn {thread_id:thread.into(),text:body.into()},
        NativeDeliveryKind::Steer=>Command::TurnSteer {thread_id:thread.into(),
            expected_turn_id:turn.into(),text:body.into()},
    };
    match command.encode(Some(&id)) {
        Ok(_)=>Ok(true),
        Err(RpcError::FrameTooLarge | RpcError::Invalid(_))=>Ok(false),
        Err(error)=>Err(inbox_error("native command preflight",error)),
    }
}

/// Resolve a physical recipient under the current Owner/E/F/H connection.
/// A wire seat or generation never identifies a process on its own.
fn live_target(db:&mut ProductDatabase<'_>,domain:&str,seat:&str,generation:&str,
    turn:&str)->Result<Target> {
    let matching:Vec<_>=db.native_sessions.iter().filter(|(key,run)|
        key.0==domain && run.evidence.seat_id()==seat
            && run.custody.binding.generation==generation)
        .map(|(key,_)|key.clone()).collect();
    if matching.len()!=1 {return Err(OrchestrationError::AccessDenied);}
    let key=matching.into_iter().next().ok_or(OrchestrationError::AccessDenied)?;
    db.drain_native_output(&key)?;
    let run=db.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
    if !run.allows_input() {return Err(OrchestrationError::AccessDenied);}
    let claim=runtime::observe_claim(&db.connection,&NativeOrigin::user(&db.owner),
        domain,seat,&key.1).map_err(|error|inbox_error("H claim",error))?
        .ok_or(OrchestrationError::AccessDenied)?;
    run.evidence.verify_live(&mut db.connection,db.root,&db.owner,
        &run.operation_id,claim.revision).map_err(OrchestrationError::V37StoreFailure)?;
    if db.process_custodian.active(&run.custody.ticket).is_none() {
        return Err(OrchestrationError::AccessDenied);
    }
    let target=Target {key,seat:seat.into(),generation:generation.into(),turn:turn.into(),
        thread:run.thread_id.clone().ok_or(OrchestrationError::AccessDenied)?,
        operation:run.operation_id.clone(),ticket:run.custody.ticket.opaque().into(),
        nonce:run.custody.custodian_nonce.clone(),
        turn_is_current:run.turn_id.as_deref()==Some(turn)};
    if !target.turn_is_current && !observed_turn(&db.connection,&target)? {
        return Err(OrchestrationError::AccessDenied);
    }
    Ok(target)
}

fn target_present(db:&VerifiedDatabaseConnection<'_>,target:&Target)
    ->std::result::Result<bool,InboxError> {
    if crate::store::session_transport::generation_change::active_for_session(db,&target.key.0,&target.key.1)?.is_some() {return Ok(false);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_claim h
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
           AND h.state='COMMITTED' AND c.state='ACTIVE'")?;
    for (index,value) in [target.key.0.as_str(),target.key.1.as_str(),target.seat.as_str(),
        target.generation.as_str(),target.operation.as_str(),target.ticket.as_str(),
        target.nonce.as_str()].iter().enumerate(){q.bind_text((index+1) as i32,value)?;}
    Ok(q.step_row()? && !q.step_row()?)
}

impl<'root> ProductDatabase<'root> {
    fn inbox_receipt_error(&mut self,request:&V37Request,error:InboxError)->Result<Vec<u8>> {
        let mapped=status(&error);
        if matches!(mapped,V37Status::Unknown) {
            return Err(inbox_error(&request.operation,error));
        }
        let revision=if mapped==V37Status::Stale {
            let owner=&self.owner;
            inbox::current_revision(&mut self.connection,&request.domain_id,&request.target_id,|db| {
                authority::check_owner_in_current_transaction(db,owner).map_err(InboxError::Authority)?;
                Ok(true)
            }).map_err(|error|inbox_error("current stale revision",error))?
        } else {request.expected_revision};
        Ok(encode_receipt(request,mapped,revision,revision,
            Default::default()))
    }

    pub(super) fn dispatch_native_inbox(&mut self,request:&V37Request)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        // C mutations and A read receipts share one request identity per
        // family/domain. A changed target or any changed original bytes collide.
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_inbox_operations
               WHERE domain_id=?1 AND request_id=?2
             UNION ALL SELECT lower(hex(request_bytes)) FROM main.v37_ledger_receipt
               WHERE family='K-INBOX' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        let exact=hex_bytes(&request.raw_bytes);
        while prior.step_row()? {
            if prior.column_text(0)?!=exact {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,Default::default()));
            }
        }
        drop(prior);
        let envelope=InboxEnvelope {domain_id:&request.domain_id,request_id:&request.request_id,
            request_bytes:&request.raw_bytes,message_id:&request.target_id,
            expected_revision:request.expected_revision};
        match request.operation.as_str() {
            "enqueue" | "edit" | "cancel" | "requeue"=>self.mutate_native_inbox(request,&envelope),
            "check-unknown"=>self.check_native_inbox(request),
            "deliver" | "steer"=>self.deliver_native_inbox(request,&envelope),
            _=>Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,Default::default())),
        }
    }

    fn mutate_native_inbox(&mut self,request:&V37Request,envelope:&InboxEnvelope<'_>)->Result<Vec<u8>> {
        // A committed write remains historical after its target turn ends.
        // Its original exact bytes and current User issuer suffice for replay;
        // no new target eligibility or physical write is granted here.
        if let Some(prior)=read_inbox_operation(&self.connection,&request.domain_id,&request.request_id)? {
            if prior.request_hex!=hex_bytes(&request.raw_bytes) || prior.message_id!=request.target_id {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,Default::default()));
            }
            let owner=&self.owner;
            let result=inbox::edit_message(&mut self.connection,envelope,InboxEdit::Cancel,|db| {
                authority::check_owner_in_current_transaction(db,owner).map_err(InboxError::Authority)?;
                Ok(true)
            });
            return match result {Ok(operation)=>Ok(outcome(request,&operation,true)),
                Err(error)=>self.inbox_receipt_error(request,error)};
        }
        let mut target=None;
        let body;
        let new_id;
        let edit=match request.operation.as_str() {
            "enqueue" if exact_fields(request,&["seatId","turnId","generation","body"])=>{
                let seat=payload(request,"seatId")?;let turn=payload(request,"turnId")?;
                let generation=payload(request,"generation")?;
                body=payload(request,"body")?;
                target=Some(match live_target(self,&request.domain_id,&seat,&generation,&turn) {
                    Ok(value)=>value,
                    Err(OrchestrationError::AccessDenied)=>return Ok(encode_receipt(request,
                        V37Status::Denied,request.expected_revision,request.expected_revision,
                        Default::default())),
                    Err(error)=>return Err(error),
                });
                if !command_encodes(NativeDeliveryKind::Deliver,&target.as_ref().unwrap().thread,
                    &turn,&body)? {
                    return Ok(encode_receipt(request,V37Status::Denied,0,0,Default::default()));
                }
                InboxEdit::Enqueue {sender_seat_id:"User",seat_id:target.as_ref().unwrap().seat.as_str(),
                    turn_id:target.as_ref().unwrap().turn.as_str(),generation:target.as_ref().unwrap().generation.as_str(),body:&body}
            }
            "edit" if exact_fields(request,&["body"])=>{
                body=payload(request,"body")?;
                let Some(original)=original_target(&self.connection,&request.domain_id,
                    &request.target_id,None)? else {return Ok(encode_receipt(request,
                        V37Status::Denied,request.expected_revision,request.expected_revision,
                        Default::default()));};
                if !command_encodes(NativeDeliveryKind::Deliver,&original.thread,
                    &original.turn,&body)? {
                    return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                        request.expected_revision,Default::default()));
                }
                InboxEdit::Edit {body:&body}
            }
            "cancel" if request.payload.is_empty()=>InboxEdit::Cancel,
            "requeue" if exact_fields(request,&["newMessageId","seatId","turnId","generation"])=>{
                new_id=payload(request,"newMessageId")?;
                let seat=payload(request,"seatId")?;let turn=payload(request,"turnId")?;
                let generation=payload(request,"generation")?;
                target=Some(match live_target(self,&request.domain_id,&seat,&generation,&turn) {
                    Ok(value)=>value,
                    Err(OrchestrationError::AccessDenied)=>return Ok(encode_receipt(request,
                        V37Status::Denied,request.expected_revision,request.expected_revision,
                        BTreeMap::from([(JsonString::from_str("reason"),text("TARGET_NOT_ELIGIBLE"))]))),
                    Err(error)=>return Err(error),
                });
                let Some(old)=read_inbox_message(&self.connection,&request.domain_id,&request.target_id)?
                    else {return Ok(encode_receipt(request,V37Status::Conflict,0,0,Default::default()));};
                if !command_encodes(NativeDeliveryKind::Deliver,&target.as_ref().unwrap().thread,
                    &turn,&old.body)? {
                    return Ok(encode_receipt(request,V37Status::Denied,old.revision,
                        old.revision,Default::default()));
                }
                InboxEdit::Requeue {new_message_id:&new_id,seat_id:target.as_ref().unwrap().seat.as_str(),
                    turn_id:target.as_ref().unwrap().turn.as_str(),generation:target.as_ref().unwrap().generation.as_str()}
            }
            _=>return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default())),
        };
        let owner=&self.owner;
        let result=inbox::edit_message(&mut self.connection,envelope,edit,|db| {
            authority::check_owner_in_current_transaction(db,owner).map_err(InboxError::Authority)?;
            match target.as_ref() {Some(target)=>target_present(db,target),None=>Ok(true)}
        });
        match result {
            Ok(operation)=>Ok(outcome(request,&operation,false)),
            Err(error)=>self.inbox_receipt_error(request,error),
        }
    }

    /// User-only C projection of already persisted OWNER notices. No model
    /// recipient, receipt append, new send, ACK or read-status write occurs.
    fn read_owner_host_notices(&mut self,request:&V37Request)->Result<Vec<u8>> {
        if request.domain_id!="global" || request.target_id!="OWNER" || request.expected_revision!=0 {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(||->Result<Vec<u8>> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let query=Statement::prepare(self.connection.as_ptr(),
                "SELECT domain_id,gate_id FROM main.gogoke_v37_seat_policy_gates
                  WHERE state='ESCALATION_REQUIRED' ORDER BY domain_id,gate_id")?;
            let mut gates=Vec::new();
            while query.step_row()? {gates.push((query.column_text(0)?,query.column_text(1)?));}
            drop(query);
            let mut notices=Vec::new();
            for (domain,gate) in gates {
                let proof=match crate::store::seat::observe_host_reject_cap_in_transaction(
                    &self.connection,&self.owner,&domain,&gate) {
                    Ok(Some(proof))=>proof,
                    Ok(None)|Err(crate::store::seat::SeatError::Denied|crate::store::seat::SeatError::Conflict)=>continue,
                    Err(error)=>return Err(inbox_error("OWNER original E cause",error)),
                };
                if proof.destination_seat_id()!="OWNER" {continue;}
                // Route changes/cancellation suppress presentation of an old
                // cause; they cannot allocate a replacement notification.
                match crate::store::seat::read_host_escalation_intent_in_transaction(
                    &self.connection,&self.owner,&proof) {
                    Ok(Some(_))=>{},
                    Ok(None)=>continue,
                    Err(crate::store::seat::SeatError::Denied|crate::store::seat::SeatError::Conflict)=>continue,
                    Err(error)=>return Err(inbox_error("OWNER current E route",error)),
                }
                let Some(message)=inbox::host_rule::read_owner_host_notice_in_transaction(
                    &self.connection,&self.owner,&proof).map_err(|error|
                        inbox_error("OWNER original C notice",error))? else {continue;};
                notices.push(Json::Object(BTreeMap::from([
                    (JsonString::from_str("domainId"),text(&domain)),
                    (JsonString::from_str("messageId"),text(&message.message_id)),
                    (JsonString::from_str("revision"),text(&message.revision.to_string())),
                    (JsonString::from_str("state"),text(&message.state)),
                    (JsonString::from_str("sourceSeatId"),text(proof.source_seat_id())),
                    (JsonString::from_str("causeEventId"),text(proof.cause_event_id())),
                    (JsonString::from_str("triggerId"),text(proof.trigger_id())),
                    (JsonString::from_str("body"),text(&message.body)),
                ])));
            }
            Ok(encode_receipt(request,V37Status::Applied,0,0,BTreeMap::from([
                (JsonString::from_str("projection"),text("OWNER_HOST_RULE_NOTICES")),
                (JsonString::from_str("notices"),Json::Array(notices)),
            ])))
        })();
        match read {
            Ok(bytes)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(inbox_error("OWNER projection rollback",format!("{primary:?}; {rollback:?}")));
                }
                Err(primary)
            }
        }
    }

    fn check_native_inbox(&mut self,request:&V37Request)->Result<Vec<u8>> {
        if exact_fields(request,&["projection"])
            && payload(request,"projection")?=="OWNER_HOST_RULE_NOTICES" {
            return self.read_owner_host_notices(request);
        }
        if !request.payload.is_empty() {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(||->Result<Vec<u8>> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let mut message=read_inbox_message(&self.connection,&request.domain_id,&request.target_id)?;
            let revision=message.as_ref().map_or(0,|message|message.revision);
            if let Some(message)=message.as_mut() {
                if message.state=="PREPARED" {message.state="UNKNOWN".into();}
            }
            let result=message_result(message.as_ref());
            let prior=Statement::prepare(self.connection.as_ptr(),
                "SELECT request_bytes,receipt_bytes FROM main.v37_ledger_receipt
                 WHERE family='K-INBOX' AND domain_id=?1 AND request_id=?2")?;
            prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
            if prior.step_row()? {
                let original=prior.column_text(0)?;
                let receipt=prior.column_text(1)?;
                if original.as_bytes()!=request.raw_bytes || prior.step_row()? {
                    return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,
                        Default::default()));
                }
                drop(prior);
                let receipt=crate::store::session_transport::decode_receipt(receipt.as_bytes())
                    .map_err(|error|inbox_error("A original read receipt",error))?;
                if receipt.family!="K-INBOX" || receipt.operation!="check-unknown"
                    || receipt.request_id!=request.request_id || receipt.target_id!=request.target_id {
                    return Err(OrchestrationError::OperationConflict);
                }
                let old_revision=receipt.revision;
                let old=receipt.into_result();
                let same=old_revision==revision && old.len()==result.len() && old.iter().all(|(key,value)|
                    result.get(key).is_some_and(|current|current.canonical()==value.canonical()));
                return Ok(encode_receipt(request,if same {V37Status::Replayed} else {V37Status::Stale},
                    revision,revision,if same {old} else {Default::default()}));
            }
            drop(prior);
            if revision!=request.expected_revision {
                return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));
            }
            let bytes=encode_receipt(request,V37Status::Applied,revision,revision,result);
            if bytes.len()>crate::ipc::MAX_FRAME_BYTES {
                return Err(OrchestrationError::Invalid("native inbox read receipt frame"));
            }
            let insert=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes)
                 VALUES('K-INBOX',?1,?2,?3,?4)")?;
            insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
            insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(bytes)
        })();
        match read {
            Ok(bytes)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(inbox_error("A read receipt rollback",format!("{primary:?}; {rollback:?}")));
                }
                Err(primary)
            }
        }
    }

    fn deliver_native_inbox(&mut self,request:&V37Request,envelope:&InboxEnvelope<'_>)->Result<Vec<u8>> {
        let kind=if request.operation=="steer" {NativeDeliveryKind::Steer} else {NativeDeliveryKind::Deliver};
        let fields=if kind==NativeDeliveryKind::Steer {&["turnId","generation"][..]}
            else {&["generation"][..]};
        if !exact_fields(request,fields) {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let generation=payload(request,"generation")?;
        let prior=read_inbox_operation(&self.connection,&request.domain_id,&request.request_id)?;
        if let Some(ref prior)=prior {
            if prior.request_hex!=hex_bytes(&request.raw_bytes) || prior.message_id!=request.target_id {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,Default::default()));
            }
            if matches!(prior.phase.as_str(),"UNKNOWN"|"PREPARED") {
                let original=original_target(&self.connection,&request.domain_id,&request.target_id,
                    Some(&request.request_id))?;
                if let Some(original)=original {
                    let step_id=inbox_step(&request.domain_id,&original.session,&request.request_id);
                    if kind==NativeDeliveryKind::Steer {
                        if let Some(aborted)=inbox::abort_native_steer_if_ended(&mut self.connection,
                            &self.owner,envelope,&original.session,&step_id,&original.generation,
                            &original.operation,&original.ticket,&original.nonce,&original.thread,&original.turn)
                            .map_err(|error|inbox_error("replayed H abort proof",error))? {
                            return Ok(outcome(request,&aborted,true));
                        }
                    }
                    if prior.phase=="UNKNOWN" {
                        match inbox::settle_native_delivery_observed(&mut self.connection,&self.owner,
                            envelope,&original.session,&step_id,kind) {
                            Ok(settled)=>return Ok(outcome(request,&settled,true)),
                            Err(InboxError::Denied)=>{},
                            Err(error)=>return self.inbox_receipt_error(request,error),
                        }
                    }
                }
            }
            return Ok(outcome(request,prior,true));
        }
        let Some(message)=read_inbox_message(&self.connection,&request.domain_id,&request.target_id)? else {
            return Ok(encode_receipt(request,V37Status::Conflict,0,0,Default::default()));
        };
        if message.revision!=request.expected_revision {
            return Ok(encode_receipt(request,V37Status::Stale,message.revision,message.revision,Default::default()));
        }
        if message.generation!=generation || (kind==NativeDeliveryKind::Steer
            && payload(request,"turnId")?!=message.turn_id) {
            return Ok(encode_receipt(request,V37Status::Conflict,message.revision,message.revision,Default::default()));
        }
        if let Some(original)=original_target(&self.connection,&request.domain_id,&request.target_id,None)? {
            if crate::store::session_transport::generation_change::active_for_session(&self.connection,
                &request.domain_id,&original.session)?.is_some() {
                return Ok(encode_receipt(request,V37Status::Conflict,message.revision,message.revision,
                    BTreeMap::from([(JsonString::from_str("reason"),text("GENERATION_CHANGE_IN_PROGRESS"))])));
            }
        }
        let target=match live_target(self,&request.domain_id,&message.seat_id,&message.generation,
            &message.turn_id) {
            Ok(target)=>target,
            Err(OrchestrationError::AccessDenied)=>return Ok(encode_receipt(request,V37Status::Conflict,
                message.revision,message.revision,BTreeMap::from([
                    (JsonString::from_str("reason"),text("TURN_ENDED"))]))),
            Err(error)=>return Err(error),
        };
        if kind==NativeDeliveryKind::Steer && !target.turn_is_current {
            return Ok(encode_receipt(request,V37Status::Conflict,message.revision,message.revision,
                BTreeMap::from([(JsonString::from_str("reason"),text("TURN_ENDED"))])));
        }
        if !command_encodes(kind,&target.thread,&target.turn,&message.body)? {
            return Ok(encode_receipt(request,V37Status::Denied,message.revision,message.revision,
                Default::default()));
        }
        let owner=&self.owner;
        let reservation=inbox::reserve_delivery(&mut self.connection,envelope,&generation,
            if kind==NativeDeliveryKind::Steer {Some(&message.turn_id)} else {None},|db| {
                authority::check_owner_in_current_transaction(db,owner).map_err(InboxError::Authority)?;
                target_present(db,&target)
            });
        let (operation,new)=match reservation {Ok(value)=>value,
            Err(error)=>return self.inbox_receipt_error(request,error)};
        if new.is_none() {return Ok(outcome(request,&operation,true));}
        let step_id=inbox_step(&request.domain_id,&target.key.1,&request.request_id);
        if kind==NativeDeliveryKind::Steer {
            if let Some(aborted)=inbox::abort_native_steer_if_ended(&mut self.connection,&self.owner,
                envelope,&target.key.1,&step_id,&target.generation,&target.operation,
                &target.ticket,&target.nonce,&target.thread,&target.turn)
                .map_err(|error|inbox_error("H prewrite abort proof",error))? {
                return Ok(outcome(request,&aborted,false));
            }
        }
        // Once the original C intent is durable, only this request may cause
        // H's one physical RPC. A crash here leaves UNKNOWN for reconciliation.
        let started=inbox::mark_commit_unknown(&mut self.connection,envelope,|db| {
            authority::check_owner_in_current_transaction(db,owner).map_err(InboxError::Authority)?;
            target_present(db,&target)
        });
        let unknown=match started {
            Ok(value)=>value,
            Err(InboxError::Denied)=>{
                if kind==NativeDeliveryKind::Steer {
                    if let Some(aborted)=inbox::abort_native_steer_if_ended(&mut self.connection,
                        &self.owner,envelope,&target.key.1,&step_id,&target.generation,
                        &target.operation,&target.ticket,&target.nonce,&target.thread,&target.turn)
                        .map_err(|error|inbox_error("H lost-target abort proof",error))? {
                        return Ok(outcome(request,&aborted,false));
                    }
                }
                inbox::abort_delivery(&mut self.connection,envelope,None,
                    "TARGET_NO_LONGER_CURRENT").map_err(|error|inbox_error("unconfirmed abort",error))?
            },
            Err(error)=>return Err(inbox_error("begin committed",error)),
        };
        if unknown.phase!="UNKNOWN" {
            return Err(OrchestrationError::OperationConflict);
        }
        if unknown.reason=="TARGET_NO_LONGER_CURRENT" {
            return Ok(outcome(request,&unknown,false));
        }
        let response=match kind {
            NativeDeliveryKind::Steer=>self.native_steer_rpc(&target.key,&step_id,&target.thread,
                &target.turn,message.body.clone()),
            NativeDeliveryKind::Deliver=>self.native_append_rpc(&target.key,&step_id,&target.thread,
                message.body.clone()),
        };
        match response {
            Ok(Some(Reply::Ack {..}))=>{
                let settled=inbox::settle_native_delivery_observed(&mut self.connection,&self.owner,
                    envelope,&target.key.1,&step_id,kind)
                    .map_err(|error|inbox_error("A/H completion proof",error))?;
                Ok(outcome(request,&settled,false))
            }
            Ok(_)=>Err(OrchestrationError::OperationConflict),
            Err(error)=>{
                if kind==NativeDeliveryKind::Steer {
                    if let Some(aborted)=inbox::abort_native_steer_if_ended(&mut self.connection,
                        &self.owner,envelope,&target.key.1,&step_id,&target.generation,
                        &target.operation,&target.ticket,&target.nonce,&target.thread,&target.turn)
                        .map_err(|error|inbox_error("H postwrite abort proof",error))? {
                        return Ok(outcome(request,&aborted,false));
                    }
                }
                match inbox::settle_native_delivery_observed(&mut self.connection,&self.owner,
                envelope,&target.key.1,&step_id,kind) {
                    Ok(settled)=>Ok(outcome(request,&settled,false)),
                    Err(InboxError::Denied)=>{
                        let recorded=inbox::record_delivery_unknown_error(&mut self.connection,
                            &self.owner,envelope,&format!("{error:?}"))
                            .map_err(|cause|inbox_error("original H failure record",cause))?;
                        Ok(outcome(request,&recorded,false))
                    },
                    Err(error)=>Err(inbox_error("native failure proof",error)),
                }
            },
        }
    }
}

fn hex_bytes(bytes:&[u8])->String {bytes.iter().map(|byte|format!("{byte:02x}")).collect()}
fn inbox_step(domain:&str,session:&str,request:&str)->String {
    format!("inbox{}",sha256_hex(format!("{domain}\n{session}\n{request}").as_bytes()))
}
fn read_inbox_operation(db:&VerifiedDatabaseConnection<'_>,domain:&str,request:&str)->Result<Option<StoredOperation>> {
    let q=Statement::prepare(db.as_ptr(),"SELECT request_hex,message_id,phase,previous_revision,revision,result_state,reason,native_receipt_id FROM main.gogoke_v37_inbox_operations WHERE domain_id=?1 AND request_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,request)?;
    if !q.step_row()? {return Ok(None);}
    let row=StoredOperation {request_hex:q.column_text(0)?,message_id:q.column_text(1)?,
        phase:q.column_text(2)?,previous_revision:q.column_text(3)?.parse().map_err(|_|OrchestrationError::OperationConflict)?,
        revision:q.column_text(4)?.parse().map_err(|_|OrchestrationError::OperationConflict)?,
        result_state:q.column_text(5)?,reason:q.column_text(6)?,native_receipt_id:q.column_text(7)?};
    if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
    Ok(Some(row))
}
fn read_inbox_message(db:&VerifiedDatabaseConnection<'_>,domain:&str,id:&str)->Result<Option<Message>> {
    let q=Statement::prepare(db.as_ptr(),"SELECT revision,state,sender_seat_id,seat_id,turn_id,generation,body,queued_at,COALESCE(requeued_as,'') FROM main.gogoke_v37_inbox_messages WHERE domain_id=?1 AND message_id=?2")?;
    q.bind_text(1,domain)?;q.bind_text(2,id)?;
    if !q.step_row()? {return Ok(None);}
    let requeued=q.column_text(8)?;
    let row=Message {domain_id:domain.into(),message_id:id.into(),
        revision:q.column_text(0)?.parse().map_err(|_|OrchestrationError::OperationConflict)?,
        state:q.column_text(1)?,sender_seat_id:q.column_text(2)?,seat_id:q.column_text(3)?,
        turn_id:q.column_text(4)?,generation:q.column_text(5)?,body:q.column_text(6)?,
        queued_at:q.column_text(7)?,requeued_as:if requeued.is_empty(){None}else{Some(requeued)}};
    if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
    Ok(Some(row))
}
struct OriginalTarget {session:String,generation:String,operation:String,ticket:String,nonce:String,
    thread:String,turn:String}

/// C's queued row has a seat/turn/generation, while H's process episode owns
/// the durable session and original physical identity. Resolve exactly one
/// original episode; the current claim may already point to a newer process.
fn original_target(db:&VerifiedDatabaseConnection<'_>,domain:&str,message:&str,
    delivery_request:Option<&str>)->Result<Option<OriginalTarget>> {
    let row=read_inbox_message(db,domain,message)?;
    let Some(row)=row else {return Ok(None);};
    let q=Statement::prepare(db.as_ptr(),
        "SELECT ep.session_id,ep.generation,ep.process_operation_id,c.ticket,
                c.custodian_nonce,ep.request_id
           FROM main.gogoke_v37_h_process_episode ep
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=ep.process_operation_id AND c.domain_id=ep.domain_id
             AND c.generation=ep.generation
          WHERE ep.domain_id=?1 AND ep.seat_id=?2 AND ep.generation=?3
            AND ((ep.phase='ACTIVE' AND c.state IN ('ACTIVE','UNKNOWN'))
              OR (ep.phase='UNKNOWN' AND c.state='UNKNOWN')
              OR (ep.phase='STOPPED' AND c.state='STOPPED'
                AND ep.stop_fact_id=c.stop_proof_hash AND length(c.stop_proof_hash)>0))")?;
    for (index,value) in [domain,row.seat_id.as_str(),row.generation.as_str()].iter().enumerate(){
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Ok(None);}
    let session=q.column_text(0)?;let generation=q.column_text(1)?;
    let operation=q.column_text(2)?;let ticket=q.column_text(3)?;
    let nonce=q.column_text(4)?;let open_request=q.column_text(5)?;
    if q.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(q);
    if let Some(request)=delivery_request {
        let step=inbox_step(domain,&session,request);
        let observed=Statement::prepare(db.as_ptr(),
            "SELECT process_operation_id,generation,ticket,custodian_nonce
             FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND step_id=?3")?;
        observed.bind_text(1,domain)?;observed.bind_text(2,&session)?;observed.bind_text(3,&step)?;
        if observed.step_row()? {
            let matches=observed.column_text(0)?==operation && observed.column_text(1)?==generation
                && observed.column_text(2)?==ticket && observed.column_text(3)?==nonce;
            if !matches || observed.step_row()? {return Err(OrchestrationError::OperationConflict);}
        }
    }
    let thread=match crate::store::session_transport::rpc_journal::observed_thread_id(db,
        domain,&session,&operation,&generation,&open_request,&ticket,&nonce) {
        Ok(thread)=>thread,
        Err(crate::store::session_transport::rpc_journal::RpcJournalError::Denied)=>return Ok(None),
        Err(error)=>return Err(inbox_error("original H thread",error)),
    };
    Ok(Some(OriginalTarget {session,generation,operation,ticket,nonce,thread,turn:row.turn_id}))
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::route_b_test_guard;
    use crate::store::session_transport::{decode_request,decode_receipt};
    use std::time::{SystemTime,UNIX_EPOCH};

    fn request(verb:&str,id:&str,target:&str,revision:u64,payload:&str)->V37Request {
        decode_request(format!(r#"{{"schema":"gogoke.37.operations.v1","family":"K-INBOX","operation":"{verb}","requestId":"{id}","targetId":"{target}","domainId":"projectA","expectedRevision":"{revision}","payload":{payload}}}"#).as_bytes()).unwrap()
    }
    fn reply(product:&mut ProductDatabase<'_>,request:&V37Request)
        ->crate::store::session_transport::V37Receipt {
        decode_receipt(&product.dispatch_user_request(request).unwrap()).unwrap()
    }

    #[test]
    fn user_inbox_read_replay_collision_stale_and_cancel_cas_use_original_store() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!("gogoke-inbox-user-{}-{stamp}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let mut product=ProductDatabase::open(&root,&folder.join("state.sqlite")).unwrap();
        // Synthetic C source row only. This proves the real User dispatcher
        // and A receipt path; it is not a Codex process or model delivery.
        product.connection.execute("INSERT INTO gogoke_v37_inbox_messages(domain_id,message_id,revision,state,sender_seat_id,seat_id,turn_id,generation,body) VALUES('projectA','messageA','1','PENDING','User','seatA','turnA','1','hello')").unwrap();
        let read=request("check-unknown","readA","messageA",1,"{}");
        assert_eq!(reply(&mut product,&read).status,V37Status::Applied);
        assert_eq!(reply(&mut product,&read).status,V37Status::Replayed);
        let collision=request("check-unknown","readA","messageB",1,"{}");
        assert_eq!(reply(&mut product,&collision).status,V37Status::Conflict);
        let cancel=request("cancel","cancelA","messageA",1,"{}");
        let cancelled=reply(&mut product,&cancel);
        assert_eq!(cancelled.status,V37Status::Applied);
        assert_eq!(cancelled.revision,2);
        assert_eq!(reply(&mut product,&read).status,V37Status::Stale);
        let stale=request("cancel","cancelB","messageA",1,"{}");
        let stale=reply(&mut product,&stale);
        assert_eq!(stale.status,V37Status::Stale);
        assert_eq!(stale.revision,2,"STALE reports the current source revision");
        product.close_checked().unwrap();drop(root);std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn inbox_preflight_uses_real_codec_and_largest_valid_rpc_id() {
        assert!(command_encodes(NativeDeliveryKind::Deliver,"threadA","turnA","short").unwrap());
        assert!(!command_encodes(NativeDeliveryKind::Deliver,"threadA","turnA",
            &"x".repeat(1024*1024)).unwrap());
        assert!(!command_encodes(NativeDeliveryKind::Steer,"threadA","turnA",
            &"x".repeat(1024*1024)).unwrap());
    }
}
