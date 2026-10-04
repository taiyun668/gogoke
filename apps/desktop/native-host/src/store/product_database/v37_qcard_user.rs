//! User-origin native card actions select stored C/H identities. Provider
//! request IDs, payloads, process paths and write receipts come from native
//! observations; a User request cannot manufacture a raised provider card.
use super::*;
use crate::store::atomic::Parser;
use crate::store::inbox::{CardEnvelope,NativeQuestionCard};

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn claude_card(card:&NativeQuestionCard)->Result<bool> {
    let Json::Object(payload)=Parser::parse(&card.question_payload)? else {
        return Err(OrchestrationError::OperationConflict);
    };
    Ok(payload.get(&JsonString::from_str("provider"))
        ==Some(&Json::String(JsonString::from_str("claude"))))
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_user_qcard(&mut self,request:&V37Request)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if !matches!(request.operation.as_str(),"answer"|"recover") {
            return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,BTreeMap::from([(JsonString::from_str("reason"),text("NATIVE_CARD_LIFECYCLE_REQUIRES_ORIGINAL_PROVIDER_OR_STOP_FACT"))])));
        }
        // All C operations, plus C reads in A's existing receipt table, share
        // request identity. Compare the original wire before any pipe write.
        let originals=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_qcard_native_operations WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT request_hex FROM main.gogoke_v37_qcard_operations WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT lower(hex(request_bytes)) FROM main.v37_ledger_receipt WHERE family='K-QCARD' AND domain_id=?1 AND request_id=?2")?;
        originals.bind_text(1,&request.domain_id)?;originals.bind_text(2,&request.request_id)?;
        let exact:String=request.raw_bytes.iter().map(|value|format!("{value:02x}")).collect();
        while originals.step_row()? {
            if originals.column_text(0)?!=exact {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,request.expected_revision,Default::default()));
            }
        }
        drop(originals);
        let exists=Statement::prepare(self.connection.as_ptr(),"SELECT 1 FROM main.gogoke_v37_qcard_native WHERE domain_id=?1 AND card_id=?2")?;
        exists.bind_text(1,&request.domain_id)?;exists.bind_text(2,&request.target_id)?;
        if !exists.step_row()? {return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,request.expected_revision,Default::default()));}drop(exists);
        let (source,_)=super::v37_qcard::read_source_descriptor(&self.connection,&request.domain_id,&request.target_id)?;
        let claim=Statement::prepare(self.connection.as_ptr(),
            "SELECT h.session_id FROM main.gogoke_v37_qcard_native q JOIN main.gogoke_v37_h_process_episode h ON h.domain_id=q.domain_id AND h.seat_id=q.seat_id AND h.generation=q.generation WHERE q.domain_id=?1 AND q.card_id=?2 AND h.process_operation_id=?3")?;
        claim.bind_text(1,&request.domain_id)?;claim.bind_text(2,&request.target_id)?;
        claim.bind_text(3,&source.operation_id)?;
        if !claim.step_row()? {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,request.expected_revision,Default::default()));
        }
        let key=(request.domain_id.clone(),claim.column_text(0)?);
        if claim.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(claim);
        if self.native_sessions.get(&key).is_some_and(|run|run.allows_input()) {
            // Capture already available expiry/terminal facts before deciding
            // whether this new answer is still permitted. No extra wait.
            self.drain_native_output(&key)?;
        }
        let card=self.query_codex_card_recovered(&key,&request.target_id)?.ok_or(OrchestrationError::OperationConflict)?;
        if request.operation=="recover" {
            if !request.payload.is_empty() {return Err(OrchestrationError::Invalid("native card recover payload"));}
            if card.revision!=request.expected_revision {
                return Ok(encode_receipt(request,V37Status::Stale,card.revision,card.revision,Default::default()));
            }
            return self.native_card_read_receipt(request,&key,&card);
        }
        if request.payload.len()!=2 || user_payload_string(request,"generation")?!=card.generation {
            return Ok(encode_receipt(request,V37Status::Conflict,card.revision,card.revision,Default::default()));
        }
        let Some(Json::Object(answers))=request.payload.get(&JsonString::from_str("answers")) else {
            return Err(OrchestrationError::Invalid("native card answers object"));
        };
        let mut values=BTreeMap::new();
        for (id,answer) in answers {
            let id=id.to_well_formed_string().ok_or(OrchestrationError::Invalid("native question ID"))?;
            let Json::Array(answer)=answer else {return Err(OrchestrationError::Invalid("native answer array"));};
            let mut items=Vec::new();
            for value in answer {
                let Json::String(value)=value else {return Err(OrchestrationError::Invalid("native answer text"));};
                items.push(value.to_well_formed_string().ok_or(OrchestrationError::Invalid("native answer text"))?);
            }
            values.insert(id,items);
        }
        let envelope=CardEnvelope {domain_id:&request.domain_id,card_id:&request.target_id,
            request_id:&request.request_id,request_bytes:&request.raw_bytes,expected_revision:request.expected_revision};
        let completed=if claude_card(&card)? {
            self.answer_claude_card(&key,&envelope,values)?
        } else {self.answer_codex_card(&key,&envelope,values)?};
        let operation=completed.operation;
        let status=if operation.phase=="ANSWERED" {
            if completed.newly_written {V37Status::Applied} else {V37Status::Replayed}
        } else {V37Status::Unknown};
        Ok(encode_receipt(request,status,operation.previous_revision,operation.revision,BTreeMap::from([
            (JsonString::from_str("state"),text(&operation.phase)),
            (JsonString::from_str("nativeReceiptId"),text(&operation.native_receipt_id)),
            (JsonString::from_str("deliveryBasis"),text(if operation.phase=="ANSWERED" {"NATIVE_EXACT_WRITE_RECEIPT"} else {"OUTCOME_UNKNOWN"})),
            (JsonString::from_str("vendorConsumptionConfirmed"),Json::Bool(false)),
        ])))
    }

    fn native_card_read_receipt(&mut self,request:&V37Request,key:&(String,String),card:&NativeQuestionCard)->Result<Vec<u8>> {
        let mut ready=false;
        let mut availability_error=Json::Null;
        if let Some(run)=self.native_sessions.get(key) {
            let current_turn=if run.evidence.driver_id()=="claude" {
                run.pending_claude.as_ref().and_then(|(bytes,_)|
                    crate::store::session_transport::decode_request(bytes).ok())
                    .filter(|send|send.operation=="send")
                    .map(|send|send.request_id)
            } else {run.turn_id.clone()};
            if card.state=="OPEN" && run.allows_input() && run.custody.binding.generation==card.generation
                && run.thread_id.as_deref()==Some(card.vendor_thread_id.as_str())
                && current_turn.as_deref()==Some(card.turn_id.as_str()) {
                let claim=crate::store::session_transport::runtime::observe_claim(&self.connection,
                    &crate::store::seat::NativeOrigin::user(&self.owner),&key.0,&card.seat_id,&key.1)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("native card current read: {error:?}")))?
                    .ok_or(OrchestrationError::AccessDenied)?;
                match run.evidence.verify_live(&mut self.connection,self.root,&self.owner,&run.operation_id,claim.revision) {
                    Ok(())=> {
                        ready=crate::store::session_transport::generation_change::active_for_session(&self.connection,&key.0,&key.1)?.is_none();
                        if !ready {availability_error=text("GENERATION_CHANGE_IN_PROGRESS");}
                    }
                    Err(error)=>availability_error=text(&error),
                }
            }
        }
        let result=BTreeMap::from([
            (JsonString::from_str("state"),text(&card.state)),
            (JsonString::from_str("seatId"),text(&card.seat_id)),
            (JsonString::from_str("turnId"),text(&card.turn_id)),
            (JsonString::from_str("generation"),text(&card.generation)),
            (JsonString::from_str("requestRef"),text(&card.vendor_request_id)),
            (JsonString::from_str("nativeQuestion"),Parser::parse(&card.question_payload)?),
            (JsonString::from_str("availableForAnswer"),Json::Bool(ready)),
            (JsonString::from_str("inputAvailabilityError"),availability_error),
        ]);
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(||->Result<Vec<u8>> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let version=Statement::prepare(self.connection.as_ptr(),"SELECT revision,state FROM main.gogoke_v37_qcard_native WHERE domain_id=?1 AND card_id=?2")?;
            version.bind_text(1,&key.0)?;version.bind_text(2,&card.card_id)?;
            if !version.step_row()? || version.column_text(0)?!=card.revision.to_string() || version.column_text(1)?!=card.state {return Err(OrchestrationError::OperationConflict);}drop(version);
            let prior=Statement::prepare(self.connection.as_ptr(),"SELECT receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-QCARD' AND domain_id=?1 AND request_id=?2")?;
            prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
            if prior.step_row()? {
                let receipt=crate::store::session_transport::decode_receipt(prior.column_text(0)?.as_bytes()).map_err(|error|OrchestrationError::V37StoreFailure(format!("native card read receipt: {error:?}")))?;
                let original=receipt.into_result();drop(prior);
                let same=original.len()==result.len() && original.iter().all(|(key,value)|
                    result.get(key).is_some_and(|candidate|candidate.canonical()==value.canonical()));
                if same {
                    return Ok(encode_receipt(request,V37Status::Replayed,card.revision,card.revision,original));
                }
                return Ok(encode_receipt(request,V37Status::Stale,card.revision,card.revision,Default::default()));
            }
            drop(prior);
            let bytes=encode_receipt(request,V37Status::Applied,card.revision,card.revision,result);
            if bytes.len()>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("native card receipt bound"));}
            let insert=Statement::prepare(self.connection.as_ptr(),"INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-QCARD',?1,?2,?3,?4)")?;
            insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
            insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(bytes)
        })();
        match read {
            Ok(bytes)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=>{
                if let Err(error)=self.connection.execute("ROLLBACK") {return Err(OrchestrationError::V37StoreFailure(format!("native card read: {primary:?}; rollback: {error:?}")));}
                Err(primary)
            }
        }
    }
}
