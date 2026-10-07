//! Host health uses sealed terminal H/A facts and the existing generation
//! journal. It has no model permissions, new scheduler or work-input resend.
use super::*;
use crate::store::atomic::Parser;
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
        // This classifier belongs to the retained fixed Codex transport.
        // Normal Claude/ACP output must never enter a Codex RPC decoder.
        if run.evidence.driver_id()!="codex" {return Ok(());}
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
        self.revisit_host_health_sources()?;
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
                let outcome=self.dispatch_native_generation_change_with_health(&request,Some((&proof,&event_id)))?;
                self.persist_terminal_host_health_receipt(&request,&outcome)?;
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
                self.persist_terminal_host_health_receipt(&request,&outcome)?;
            }
        }
        Ok(())
    }

    fn persist_terminal_host_health_receipt(&mut self,request:&V37Request,bytes:&[u8])->Result<()> {
        let receipt=h::decode_receipt(bytes).map_err(source_error)?;
        if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed|V37Status::Unsupported) {return Ok(());}
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let settled=self.settle_host_health_change_in_transaction(request,bytes);
        self.finish_host_health_transaction(settled)
    }

    /// Read original H/A facts with the retained physical witness. The pump
    /// records E's composite observation; the Owner projection only reads it.
    pub(super) fn current_stalled_health_in_transaction(&mut self,record:bool)
        ->Result<Vec<host_health::StalledHealthProof>> {
        authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.event_id,r.domain_id,r.session_id,r.operation_id,r.source_epoch,r.source_cursor
              FROM main.gogoke_v37_seat_health e JOIN main.v37_ledger_raw_source r
                ON r.domain_id=e.domain_id AND r.resolved_event_id=e.source_event_id
              JOIN main.gogoke_v37_h_generation_change c
                ON c.domain_id=e.domain_id AND c.request_id=e.session_request_id
              WHERE e.state='RECEIPTED' AND e.action IN ('COMPACT','RENEW') AND c.stage='UNSUPPORTED'
              ORDER BY e.domain_id,e.event_id")?;
        let mut sources=Vec::new();
        while q.step_row()? {sources.push((q.column_text(0)?,(q.column_text(1)?,q.column_text(2)?),
            RawSourceKey {operation_id:q.column_text(3)?,source_epoch:q.column_text(4)?,source_cursor:q.column_text(5)?}));}
        drop(q);
        let mut proofs=Vec::new();
        for (event,key,source) in sources {
            let Some(run)=self.native_sessions.get(&key) else {continue;};
            if run.evidence.driver_id()!="codex" || !run.allows_input() || run.turn_id.is_some() {continue;}
            let proof=match host_health::observe_stalled_host_health_in_transaction(&self.connection,
                &self.owner,&run.custody,&source,&event) {
                Ok(Some(proof))=>proof,Ok(None)=>continue,
                Err(host_health::HostHealthError::Denied|host_health::HostHealthError::Conflict
                    |host_health::HostHealthError::Rpc(h::rpc_journal::RpcJournalError::Denied))=>continue,
                Err(error)=>return Err(source_error(error)),
            };
            if record {seat::observe_stalled_host_health_in_transaction(&mut self.connection,&self.owner,&proof)?;}
            proofs.push(proof);
        }
        Ok(proofs)
    }

    /// A terminal notification can be normalized before its ordinary send
    /// receipt commits. Its original resolved A source remains authoritative;
    /// revisit it after the drain, with the same physical custody and seal.
    /// This creates no new source, RPC ID, writer or scheduler.
    pub(super) fn revisit_host_health_sources(&mut self)->Result<()> {
        let held=self.native_sessions.iter().filter(|(_,run)|run.evidence.driver_id()=="codex")
            .map(|(key,run)|(key.clone(),run.operation_id.clone(),run.custody.custodian_nonce.clone()))
            .collect::<Vec<_>>();
        for (key,operation,epoch) in held {
            let query=Statement::prepare(self.connection.as_ptr(),
                "SELECT r.source_cursor,i.update_json FROM main.v37_ledger_raw_source r
                   JOIN main.v37_ledger_index i ON i.source_kind='v37'
                     AND i.source_event_id=r.resolved_event_id AND i.domain_id=r.domain_id
                     AND i.session_id=r.session_id AND i.source_epoch=r.source_epoch
                  WHERE r.operation_id=?1 AND r.source_epoch=?2 AND r.domain_id=?3
                    AND r.session_id=?4 AND r.state='RESOLVED' AND r.no_event_reason IS NULL
                    AND NOT EXISTS (SELECT 1 FROM main.gogoke_v37_seat_health e
                      WHERE e.domain_id=r.domain_id AND e.source_event_id=r.resolved_event_id)
                  ORDER BY CAST(r.source_cursor AS INTEGER)")?;
            for (index,value) in [operation.as_str(),epoch.as_str(),key.0.as_str(),key.1.as_str()]
                .iter().enumerate() {query.bind_text((index+1) as i32,value)?;}
            let mut sources=Vec::new();
            while query.step_row()? {
                let Json::Object(update)=Parser::parse(&query.column_text(1)?)? else {
                    return Err(OrchestrationError::Invalid("host health normalized update"));
                };
                let Some(Json::Object(meta))=update.get(&JsonString::from_str("_meta")) else {
                    return Err(OrchestrationError::Invalid("host health normalized metadata"));
                };
                let matches=|name:&str,value:&str|matches!(meta.get(&JsonString::from_str(name)),
                    Some(Json::String(actual)) if actual.to_well_formed_string().as_deref()==Some(value));
                if matches("provider","codex")&&matches("codexMethod","turn/completed")&&matches("turnStatus","failed") {
                    sources.push(RawSourceKey {operation_id:operation.clone(),source_epoch:epoch.clone(),
                        source_cursor:query.column_text(0)?});
                }
            }
            drop(query);
            for source in sources {
                self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
                let observed=self.observe_host_health_raw_in_transaction(&key,&source);
                self.finish_host_health_transaction(observed)?;
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
        let event_id=row.column_text(0)?;
        let action=row.column_text(2)?;let state=row.column_text(3)?;let saved=row.column_text(4)?;
        if row.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(row);
        let receipt=h::decode_receipt(bytes).map_err(source_error)?;
        let c=change::read(&self.connection,&request.domain_id,&request.request_id).map_err(source_error)?
            .ok_or(OrchestrationError::OperationConflict)?;
        // The health row is indexed by E authorization; the frozen original
        // K-SESSION request and H change journal identify its physical process.
        let old_generation=match request.payload.get(&JsonString::from_str("generation")) {
            Some(Json::String(value))=>value.to_well_formed_string(),_=>None,
        }.ok_or(OrchestrationError::Invalid("host health physical generation"))?;
        if receipt.status==V37Status::Unsupported {
            if c.stage!="UNSUPPORTED" || c.raw_hex!=hex(&request.raw_bytes)
                || c.session_id!=request.target_id || c.old_generation!=old_generation
                || c.operation!=request.operation || c.owner_stop_request_id.is_some()
                || c.unknown_revision.is_some() || receipt.family!="K-SESSION"
                || receipt.operation!=request.operation || receipt.request_id!=request.request_id
                || receipt.target_id!=request.target_id || receipt.previous_revision!=request.expected_revision
                || receipt.revision!=request.expected_revision || !receipt.into_result().is_empty()
                || !matches!((action.as_str(),request.operation.as_str()),("COMPACT","compact")|("RENEW","renew-session")) {
                return Err(OrchestrationError::OperationConflict);
            }
            let step=format!("compact-{}",&crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
            if request.operation!="compact" || c.ack_step_id.as_deref()!=Some(step.as_str()) {
                return Err(OrchestrationError::OperationConflict);
            }
            let response=match h::rpc_journal::observed_compact_ack(&self.connection,
                &request.domain_id,&request.target_id,&c.old_operation,&c.old_generation,
                &c.old_ticket,&c.old_nonce,&step,&c.thread_id) {
                Err(h::rpc_journal::RpcJournalError::Codec(h::codex_rpc::RpcError::RemoteResponse(raw)))=>raw,
                Ok(_)=>return Err(OrchestrationError::OperationConflict),
                Err(error)=>return Err(source_error(error)),
            };
            let Json::Object(root)=Parser::parse(std::str::from_utf8(&response).map_err(source_error)?.trim_end_matches('\n'))? else {
                return Err(OrchestrationError::OperationConflict);
            };
            if !matches!(root.get(&JsonString::from_str("error")),Some(Json::Object(error))
                if matches!(error.get(&JsonString::from_str("code")),Some(Json::Number(code)) if code=="-32601")) {
                return Err(OrchestrationError::OperationConflict);
            }
            return self.save_host_health_receipt_in_transaction(request,bytes,&event_id,&state,&saved);
        }
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
        self.save_host_health_receipt_in_transaction(request,bytes,&event_id,&state,&saved)
    }

    fn save_host_health_receipt_in_transaction(&mut self,request:&V37Request,bytes:&[u8],
        event_id:&str,state:&str,saved:&str)->Result<()> {
        let receipt_id=format!("health-receipt-{}",crate::store::digest::sha256_hex(bytes));
        if state=="RECEIPTED" {
            // A replay may encode REPLAYED rather than APPLIED; retain the
            // original bytes already committed by H, never replace them.
            let q=Statement::prepare(self.connection.as_ptr(),
                "SELECT hex(request_bytes),hex(receipt_bytes) FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
            q.bind_text(1,&request.domain_id)?;q.bind_text(2,&request.request_id)?;
            if !q.step_row()? || q.column_text(0)?.to_lowercase()!=hex(&request.raw_bytes) {
                return Err(OrchestrationError::OperationConflict);
            }
            let original=decode_original_hex(&q.column_text(1)?)?;
            if saved!=format!("health-receipt-{}",crate::store::digest::sha256_hex(&original)) || q.step_row()? {
                return Err(OrchestrationError::OperationConflict);
            }
            return Ok(());
        }
        if state!="REQUESTED" {return Err(OrchestrationError::OperationConflict);}
        let insert=Statement::prepare(self.connection.as_ptr(),
            "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-SESSION',?1,?2,?3,?4)")?;
        insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
        insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,bytes)?;insert.step_done()?;drop(insert);
        let update=Statement::prepare(self.connection.as_ptr(),
            "UPDATE main.gogoke_v37_seat_health SET state='RECEIPTED',receipt_id=?1 WHERE domain_id=?2 AND event_id=?3 AND state='REQUESTED'")?;
        update.bind_text(1,&receipt_id)?;update.bind_text(2,&request.domain_id)?;update.bind_text(3,event_id)?;update.step_done()?;
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
