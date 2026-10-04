//! Existing authority-thread safe point: original E cause -> C pending ->
//! ordinary H send. Busy/unavailable targets stay queued; no synthetic turn.
use super::*;
use crate::store::atomic::Parser;
use crate::store::inbox::{self,host_rule as c};
use crate::store::seat::{self,HostEscalationProof,NativeOrigin};
use crate::store::session_transport::{self as h,runtime};

fn failure(error:impl std::fmt::Debug)->OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("original host rule: {error:?}"))
}
fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn field(fields:&BTreeMap<JsonString,Json>,name:&str)->Result<String> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::String(value))=>value.to_well_formed_string().ok_or(OrchestrationError::Invalid("original host rule string")),
        _=>Err(OrchestrationError::Invalid("original host rule field")),
    }
}
pub(super) fn original_bytes(encoded:&str)->Result<Vec<u8>> {
    if encoded.len()%2!=0 {return Err(OrchestrationError::Invalid("original host rule hex"));}
    encoded.as_bytes().chunks_exact(2).map(|pair| {
        let value=std::str::from_utf8(pair).map_err(failure)?;
        u8::from_str_radix(value,16).map_err(failure)
    }).collect::<Result<Vec<_>>>()
}
fn original_fields(encoded:&str)->Result<BTreeMap<JsonString,Json>> {
    let bytes=original_bytes(encoded)?;
    let Json::Object(fields)=Parser::parse(std::str::from_utf8(&bytes).map_err(failure)?)? else {
        return Err(OrchestrationError::Invalid("original host rule object"));
    };
    Ok(fields)
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn pump_host_rules(&mut self)->Result<()> {
        self.settle_original_host_deliveries()?;
        self.cleanup_host_rule_preparations()?;
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT domain_id,gate_id FROM main.gogoke_v37_seat_policy_gates WHERE state='ESCALATION_REQUIRED' ORDER BY domain_id,gate_id")?;
        let mut causes=Vec::new();while query.step_row()? {causes.push((query.column_text(0)?,query.column_text(1)?));}drop(query);
        for (domain,gate) in causes {
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let selected=(||->Result<Option<HostEscalationProof>> {
                let proof=match seat::observe_host_reject_cap_in_transaction(&self.connection,&self.owner,&domain,&gate) {
                    Ok(Some(proof))=>proof,Ok(None)|Err(seat::SeatError::Denied|seat::SeatError::Conflict)=>return Ok(None),
                    Err(error)=>return Err(failure(error)),
                };
                match seat::begin_host_escalation_in_transaction(&mut self.connection,&self.owner,&proof) {
                    Ok(_)=>{},Err(seat::SeatError::Denied|seat::SeatError::Conflict)=>return Ok(None),
                    Err(error)=>return Err(failure(error)),
                }
                c::enqueue_host_escalation_in_transaction(&mut self.connection,&self.owner,&proof).map_err(failure)?;
                Ok(Some(proof))
            })();
            let proof=match selected {
                Ok(proof)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;proof},
                Err(primary)=>{if let Err(rollback)=self.connection.execute("ROLLBACK") {return Err(failure((primary,rollback)));}return Err(primary);}
            };
            let Some(proof)=proof else {continue;};
            let ids=c::host_message_ids(&proof);
            let message=inbox::read_message(&self.connection,proof.domain_id(),&ids.message_id).map_err(failure)?
                .ok_or(OrchestrationError::OperationConflict)?;
            if message.state!="PENDING" {continue;}
            // OWNER is the existing native notification endpoint, never a
            // model recipient. C retains the original pending notice; the
            // User projection displays it without a fabricated H delivery.
            if proof.destination_seat_id()=="OWNER" {continue;}
            if c::host_recipient_failure_recorded(&self.connection,&proof).map_err(failure)? {
                continue;
            }
            // No current process or a currently busy upper seat is not a
            // failed delivery. The original C pending survives without a turn.
            let candidate=self.native_sessions.iter().filter(|(key,run)|key.0==proof.domain_id()
                &&run.evidence.seat_id()==proof.destination_seat_id()&&run.evidence.driver_id()=="codex"
                &&run.thread_id.is_some()&&run.allows_input())
                .map(|(key,run)|(key.clone(),run.custody.clone())).collect::<Vec<_>>();
            let key=match candidate.as_slice() {
                [(key,_)]=>{
                    if c::read_host_recipient(&self.connection,&proof).map_err(failure)?
                        .is_some_and(|choice|choice.session_id!=key.1) {continue;}
                    key.clone()
                },
                []=>match self.prepare_host_rule_recipient(&proof) {
                    Ok(Some(key))=>key,
                    Ok(None)=>continue,
                    Err(original)=>{
                        match c::record_host_recipient_error(&mut self.connection,
                            &self.owner,&proof,&format!("{original:?}")) {
                            Ok(())|Err(inbox::InboxError::Denied|inbox::InboxError::Conflict)=>{},
                            Err(record)=>return Err(failure((original,record))),
                        }
                        continue;
                    },
                },
                _=>continue,
            };
            if !self.native_sessions.contains_key(&key) {continue;}
            self.drain_native_output(&key)?;
            if !self.host_rule_recipient_idle(&key)? {continue;}
            let custody=self.native_sessions.get(&key)
                .ok_or(OrchestrationError::AccessDenied)?.custody.clone();
            let registration=crate::store::ledger::read_registered_session(&self.connection,&key.1)?
                .ok_or(OrchestrationError::AccessDenied)?;
            if registration.purpose!=crate::store::ledger::SessionPurpose::Work {continue;}
            let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
                &key.0,proof.destination_seat_id(),&key.1).map_err(failure)?
                .ok_or(OrchestrationError::AccessDenied)?;
            let target=c::HostDeliveryTarget {session_id:&key.1,ticket:custody.ticket.opaque(),
                generation:&custody.binding.generation};
            let (operation,permit)=match c::reserve_host_delivery(&mut self.connection,&self.owner,&proof,&target) {
                Ok(reserved)=>reserved,
                // The retained process may now be UNKNOWN or stopped. C's
                // original target guard grants no write in that case.
                Err(inbox::InboxError::Denied|inbox::InboxError::Conflict)=>continue,
                Err(error)=>return Err(failure(error)),
            };
            if permit.is_none() {continue;}
            let raw=Json::Object(BTreeMap::from([
                (JsonString::from_str("schema"),text("gogoke.37.operations.v1")),
                (JsonString::from_str("family"),text("K-SESSION")),(JsonString::from_str("operation"),text("send")),
                (JsonString::from_str("requestId"),text(&ids.send_request_id)),(JsonString::from_str("targetId"),text(&key.1)),
                (JsonString::from_str("domainId"),text(&key.0)),(JsonString::from_str("expectedRevision"),text(&claim.revision.to_string())),
                (JsonString::from_str("payload"),Json::Object(BTreeMap::from([
                    (JsonString::from_str("generation"),text(&custody.binding.generation)),
                    (JsonString::from_str("body"),text(proof.notice_body())),
                ]))),
            ])).canonical();
            let request=h::decode_request(raw.as_bytes()).map_err(failure)?;
            // Commit the original uncertainty before entering H's sole
            // writer, as the existing C delivery path does. A crash never
            // turns the same notification into permission for a second send.
            c::mark_host_delivery_unknown(&mut self.connection,&self.owner,
                proof.domain_id(),&ids.message_id,&target).map_err(failure)?;
            match self.dispatch_host_rule_send(&request,&proof) {
                Ok(bytes)=>{
                    let receipt=h::decode_receipt(&bytes).map_err(failure)?;
                    if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed|V37Status::Unknown) {
                        self.record_host_delivery_error(&proof,&ids,&operation,
                            &format!("original H receipt: {}",String::from_utf8_lossy(&bytes)))?;
                    }
                    self.settle_original_host_deliveries()?;
                },
                Err(original @ OrchestrationError::NativeRecipientFailure(_))=>{
                    self.record_host_delivery_error(&proof,&ids,&operation,&format!("{original:?}"))?;
                    self.settle_original_host_deliveries()?;
                },
                Err(original)=>{
                    if let Err(record)=self.record_host_delivery_error(&proof,&ids,&operation,&format!("{original:?}")) {
                        return Err(failure((original,record)));
                    }
                    return Err(original);
                },
            }
        }
        self.cleanup_host_rule_preparations()?;
        Ok(())
    }

    fn record_host_delivery_error(&mut self,proof:&HostEscalationProof,
        ids:&c::HostMessageIds,operation:&inbox::StoredOperation,original:&str)->Result<()> {
        let bytes=original_bytes(&operation.request_hex)?;
        let envelope=inbox::InboxEnvelope {domain_id:proof.domain_id(),message_id:&ids.message_id,
            request_id:&ids.delivery_request_id,request_bytes:&bytes,expected_revision:operation.previous_revision};
        // Reuse C's exact-request original error record. One recipient's
        // uncertain response must not terminate the host authority loop.
        inbox::record_delivery_unknown_error(&mut self.connection,&self.owner,&envelope,original).map_err(failure)?;
        Ok(())
    }

    /// Prefix alone is never authority: the internal path holds the E seal
    /// and checks the original prepared C request before entering H's writer.
    pub(super) fn check_host_rule_send(&mut self,request:&V37Request,proof:&HostEscalationProof)->Result<()> {
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let checked=(||->Result<()> {
            seat::revalidate_host_escalation_in_transaction(&self.connection,&self.owner,proof).map_err(failure)?;
            let ids=c::host_message_ids(proof);
            let message=inbox::read_message(&self.connection,proof.domain_id(),&ids.message_id).map_err(failure)?
                .ok_or(OrchestrationError::AccessDenied)?;
            let operation=inbox::read_operation(&self.connection,proof.domain_id(),&ids.delivery_request_id).map_err(failure)?
                .ok_or(OrchestrationError::AccessDenied)?;
            let original=original_fields(&operation.request_hex)?;
            if request.request_id!=ids.send_request_id||request.family!="K-SESSION"||request.operation!="send"
                ||request.domain_id!=proof.domain_id()||request.payload.len()!=2
                ||message.sender_seat_id!=c::HOST_RULE_ACTOR||message.state!="UNKNOWN"||operation.phase!="UNKNOWN"
                ||message.seat_id!=proof.destination_seat_id()||message.body!=proof.notice_body()
                ||field(&original,"actor")?!=c::HOST_RULE_ACTOR||field(&original,"sessionId")?!=request.target_id
                ||field(&original,"hSendRequestId")?!=request.request_id
                ||field(&original,"generation")?!=field(&request.payload,"generation")?
                ||field(&request.payload,"body")?!=message.body {
                return Err(OrchestrationError::AccessDenied);
            }
            Ok(())
        })();
        match checked {Ok(())=>self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause),
            Err(primary)=>{if let Err(rollback)=self.connection.execute("ROLLBACK") {return Err(failure((primary,rollback)));}Err(primary)}}
    }

    fn settle_original_host_deliveries(&mut self)->Result<()> {
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT o.domain_id,o.message_id,o.request_hex FROM main.gogoke_v37_inbox_operations o
               JOIN main.gogoke_v37_inbox_messages m ON m.domain_id=o.domain_id AND m.message_id=o.message_id
              WHERE m.sender_seat_id='HOST_RULE' AND o.request_id LIKE 'hostdeliver-%'
                AND o.phase IN ('PREPARED','UNKNOWN','APPLIED') AND m.state!='CANCELLED'")?;
        let mut rows=Vec::new();while query.step_row()? {rows.push((query.column_text(0)?,query.column_text(1)?,query.column_text(2)?));}drop(query);
        for (domain,message,raw) in rows {
            let original=original_fields(&raw)?;
            let session=field(&original,"sessionId")?;let ticket=field(&original,"ticket")?;
            let generation=field(&original,"generation")?;
            let target=c::HostDeliveryTarget {session_id:&session,ticket:&ticket,generation:&generation};
            c::mark_host_delivery_unknown(&mut self.connection,&self.owner,&domain,&message,&target)
                .map_err(failure)?;
            h::rpc_journal::reconcile_written_sends_from_a(&mut self.connection,&self.owner,
                &domain,&session,&generation).map_err(failure)?;
            h::reconcile_observed_codex_sends(&mut self.connection,&domain,&session,&generation)
                .map_err(failure)?;
            let operation=match c::settle_host_turn_start_observed(&mut self.connection,&self.owner,&domain,&message,&target) {
                Ok(operation)=>operation,Err(inbox::InboxError::Unknown|inbox::InboxError::Denied)=>continue,
                Err(error)=>return Err(failure(error)),
            };
            if operation.phase!="APPLIED" {continue;}
            let trigger=field(&original,"triggerId")?;let escalation=field(&original,"escalationRequestId")?;
            let destination=field(&original,"destinationSeatId")?;
            let evidence=seat::NativeDeliveryEvidence::from_verified_c_delivery(&domain,&trigger,&escalation,
                &destination,&operation.native_receipt_id).map_err(failure)?;
            seat::settle_escalation(&mut self.connection,&evidence).map_err(failure)?;
        }
        Ok(())
    }
}
