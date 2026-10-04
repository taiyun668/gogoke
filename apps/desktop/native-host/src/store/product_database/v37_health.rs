//! Host health uses sealed terminal H/A facts and the existing generation
//! journal. It has no model permissions, new scheduler or work-input resend.
use super::*;
use crate::store::ledger::RawSourceKey;
use crate::store::session_transport::{self as h,host_health,generation_change as change,runtime};
use crate::store::seat::{self,NativeOrigin};

fn source_error(error:impl std::fmt::Debug)->OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("original host health: {error:?}"))
}
fn health_id(source:&str)->String {format!("health-{}",crate::store::digest::sha256_hex(source.as_bytes()))}

impl<'root> ProductDatabase<'root> {
    pub(super) fn observe_host_health_raw_in_transaction(&mut self,key:&(String,String),source:&RawSourceKey)->Result<()> {
        let Some(run)=self.native_sessions.get(key) else {return Ok(());};
        let proof=match host_health::observe_codex_host_health(&self.connection,&self.owner,&run.custody,source) {
            Ok(Some(proof))=>proof,
            Ok(None)=>return Ok(()),
            Err(host_health::HostHealthError::Denied|host_health::HostHealthError::Rpc(h::rpc_journal::RpcJournalError::Denied))=>return Ok(()),
            Err(error)=>return Err(source_error(error)),
        };
        seat::observe_host_health_in_transaction(&mut self.connection,&self.owner,&proof,
            &health_id(proof.source_event_id()))?;
        Ok(())
    }

    /// Called only after the authority-thread drain, never inside a raw-frame
    /// traversal. An old captured source must revalidate before any H intent.
    pub(super) fn pump_host_health(&mut self)->Result<()> {
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT h.event_id,r.domain_id,r.session_id,r.operation_id,r.source_epoch,r.source_cursor FROM main.gogoke_v37_seat_health h JOIN main.v37_ledger_raw_source r ON r.resolved_event_id=h.source_event_id AND r.domain_id=h.domain_id WHERE h.state='OBSERVED' AND h.action IN ('COMPACT','RENEW') ORDER BY h.event_id")?;
        let mut observed=Vec::new();
        while query.step_row()? {observed.push((query.column_text(0)?,(query.column_text(1)?,query.column_text(2)?),RawSourceKey {
            operation_id:query.column_text(3)?,source_epoch:query.column_text(4)?,source_cursor:query.column_text(5)?,
        }));}
        drop(query);
        for (event_id,key,source) in observed {
            let Some(run)=self.native_sessions.get(&key) else {continue;};
            let custody=run.custody.clone();
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let selected=(||->Result<Option<(host_health::HostHealthProof,V37Request)>> {
                let proof=match host_health::observe_codex_host_health(&self.connection,&self.owner,&custody,&source) {
                    Ok(Some(proof))=>proof,Ok(None)=>return Ok(None),
                    Err(host_health::HostHealthError::Denied|host_health::HostHealthError::Rpc(h::rpc_journal::RpcJournalError::Denied))=>return Ok(None),
                    Err(error)=>return Err(source_error(error)),
                };
                if health_id(proof.source_event_id())!=event_id {return Err(OrchestrationError::OperationConflict);}
                let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
                    &key.0,proof.seat_id(),&key.1).map_err(source_error)?.ok_or(OrchestrationError::AccessDenied)?;
                let operation=match proof.signal() {seat::HealthSignal::ContextCompact=>"compact",
                    seat::HealthSignal::RepeatedFailure=>"renew-session",
                    _=>return Err(OrchestrationError::Invalid("host health action")),};
                let raw=Json::Object(BTreeMap::from([
                    (JsonString::from_str("schema"),text("gogoke.37.operations.v1")),
                    (JsonString::from_str("family"),text("K-SESSION")),
                    (JsonString::from_str("operation"),text(operation)),
                    (JsonString::from_str("requestId"),text(&format!("health-change-{}",&crate::store::digest::sha256_hex(event_id.as_bytes())[..40]))),
                    (JsonString::from_str("targetId"),text(&key.1)),
                    (JsonString::from_str("domainId"),text(&key.0)),
                    (JsonString::from_str("expectedRevision"),text(&claim.revision.to_string())),
                    (JsonString::from_str("payload"),Json::Object(BTreeMap::from([(JsonString::from_str("generation"),text(&claim.generation))]))),
                ])).canonical();
                let request=h::decode_request(raw.as_bytes()).map_err(source_error)?;
                Ok(Some((proof,request)))
            })();
            let selected=match selected {
                Ok(value)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;value},
                Err(primary)=>{
                    if let Err(rollback)=self.connection.execute("ROLLBACK") {return Err(source_error((primary,rollback)));}
                    return Err(primary);
                }
            };
            if let Some((proof,request))=selected {
                if !self.user_session_request_identity_matches(&request)? {return Err(OrchestrationError::OperationConflict);}
                self.dispatch_native_generation_change_with_health(&request,Some((&proof,&event_id)))?;
            }
        }
        // A pending action may continue only after an original ACK/completion
        // or stop fact proves progress. UNKNOWN itself grants no send permit.
        let pending=Statement::prepare(self.connection.as_ptr(),
            "SELECT c.raw_hex FROM main.gogoke_v37_seat_health e JOIN main.gogoke_v37_h_generation_change c ON c.domain_id=e.domain_id AND c.request_id=e.session_request_id WHERE e.state='REQUESTED' AND c.owner_stop_request_id IS NULL AND c.stage NOT IN ('CANCELLED','UNSUPPORTED')")?;
        let mut original=Vec::new();while pending.step_row()? {original.push(pending.column_text(0)?);}
        drop(pending);
        for encoded in original {
            let bytes=decode_original_hex(&encoded)?;
            let request=h::decode_request(&bytes).map_err(source_error)?;
            if self.health_generation_may_continue(&request)? {
                let outcome=self.dispatch_native_generation_change(&request)?;
                let receipt=h::decode_receipt(&outcome).map_err(source_error)?;
                if matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
                    self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    let settled=self.settle_host_health_change_in_transaction(&request,&outcome);
                    self.finish_host_health_transaction(settled)?;
                }
            }
        }
        Ok(())
    }

    fn finish_host_health_transaction(&mut self,result:Result<()>)->Result<()> {
        match result {Ok(())=>self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause),
            Err(primary)=>{if let Err(rollback)=self.connection.execute("ROLLBACK") {return Err(source_error((primary,rollback)));}Err(primary)}}
    }

    /// H calls this with mark_applied in the same transaction. The saved exact
    /// K-SESSION bytes link the old source to the actual new-generation receipt.
    pub(super) fn settle_host_health_change_in_transaction(&mut self,request:&V37Request,bytes:&[u8])->Result<()> {
        authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT event_id,generation,action,state,COALESCE(receipt_id,'') FROM main.gogoke_v37_seat_health WHERE domain_id=?1 AND session_request_id=?2")?;
        row.bind_text(1,&request.domain_id)?;row.bind_text(2,&request.request_id)?;
        if !row.step_row()? {return Ok(());}
        let event_id=row.column_text(0)?;let old_generation=row.column_text(1)?;
        let action=row.column_text(2)?;let state=row.column_text(3)?;let saved=row.column_text(4)?;
        if row.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(row);
        let receipt=h::decode_receipt(bytes).map_err(source_error)?;
        let c=change::read(&self.connection,&request.domain_id,&request.request_id).map_err(source_error)?
            .ok_or(OrchestrationError::OperationConflict)?;
        let fields=h::decode_receipt(bytes).map_err(source_error)?.into_result();
        let old=match fields.get(&JsonString::from_str("oldGeneration")) {Some(Json::String(value))=>value.to_well_formed_string(),_=>None};
        let new=match fields.get(&JsonString::from_str("newGeneration")) {Some(Json::String(value))=>value.to_well_formed_string(),_=>None};
        let source_receipt=match fields.get(&JsonString::from_str("receiptId")) {Some(Json::String(value))=>value.to_well_formed_string(),_=>None};
        let expected_new=old_generation.parse::<u64>().map_err(source_error)?.checked_add(1)
            .ok_or(OrchestrationError::Invalid("host health generation"))?.to_string();
        let episode=Statement::prepare(self.connection.as_ptr(),
            "SELECT process_operation_id FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND generation=?4 AND phase IN ('ACTIVE','STOPPED')")?;
        episode.bind_text(1,&request.domain_id)?;episode.bind_text(2,&request.request_id)?;
        episode.bind_text(3,&request.target_id)?;episode.bind_text(4,&expected_new)?;
        if !episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let process=episode.column_text(0)?;
        if episode.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(episode);
        let actual_source_receipt=self.resume_source_receipt(&request.domain_id,&request.target_id,
            &process,&expected_new,&request.request_id)?;
        if c.raw_hex!=hex(&request.raw_bytes)||c.stage!="APPLIED"||c.session_id!=request.target_id
            ||c.old_generation!=old_generation||receipt.family!="K-SESSION"
            ||receipt.request_id!=request.request_id||receipt.target_id!=request.target_id
            ||receipt.operation!=request.operation||!matches!(receipt.status,V37Status::Applied|V37Status::Replayed)
            ||old.as_deref()!=Some(old_generation.as_str())||new.as_deref()!=Some(expected_new.as_str())
            ||source_receipt.as_deref()!=Some(actual_source_receipt.as_str())
            ||c.result_revision!=Some(i64::try_from(receipt.revision).map_err(source_error)?)
            ||!matches!((action.as_str(),request.operation.as_str()),("COMPACT","compact")|("RENEW","renew-session")) {
            return Err(OrchestrationError::OperationConflict);
        }
        let receipt_id=format!("health-receipt-{}",crate::store::digest::sha256_hex(bytes));
        if state=="RECEIPTED" {return if saved==receipt_id {Ok(())} else {Err(OrchestrationError::OperationConflict)};}
        if state!="REQUESTED" {return Err(OrchestrationError::OperationConflict);}
        let insert=Statement::prepare(self.connection.as_ptr(),
            "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-SESSION',?1,?2,?3,?4)")?;
        insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
        insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,bytes)?;insert.step_done()?;drop(insert);
        let update=Statement::prepare(self.connection.as_ptr(),
            "UPDATE main.gogoke_v37_seat_health SET state='RECEIPTED',receipt_id=?1 WHERE domain_id=?2 AND event_id=?3 AND state='REQUESTED'")?;
        update.bind_text(1,&receipt_id)?;update.bind_text(2,&request.domain_id)?;update.bind_text(3,&event_id)?;update.step_done()?;
        Ok(())
    }
}

fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn hex(bytes:&[u8])->String {bytes.iter().map(|byte|format!("{byte:02x}")).collect()}
fn decode_original_hex(encoded:&str)->Result<Vec<u8>> {
    if encoded.len()%2!=0 {return Err(OrchestrationError::Invalid("host health original request hex"));}
    encoded.as_bytes().chunks_exact(2).map(|pair| {
        let value=std::str::from_utf8(pair).map_err(source_error)?;
        u8::from_str_radix(value,16).map_err(source_error)
    }).collect()
}
