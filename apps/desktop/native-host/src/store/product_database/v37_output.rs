//! Native provider output enters A before interpretation. Conversion and
//! source resolution use the same verified store; there is no second history.
use super::*;
use crate::store::atomic::Parser;
use crate::store::ledger::{self, EventInput, Tier};
use crate::store::session_transport::codex_output::{self, Output};
use crate::store::session_transport::provider_evidence::normalize as vendor_output;
use crate::process::{OriginBoundFrame, PreparedCustody, ProcessCustodian};

#[derive(Default)]
pub(super) struct NativeRawCapture {
    cursor: u64,
    pending: Option<OriginBoundFrame>,
    source_error: Option<String>,
    source_eof: bool,
}

#[cfg(all(test,windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new,route_b_test_guard};
    use std::time::{Duration,SystemTime,UNIX_EPOCH};

    #[test]
    fn stop_requires_clean_original_eof_not_a_generic_source_error() {
        use crate::process::ProcessCustodyError;
        for kind in [std::io::ErrorKind::WouldBlock,std::io::ErrorKind::InvalidData,std::io::ErrorKind::PermissionDenied] {
            let mut capture=NativeRawCapture::default();
            capture.record_source_error(ProcessCustodyError::ProtocolEvidence {
                cause:Box::new(ProcessCustodyError::ProtocolPipe(std::io::Error::new(kind,"original reader error"))),
                stderr_tail:"retained diagnostic".into()});
            assert!(capture.source_failed());
            assert!(!capture.source_exhausted(),"a failed reader does not prove its tail exhausted: {kind:?}");
        }
        let mut capture=NativeRawCapture::default();
        capture.record_source_error(ProcessCustodyError::ProtocolEvidence {
            cause:Box::new(ProcessCustodyError::ProtocolPipe(std::io::Error::new(std::io::ErrorKind::UnexpectedEof,"clean closed stdout"))),
            stderr_tail:String::new()});
        assert!(capture.source_exhausted());
    }

    #[test]
    fn native_raw_capture_keeps_original_frame_on_sql_failure_and_preserves_tail_at_eof() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-native-raw-capture-{}-{stamp}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("ledger.db")).unwrap();
        db.execute("CREATE TABLE orchestration_events(sequence INTEGER PRIMARY KEY,event_id TEXT UNIQUE,stream_id TEXT,occurred_at TEXT,event_type TEXT,payload_json TEXT)").unwrap();
        ledger::initialize_schema(&mut db).unwrap();
        ledger::tests::initialize_raw_h_fixture(&mut db);
        let registration=ledger::SessionRegistration {domain_id:"project-raw".into(),seat_id:"seat-raw".into(),
            session_id:"session-raw".into(),purpose:ledger::SessionPurpose::Work,side_id:None};
        ledger::register_session(&mut db,&registration).unwrap();
        let (mut custodian,custody,first)=ledger::tests::raw_process_fixture(&mut db,&registration,"operation-raw");
        let original=first.bytes().to_vec();
        let mut source=NativeRawCapture::default();
        source.retain(first).unwrap();
        db.execute("CREATE TRIGGER inject_raw_capture_failure BEFORE INSERT ON v37_ledger_raw_source BEGIN SELECT RAISE(ABORT,'injected original raw capture failure'); END").unwrap();
        assert!(source.poll_capture(&mut db,&custodian,&custody,"operation-raw").is_err());
        assert_eq!(source.cursor,0,"failed INSERT cannot advance source ordinal");
        assert_eq!(source.pending.as_ref().unwrap().bytes(),original);
        db.execute("DROP TRIGGER inject_raw_capture_failure").unwrap();
        assert!(source.poll_capture(&mut db,&custodian,&custody,"operation-raw").unwrap());
        assert!(!source.has_pending());
        let first=ledger::read_pending_raw_source(&db,"operation-raw",&custody.custodian_nonce,"1").unwrap().unwrap();
        assert_eq!(first.raw_bytes,original,"retry captured original frame, not second pipe frame");
        let second=custodian.read_persistent_child_frame(&custody.ticket,Duration::from_secs(5)).unwrap();
        let tail=second.bytes().to_vec();
        source.retain(second).unwrap();
        source.capture(&mut db,"operation-raw",&custody.custodian_nonce).unwrap();
        assert!(custodian.read_persistent_child_frame(&custody.ticket,Duration::from_secs(5)).is_err());
        assert!(!source.poll_capture(&mut db,&custodian,&custody,"operation-raw").unwrap());
        let eof=source.source_error.clone().expect("original EOF/error remains explicit");
        assert!(source.source_exhausted(),"the actual held pipe exhausted its complete frames");
        assert!(!source.poll_capture(&mut db,&custodian,&custody,"operation-raw").unwrap());
        assert_eq!(source.source_error.as_ref(),Some(&eof));
        assert_eq!(source.cursor,2);
        let last=ledger::read_pending_raw_source(&db,"operation-raw",&custody.custodian_nonce,"2").unwrap().unwrap();
        assert_eq!(last.raw_bytes,tail,"EOF does not prevent reading durable tail");
        ledger::resolve_raw_source_no_event(&mut db,"operation-raw",&custody.custodian_nonce,"2","RAW_FIXTURE_CONTROL").unwrap();
        assert!(ledger::read_pending_raw_source(&db,"operation-raw",&custody.custodian_nonce,"2").unwrap().is_none(),
            "EOF does not prevent processing durable tail; this cmd fixture is not provider/model evidence");
        custodian.stop(&custody.ticket,crate::process::StopBudgets::production(),||Ok(())).unwrap();
        drop(custodian);
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_dir_all(path).unwrap();
    }
}

impl NativeRawCapture {
    pub(super) fn has_pending(&self) -> bool { self.pending.is_some() }
    pub(super) fn source_failed(&self) -> bool { self.source_error.is_some() }
    pub(super) fn source_exhausted(&self) -> bool { self.source_eof }

    fn record_source_error(&mut self,error:crate::process::ProcessCustodyError) {
        fn is_eof(error:&crate::process::ProcessCustodyError)->bool {
            match error {
                crate::process::ProcessCustodyError::ProtocolPipe(source)=>source.kind()==std::io::ErrorKind::UnexpectedEof,
                crate::process::ProcessCustodyError::ProtocolEvidence {cause,..}=>is_eof(cause),
                _=>false,
            }
        }
        self.source_eof=is_eof(&error);
        self.source_error=Some(error.to_string());
    }

    #[cfg(test)]
    pub(super) fn install_sql_fixture_cursor(&mut self,cursor:u64) {
        assert!(self.pending.is_none() && cursor>=self.cursor);
        self.cursor=cursor;
    }

    #[cfg(test)]
    pub(super) fn model_fixture_source_error(&mut self,value:Option<String>) {
        self.source_error=value;
    }

    pub(super) fn retain(&mut self, frame: OriginBoundFrame) -> Result<()> {
        if self.pending.is_some() { return Err(OrchestrationError::OperationConflict); }
        self.pending = Some(frame);
        Ok(())
    }

    // The owned frame and its next ordinal stay here until A has durably
    // captured exactly these bytes. A failed INSERT never consumes another
    // pipe frame, increments the ordinal, or manufactures a replacement.
    pub(super) fn capture(&mut self, connection: &mut VerifiedDatabaseConnection<'_>,
        operation: &str, epoch: &str) -> Result<Option<(OriginBoundFrame, ledger::RawSourceRecord)>> {
        let Some(frame)=self.pending.as_ref() else { return Ok(None); };
        let ordinal=self.cursor.checked_add(1).filter(|value| *value<=i64::MAX as u64)
            .ok_or(OrchestrationError::Invalid("native raw cursor overflow"))?;
        let raw=ledger::capture_raw_source(connection,frame,operation,epoch,&ordinal.to_string())?;
        self.cursor=ordinal;
        let frame=self.pending.take().ok_or(OrchestrationError::Invalid("native captured frame absent"))?;
        Ok(Some((frame,raw)))
    }

    fn poll_capture(&mut self, connection: &mut VerifiedDatabaseConnection<'_>,
        custodian: &ProcessCustodian, custody: &PreparedCustody, operation: &str) -> Result<bool> {
        if !self.has_pending() {
            if self.source_error.is_some() { return Ok(false); }
            match custodian.poll_persistent_child_frame(&custody.ticket) {
                Ok(Some(frame))=>self.retain(frame)?,
                Ok(None)=>return Ok(false),
                Err(error)=>{self.record_source_error(error); return Ok(false);}
            }
        }
        self.capture(connection,operation,&custody.custodian_nonce)?;
        Ok(true)
    }
}

impl<'root> ProductDatabase<'root> {
    /// Run on the existing authority thread even when no UI is subscribed.
    /// Poll held pipes into A first, then run the Owner-authorized health and
    /// rule effects through their original E/C/H reservations. These internal
    /// safe points confer no new User or Model write authority.
    pub fn pump_native_output(&mut self) -> Result<()> {
        let keys = self.native_sessions.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            // A prior parent's control call can durably stop and remove its
            // child during this same authority-thread pass. Only remaining
            // held sessions have a pipe to drain; their errors still surface.
            if self.native_sessions.contains_key(&key) { self.drain_native_output(&key)?; }
        }
        self.pump_host_health()?;
        self.pump_host_rules()?;
        Ok(())
    }

    pub(super) fn drain_native_output(&mut self, key: &(String,String)) -> Result<()> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let custody=run.custody.clone();
        let operation=run.operation_id.clone();
        // Bound work per read; remaining frames stay in the same native pipe.
        for _ in 0..32 {
            let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
            if !run.raw_capture.poll_capture(&mut self.connection,&self.process_custodian,&custody,&operation)? {break;}
        }
        self.process_native_pending_output(key)
    }

    pub(super) fn dispatch_native_output(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        use crate::store::seat::NativeOrigin;
        use crate::store::session_transport::runtime;
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if request.payload.len()!=2 {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let generation=user_payload_string(request,"generation")?;
        let after_text=user_payload_string(request,"afterCursor")?;
        let after=after_text.parse::<u64>().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native output cursor: {error}")))?;
        if after.to_string()!=after_text || after>i64::MAX as u64 {return Err(OrchestrationError::Invalid("native output cursor"));}
        let key=(request.domain_id.clone(),request.target_id.clone());
        let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
        let seat_id=run.evidence.seat_id().to_owned();
        let operation=run.operation_id.clone();
        let nonce=run.custody.custodian_nonce.clone();
        let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,&seat_id,&key.1).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native output claim: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        let revision=u64::try_from(claim.revision).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native output revision: {error}")))?;
        if claim.generation!=generation || run.custody.binding.generation!=generation {
            return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));
        }
        if revision!=request.expected_revision {
            return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));
        }
        run.evidence.verify_live(&mut self.connection,self.root,&self.owner,&operation,claim.revision)
            .map_err(OrchestrationError::V37StoreFailure)?;
        self.drain_native_output(&key)?;
        // A pending original send may have obtained its terminal RPC receipt
        // while draining. The response reports the actual post-drain claim.
        let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,&seat_id,&key.1).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native output current claim: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if claim.generation!=generation {return Err(OrchestrationError::OperationConflict);}
        let revision=u64::try_from(claim.revision).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native output current revision: {error}")))?;
        let (card_refs,card_refs_incomplete)=self.native_card_refs(&key,&seat_id,&generation)?;
        let input_receipts=self.native_input_receipts(&key,&operation,&nonce,&generation)?;
        let input_signature=input_receipts.canonical();
        let card_refs_signature=card_refs.canonical();
        let source_error=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?
            .raw_capture.source_error.as_ref().map(|text|Json::String(JsonString::from_str(text))).unwrap_or(Json::Null);
        let position=ledger::recover(&self.connection)?;
        if after>position.cursor {return Err(OrchestrationError::OperationConflict);}
        let raw_highwater=Statement::prepare(self.connection.as_ptr(),
            "SELECT COALESCE(MAX(CAST(source_cursor AS INTEGER)),0) FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2")?;
        raw_highwater.bind_text(1,&operation)?; raw_highwater.bind_text(2,&nonce)?;
        if !raw_highwater.step_row()? {return Err(OrchestrationError::Invalid("native raw highwater"));}
        let raw_cursor=raw_highwater.column_text(0)?; drop(raw_highwater);
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_bytes,receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,&key.0)?; prior.bind_text(2,&request.request_id)?;
        if prior.step_row()? {
            let original=prior.column_text(0)?;
            if original.as_bytes()!=request.raw_bytes {
                return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));
            }
            let bytes=prior.column_text(1)?.into_bytes(); drop(prior);
            let receipt=crate::store::session_transport::decode_receipt(&bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native output receipt: {error:?}")))?;
            let result=receipt.into_result();
            let unchanged=result.get(&JsonString::from_str("ledgerHighwater"))
                .is_some_and(|value|matches!(value,Json::String(text) if text.to_well_formed_string().as_deref()==Some(position.cursor.to_string().as_str())))
                && result.get(&JsonString::from_str("rawHighwater"))
                    .is_some_and(|value|matches!(value,Json::String(text) if text.to_well_formed_string().as_deref()==Some(raw_cursor.as_str())))
                && result.get(&JsonString::from_str("sourceError")).is_some_and(|value|value.canonical()==source_error.canonical());
            let unchanged=unchanged && result.get(&JsonString::from_str("nativeInputReceipts")).is_some_and(|value|value.canonical()==input_signature)
                && result.get(&JsonString::from_str("nativeCardRefs")).is_some_and(|value|value.canonical()==card_refs_signature)
                && result.get(&JsonString::from_str("nativeCardRefsIncomplete")).is_some_and(|value|matches!(value,Json::Bool(value) if *value==card_refs_incomplete));
            return Ok(encode_receipt(request,if unchanged {V37Status::Replayed} else {V37Status::Stale},revision,revision,
                if unchanged {result} else {Default::default()}));
        }
        drop(prior);
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(|| -> Result<Vec<u8>> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let query=Statement::prepare(self.connection.as_ptr(),
                "SELECT cursor,update_json FROM main.v37_ledger_index WHERE source_kind='v37' AND domain_id=?1 AND session_id=?2 AND source_epoch=?3 AND cursor>?4 ORDER BY cursor LIMIT 3")?;
            query.bind_text(1,&key.0)?; query.bind_text(2,&key.1)?; query.bind_text(3,&nonce)?;
            query.bind_i64(4,after as i64)?;
            let mut events=Vec::new();
            let mut cursor=position.cursor;
            let mut emitted=after;
            while query.step_row()? {
                if events.len()==2 {cursor=emitted;break;}
                emitted=query.column_text(0)?.parse::<u64>().map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("native output event cursor: {error}")))?;
                events.push(Parser::parse(&query.column_text(1)?)?);
            }
            drop(query);
            let pending=Statement::prepare(self.connection.as_ptr(),
                "SELECT count(*) FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2 AND state='PENDING'")?;
            pending.bind_text(1,&operation)?;pending.bind_text(2,&nonce)?;
            if !pending.step_row()? {return Err(OrchestrationError::Invalid("native pending source count"));}
            let unresolved=pending.column_text(0)?;drop(pending);
            let result=BTreeMap::from([
                (JsonString::from_str("events"),Json::Array(events)),
                (JsonString::from_str("cursor"),Json::String(JsonString::from_str(&cursor.to_string()))),
                (JsonString::from_str("generation"),Json::String(JsonString::from_str(&generation))),
                (JsonString::from_str("ledgerHighwater"),Json::String(JsonString::from_str(&position.cursor.to_string()))),
                (JsonString::from_str("rawHighwater"),Json::String(JsonString::from_str(&raw_cursor))),
                (JsonString::from_str("unresolvedRawFrames"),Json::String(JsonString::from_str(&unresolved))),
                (JsonString::from_str("sourceError"),source_error),
                (JsonString::from_str("nativeInputReceipts"),input_receipts),
                (JsonString::from_str("nativeCardRefs"),card_refs),
                (JsonString::from_str("nativeCardRefsIncomplete"),Json::Bool(card_refs_incomplete)),
            ]);
            let bytes=encode_receipt(request,V37Status::Applied,revision,revision,result);
            if bytes.len()>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("native output receipt bound"));}
            let insert=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-SESSION',?1,?2,?3,?4)")?;
            insert.bind_text(1,&key.0)?; insert.bind_text(2,&request.request_id)?;
            insert.bind_blob(3,&request.raw_bytes)?; insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(bytes)
        })();
        match read {
            Ok(bytes)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!("native output read: {primary:?}; rollback: {rollback:?}")));
                }
                Err(primary)
            }
        }
    }

    // Read original H rows for this physical episode. This is a finite UI
    // projection, not another input journal or a vendor-text success heuristic.
    fn native_input_receipts(&self,key:&(String,String),operation:&str,nonce:&str,
        generation:&str)->Result<Json> {
        use crate::store::session_transport::{read_stdin_journal,StdinJournalKey};
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let ticket=run.custody.ticket.opaque();
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_id,phase,expected_revision FROM main.gogoke_v37_h_stdin_journal
              WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
                AND custodian_nonce=?4 AND ticket=?5 AND generation=?6
              ORDER BY CAST(expected_revision AS INTEGER) DESC,request_id LIMIT 4")?;
        for (index,value) in [key.0.as_str(),key.1.as_str(),operation,nonce,ticket,generation].iter().enumerate() {
            query.bind_text((index+1) as i32,value)?;
        }
        let mut rows=Vec::new();while query.step_row()? {
            rows.push((query.column_text(0)?,query.column_text(1)?,query.column_text(2)?));
        }
        drop(query);
        let mut facts=Vec::new();
        for (id,phase,expected) in rows {
            let record=read_stdin_journal(&self.connection,&StdinJournalKey {domain_id:&key.0,
                request_id:&id,session_id:&key.1,ticket,generation}).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native input fact: {error:?}")))?
                .ok_or(OrchestrationError::OperationConflict)?;
            if matches!(run.evidence.driver_id(),"opencode"|"grok") {
                self.verify_acp_input_receipt(&record)?;
            } else if run.evidence.driver_id()=="claude" {
                self.verify_claude_input_receipt(&record)?;
            }
            let receipt=if let Some(bytes)=record.receipt_bytes {
                Parser::parse(std::str::from_utf8(&bytes).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("native input receipt UTF-8: {error}")))?)?
            } else {Json::Null};
            facts.push(Json::Object(BTreeMap::from([
                (JsonString::from_str("requestId"),Json::String(JsonString::from_str(&id))),
                (JsonString::from_str("phase"),Json::String(JsonString::from_str(&phase))),
                (JsonString::from_str("expectedRevision"),Json::String(JsonString::from_str(&expected))),
                (JsonString::from_str("receipt"),receipt),
            ])));
        }
        Ok(Json::Array(facts))
    }

    // Output carries references to C's current unresolved cards, not a
    // second card history. Full native payloads are read through K-QCARD.
    fn native_card_refs(&self,key:&(String,String),seat:&str,generation:&str)->Result<(Json,bool)> {
        let thread=self.native_sessions.get(key).and_then(|run|run.thread_id.as_deref()).unwrap_or("");
        let cards=Statement::prepare(self.connection.as_ptr(),
            "SELECT card_id,revision,state,turn_id FROM main.gogoke_v37_qcard_native WHERE domain_id=?1 AND seat_id=?2 AND generation=?3 AND vendor_thread_id=?4 AND state IN ('OPEN','ANSWER_UNKNOWN') ORDER BY card_id LIMIT 33")?;
        for (index,value) in [key.0.as_str(),seat,generation,thread].iter().enumerate() {cards.bind_text((index+1) as i32,value)?;}
        let mut refs=Vec::new();let mut incomplete=false;
        while cards.step_row()? {
            if refs.len()==32 {incomplete=true;break;}
            refs.push(Json::Object(BTreeMap::from([
                (JsonString::from_str("cardId"),Json::String(JsonString::from_str(&cards.column_text(0)?))),
                (JsonString::from_str("revision"),Json::String(JsonString::from_str(&cards.column_text(1)?))),
                (JsonString::from_str("state"),Json::String(JsonString::from_str(&cards.column_text(2)?))),
                (JsonString::from_str("turnId"),Json::String(JsonString::from_str(&cards.column_text(3)?))),
            ])));
        }
        Ok((Json::Array(refs),incomplete))
    }

    pub(super) fn process_native_pending_output(&mut self, key: &(String,String)) -> Result<()> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let thread_id=run.thread_id.clone();
        let driver=run.evidence.driver_id().to_owned();
        let operation=run.operation_id.clone();
        let nonce=run.custody.custodian_nonce.clone();
        let ticket=run.custody.ticket.opaque().to_owned();
        let generation=run.custody.binding.generation.clone();
        let seat_id=run.evidence.seat_id().to_owned();
        let registration=ledger::read_registered_session(&self.connection,&key.1)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if registration.domain_id!=key.0 || registration.seat_id!=seat_id {
            return Err(OrchestrationError::OperationConflict);
        }
        let tier=if registration.purpose==ledger::SessionPurpose::SideChat {Tier::Side} else {Tier::Session};
        let side_id=registration.side_id;
        let replies=Statement::prepare(self.connection.as_ptr(),
            "SELECT s.step_id FROM main.gogoke_v37_rpc_steps s JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3 AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.phase='OBSERVED' AND s.requires_response=1 AND r.state='PENDING'")?;
        for (index,value) in [key.0.as_str(),key.1.as_str(),operation.as_str(),ticket.as_str(),nonce.as_str()].iter().enumerate() {
            replies.bind_text((index+1) as i32,value)?;
        }
        let mut steps=Vec::new();
        while replies.step_row()? {steps.push(replies.column_text(0)?);}
        drop(replies);
        for step in steps {
            crate::store::session_transport::rpc_journal::reconcile_observed_no_event(
                &mut self.connection,&self.owner,&key.0,&key.1,&step).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("native observed response recovery: {error:?}")))?;
        }
        if matches!(driver.as_str(),"opencode"|"grok") {
            self.complete_pending_native_acp_send(key)?;
        }
        if driver=="claude" {self.complete_pending_native_claude_send(key)?;}
        let thread_id=if driver=="claude" {
            self.native_sessions.get(key).and_then(|run|run.thread_id.clone())
        } else {thread_id};
        let Some(thread_id)=thread_id else {return Ok(());};
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_cursor FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2 AND state='PENDING' ORDER BY CAST(source_cursor AS INTEGER)")?;
        query.bind_text(1,&operation)?; query.bind_text(2,&nonce)?;
        let mut cursors=Vec::new();
        while query.step_row()? { cursors.push(query.column_text(0)?); }
        drop(query);
        for raw_cursor in cursors {
            let Some(raw)=ledger::read_pending_raw_source(&self.connection,&operation,&nonce,&raw_cursor)? else {continue;};
            if raw.process_ticket!=ticket || raw.custodian_nonce!=nonce || raw.domain_id!=key.0
                || raw.session_id!=key.1 || raw.generation!=generation {
                return Err(OrchestrationError::OperationConflict);
            }
            // A captured response without an OBSERVED association remains
            // an unresolved original outcome. It is not a notification and
            // must not block later, independently captured notifications.
            if let Json::Object(fields)=Parser::parse(std::str::from_utf8(&raw.raw_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native raw UTF-8: {error}")))?)? {
                if fields.contains_key(&JsonString::from_str("id"))
                    && !fields.contains_key(&JsonString::from_str("method"))
                    && (fields.contains_key(&JsonString::from_str("result")) || fields.contains_key(&JsonString::from_str("error"))) {
                    continue;
                }
            }
            if driver=="codex" && self.dispatch_captured_model_tool(key,&raw)? {continue;}
            if driver=="claude" {
                use crate::store::session_transport::provider_evidence::{claude_question, stream_json};
                if claude_question::decode(&raw.raw_bytes).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("original Claude question: {error:?}")))?.is_some() {
                    // The C factory verifies the original H User echo, current
                    // physical custody and host work-request identity. An
                    // unrelated permission request never becomes a question.
                    self.raise_claude_card(key,&raw.key)?;
                    ledger::resolve_raw_source_no_event(&mut self.connection,&operation,&nonce,
                        &raw_cursor,"NATIVE_CLAUDE_QUESTION_CARD")?;
                    continue;
                }
                if stream_json::is_claude_tool_result_line(&raw.raw_bytes) {
                    self.expire_claude_resolved_cards(key,&raw.key)?;
                }
            }
            let output=if driver=="codex" {
                codex_output::normalize(&raw.raw_bytes,&thread_id).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("native output: {error:?}")))?
            } else {
                let provider=match driver.as_str() {
                    "opencode"=>vendor_output::Provider::OpenCode,
                    "grok"=>vendor_output::Provider::GrokBuild,
                    "claude"=>vendor_output::Provider::Claude,
                    _=>return Err(OrchestrationError::Invalid("native output provider")),
                };
                let projected=if driver=="claude" {
                    vendor_output::claude(&raw.raw_bytes,&thread_id,&thread_id)
                } else {vendor_output::acp(&raw.raw_bytes,provider,&thread_id,&thread_id,None)}
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!(
                        "native provider output: {}; raw: {}",error.reason,String::from_utf8_lossy(&error.raw_frame))))?;
                match projected {
                    vendor_output::Output::Update(update)=>Output::Update(update),
                    // Permission/terminal/unknown data stay in original A until
                    // their H/C protocol operation has an exact source binding.
                    _=>continue,
                }
            };
            let (update,terminal_turn,started_turn)=match output {
                Output::Update(update) | Output::CompactionCompleted {update,..} => (update,None,None),
                Output::TurnStarted {update,turn_id} => (update,None,Some(turn_id)),
                Output::TurnTerminal {update,turn_id,..} => (update,Some(turn_id),None),
                Output::Question(card) => {
                    if self.native_sessions.get(key).and_then(|run|run.turn_id.as_deref())!=Some(card.turn_id.as_str()) {
                        // Questions may arrive before the matching turn/start
                        // response. Keep their original source until observed.
                        continue;
                    }
                    self.raise_codex_card(key,&raw.key)?;
                    self.reconcile_native_card_closures(key,&raw,&card)?;
                    ledger::resolve_raw_source_no_event(&mut self.connection,&operation,&nonce,&raw_cursor,"NATIVE_QUESTION_CARD")?;
                    continue;
                }
                Output::Unhandled {method} => {
                    if method=="serverRequest/resolved" {
                        self.expire_observed_native_cards(key,&raw,None)?;
                        ledger::resolve_raw_source_no_event(&mut self.connection,&operation,&nonce,&raw_cursor,"NATIVE_SERVER_REQUEST_RESOLVED")?;
                    }
                    continue;
                }
            };
            if let Some(turn_id)=terminal_turn.as_deref() {
                self.expire_observed_native_cards(key,&raw,Some(turn_id))?;
            }
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let normalized=(|| -> Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                let raw=ledger::read_pending_raw_source(&self.connection,&operation,&nonce,&raw_cursor)?
                    .ok_or(OrchestrationError::OperationConflict)?;
                let clock=Statement::prepare(self.connection.as_ptr(),
                    "SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')")?;
                if !clock.step_row()? { return Err(OrchestrationError::Invalid("native event time")); }
                let observed_at=clock.column_text(0)?;
                drop(clock);
                let stream=Statement::prepare(self.connection.as_ptr(),
                    "SELECT last_cursor FROM main.v37_ledger_source_stream WHERE session_id=?1 AND source_epoch=?2")?;
                stream.bind_text(1,&key.1)?; stream.bind_text(2,&nonce)?;
                let ordinal=if stream.step_row()? {
                    stream.column_text(0)?.parse::<u64>().map_err(|error|
                        OrchestrationError::V37StoreFailure(format!("native ledger cursor: {error}")))?
                        .checked_add(1).ok_or(OrchestrationError::Invalid("native event cursor overflow"))?
                } else { 1 };
                drop(stream);
                let mut fields=match Parser::parse(&update.update_json)? {
                    Json::Object(fields)=>fields,
                    _=>return Err(OrchestrationError::Invalid("normalized update object")),
                };
                let Some(Json::Object(meta))=fields.get_mut(&JsonString::from_str("_meta")) else {
                    return Err(OrchestrationError::Invalid("normalized update metadata"));
                };
                meta.insert(JsonString::from_str("timeBasis"),Json::String(JsonString::from_str("NATIVE_NORMALIZATION_OBSERVATION")));
                meta.insert(JsonString::from_str("rawSourceCursor"),Json::String(JsonString::from_str(&raw_cursor)));
                let source_id=format!("{}\n{}\n{}",operation,nonce,raw_cursor);
                let event_id=format!("{driver}-{}",crate::store::digest::sha256_hex(source_id.as_bytes()));
                ledger::record(&mut self.connection,&EventInput {
                    event_id:event_id.clone(),source_epoch:nonce.clone(),source_cursor:ordinal.to_string(),
                    domain_id:raw.domain_id,seat_id:seat_id.clone(),session_id:raw.session_id,
                    tier,side_id:side_id.clone(),occurred_at:observed_at,
                    update_json:Json::Object(fields).canonical(),
                })?;
                ledger::resolve_raw_source(&mut self.connection,&operation,&nonce,&raw_cursor,&event_id)?;
                self.observe_host_health_raw_in_transaction(key,&ledger::RawSourceKey {
                    operation_id:operation.clone(),source_epoch:nonce.clone(),source_cursor:raw_cursor.clone(),
                })?;
                Ok(())
            })();
            match normalized {
                Ok(())=>self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?,
                Err(primary)=>{
                    if let Err(rollback)=self.connection.execute("ROLLBACK") {
                        return Err(OrchestrationError::V37StoreFailure(format!("native output: {primary:?}; rollback: {rollback:?}")));
                    }
                    return Err(primary);
                }
            }
            if let Some(turn_id)=terminal_turn {
                let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
                if run.turn_id.as_deref()==Some(turn_id.as_str()) {run.turn_id=None;}
            }
            if let Some(turn_id)=started_turn {
                self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?.turn_id=Some(turn_id);
            }
        }
        self.reconcile_retained_terminal_turn(key)?;
        Ok(())
    }

    // The turn/start ACK can arrive after the same turn's terminal source was
    // already normalized. Re-read that original physical source instead of
    // allowing a late InProgress reply to resurrect an ended turn.
    fn reconcile_retained_terminal_turn(&mut self,key:&(String,String))->Result<()> {
        use crate::store::session_transport::codex_rpc::{self,Reply,TurnStatus};
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if run.evidence.driver_id()!="codex" {return Ok(());}
        let (Some(thread),Some(turn))=(run.thread_id.clone(),run.turn_id.clone()) else {return Ok(());};
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT r.source_cursor FROM main.v37_ledger_raw_source r
               JOIN main.v37_ledger_index i ON i.source_kind='v37'
                 AND i.source_event_id=r.resolved_event_id AND i.domain_id=r.domain_id
                 AND i.session_id=r.session_id AND i.source_epoch=r.source_epoch
              WHERE r.operation_id=?1 AND r.source_epoch=?2 AND r.domain_id=?3
                AND r.session_id=?4 AND r.process_ticket=?5 AND r.custodian_nonce=?2
                AND r.generation=?6 AND r.state='RESOLVED' AND r.no_event_reason IS NULL
                AND i.tier='SESSION' AND i.side_id IS NULL
              ORDER BY CAST(r.source_cursor AS INTEGER)")?;
        for (index,value) in [run.operation_id.as_str(),run.custody.custodian_nonce.as_str(),
            key.0.as_str(),key.1.as_str(),run.custody.ticket.opaque(),run.custody.binding.generation.as_str()]
            .iter().enumerate() {query.bind_text((index+1) as i32,value)?;}
        let mut ended=false;
        while query.step_row()? {
            let source=ledger::read_captured_raw_source(&self.connection,&run.operation_id,
                &run.custody.custodian_nonce,&query.column_text(0)?)?
                .ok_or(OrchestrationError::OperationConflict)?;
            if matches!(codex_rpc::decode(&source.raw_bytes,None).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native retained turn terminal: {error:?}")))?,
                Reply::TurnNotification {thread_id,turn_id,status,..}
                    if thread_id==thread&&turn_id==turn&&status!=TurnStatus::InProgress) {
                ended=true;
                break;
            }
        }
        drop(query);
        if ended {
            let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
            if run.turn_id.as_deref()==Some(turn.as_str()) {run.turn_id=None;}
        }
        Ok(())
    }

    // A close fact can already be terminal before a delayed question is
    // projected into C. Read the original later source facts again; never
    // reopen a card merely because the earlier expiry scan found no row.
    fn reconcile_native_card_closures(&mut self,key:&(String,String),question_source:&ledger::RawSourceRecord,
        question:&crate::store::session_transport::codex_rpc::QuestionCard) -> Result<()> {
        use crate::store::session_transport::codex_rpc::{self,Reply,RpcId,TurnStatus};
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT r.source_cursor FROM main.v37_ledger_raw_source r LEFT JOIN main.v37_ledger_index i ON i.source_kind='v37' AND i.source_event_id=r.resolved_event_id WHERE r.operation_id=?1 AND r.source_epoch=?2 AND CAST(r.source_cursor AS INTEGER)>CAST(?3 AS INTEGER) AND (r.no_event_reason='NATIVE_SERVER_REQUEST_RESOLVED' OR i.update_json LIKE '%\"codexMethod\":\"turn/completed\"%') ORDER BY CAST(r.source_cursor AS INTEGER)")?;
        query.bind_text(1,&question_source.key.operation_id)?;query.bind_text(2,&question_source.key.source_epoch)?;
        query.bind_text(3,&question_source.key.source_cursor)?;
        let mut cursors=Vec::new();while query.step_row()? {cursors.push(query.column_text(0)?);}drop(query);
        let vendor=match &question.request_id {RpcId::Number(value)=>Json::Number(value.to_string()).canonical(),
            RpcId::String(value)=>Json::String(JsonString::from_str(value)).canonical()};
        for cursor in cursors {
            let source=ledger::read_captured_raw_source(&self.connection,&question_source.key.operation_id,
                &question_source.key.source_epoch,&cursor)?.ok_or(OrchestrationError::OperationConflict)?;
            match codex_rpc::decode(&source.raw_bytes,None).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native retained card closure: {error:?}")))? {
                Reply::TurnNotification {thread_id,turn_id,status,..} if thread_id==question.thread_id
                    && turn_id==question.turn_id && status!=TurnStatus::InProgress => {
                    self.expire_observed_native_cards(key,&source,Some(&turn_id))?;
                    let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
                    if run.turn_id.as_deref()==Some(turn_id.as_str()) {run.turn_id=None;}
                }
                Reply::Event {method,..} if method=="serverRequest/resolved" => {
                    let Json::Object(fields)=Parser::parse(std::str::from_utf8(&source.raw_bytes).map_err(|error|
                        OrchestrationError::V37StoreFailure(format!("native closure UTF-8: {error}")))?)? else {return Err(OrchestrationError::Invalid("native closure object"));};
                    let Some(Json::Object(params))=fields.get(&JsonString::from_str("params")) else {return Err(OrchestrationError::Invalid("native closure params"));};
                    if params.get(&JsonString::from_str("threadId")).map(Json::canonical)==Some(Json::String(JsonString::from_str(&question.thread_id)).canonical())
                        && params.get(&JsonString::from_str("requestId")).map(Json::canonical).as_deref()==Some(vendor.as_str()) {
                        self.expire_observed_native_cards(key,&source,None)?;
                    }
                }
                _=>{}
            }
        }
        Ok(())
    }

    // A's original lifecycle notification can expire an OPEN card. It never
    // proves answer consumption and cannot overwrite ANSWER_UNKNOWN/ANSWERED.
    fn expire_observed_native_cards(&mut self,key:&(String,String),source:&ledger::RawSourceRecord,
        terminal_turn:Option<&str>) -> Result<()> {
        use crate::store::inbox::{self,CardEnvelope,InboxError};
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let thread=run.thread_id.as_deref().ok_or(OrchestrationError::AccessDenied)?;
        let seat=run.evidence.seat_id();
        if source.key.operation_id!=run.operation_id || source.process_ticket!=run.custody.ticket.opaque()
            || source.custodian_nonce!=run.custody.custodian_nonce || source.domain_id!=key.0
            || source.session_id!=key.1 || source.generation!=run.custody.binding.generation {
            return Err(OrchestrationError::OperationConflict);
        }
        let vendor_id=if terminal_turn.is_none() {
            let Json::Object(fields)=Parser::parse(std::str::from_utf8(&source.raw_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native lifecycle UTF-8: {error}")))?)? else {
                return Err(OrchestrationError::Invalid("native lifecycle object"));
            };
            let Some(Json::Object(params))=fields.get(&JsonString::from_str("params")) else {
                return Err(OrchestrationError::Invalid("native lifecycle params"));
            };
            if params.get(&JsonString::from_str("threadId")).map(Json::canonical)!=Some(Json::String(JsonString::from_str(thread)).canonical()) {
                return Err(OrchestrationError::OperationConflict);
            }
            let Some(id)=params.get(&JsonString::from_str("requestId")) else {return Err(OrchestrationError::Invalid("native lifecycle requestId"));};
            if !matches!(id,Json::Number(_)|Json::String(_)) {return Err(OrchestrationError::Invalid("native lifecycle requestId"));}
            Some(id.canonical())
        } else {None};
        let cards=Statement::prepare(self.connection.as_ptr(),
            "SELECT card_id,revision,vendor_request_id,turn_id FROM main.gogoke_v37_qcard_native WHERE domain_id=?1 AND seat_id=?2 AND generation=?3 AND vendor_thread_id=?4 AND state='OPEN'")?;
        for (index,value) in [key.0.as_str(),seat,source.generation.as_str(),thread].iter().enumerate() {cards.bind_text((index+1) as i32,value)?;}
        let mut open=Vec::new();
        while cards.step_row()? {
            let id=cards.column_text(2)?;let turn=cards.column_text(3)?;
            if vendor_id.as_deref().is_some_and(|expected|expected!=id) || terminal_turn.is_some_and(|expected|expected!=turn) {continue;}
            let revision=cards.column_text(1)?.parse::<u64>().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native card revision: {error}")))?;
            open.push((cards.column_text(0)?,revision,id,turn));
        }
        drop(cards);
        let seat=seat.to_owned();
        let descriptor=Json::Object(BTreeMap::from([
            (JsonString::from_str("operationId"),Json::String(JsonString::from_str(&source.key.operation_id))),
            (JsonString::from_str("sourceEpoch"),Json::String(JsonString::from_str(&source.key.source_epoch))),
            (JsonString::from_str("sourceCursor"),Json::String(JsonString::from_str(&source.key.source_cursor))),
            (JsonString::from_str("frameSha256"),Json::String(JsonString::from_str(&crate::store::digest::sha256_hex(&source.raw_bytes)))),
        ])).canonical();
        for (card,revision,id,turn) in open {
            let expiry=format!("expire{}",crate::store::digest::sha256_hex(format!("{}\n{}",card,descriptor).as_bytes()));
            let envelope=CardEnvelope {domain_id:&key.0,card_id:&card,request_id:&expiry,
                request_bytes:descriptor.as_bytes(),expected_revision:revision};
            inbox::expire_native_card(&mut self.connection,&envelope,&id,&seat,&turn,&source.generation,|db| {
                authority::check_owner_in_current_transaction(db,&self.owner).map_err(InboxError::Authority)?;
                let current=ledger::read_captured_raw_source(db,&source.key.operation_id,&source.key.source_epoch,&source.key.source_cursor)?;
                Ok(current.as_ref()==Some(source))
            }).map_err(|error|OrchestrationError::V37StoreFailure(format!("native observed card expiry: {error:?}")))?;
        }
        Ok(())
    }
}
