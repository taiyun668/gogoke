//! K-POLICY accepts only H's opaque, live native caller. The User wire has no
//! path to construct this value. External coordinator and delivery operations
//! remain closed until their original receipts are available here.
use super::*;
use crate::store::seat::{self, GateDecision, NativeSeatCall, SeatError};

fn key(name:&str)->JsonString {JsonString::from_str(name)}
fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn field(request:&V37Request,name:&'static str)->Result<String> {
    super::v37_seat::string_field(&request.payload,name)
}
fn gate_revision(request:&V37Request)->Result<i64> {
    i64::try_from(request.expected_revision).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("gate revision: {error}")))
}
fn result_for(event:&seat::PolicyEvent)->BTreeMap<JsonString,Json> {
    BTreeMap::from([
        (key("state"),text(&event.state)),
        (key("reason"),text(&event.detail)),
        (key("policyRevision"),text(&event.policy_revision.to_string())),
    ])
}
fn error_status(error:&SeatError)->V37Status {
    match error {
        SeatError::Invalid(_)|SeatError::Denied=>V37Status::Denied,
        SeatError::Conflict=>V37Status::Conflict,
        SeatError::Busy=>V37Status::Conflict,
        SeatError::Unknown|SeatError::Store(_)|SeatError::Open(_)|
        SeatError::CommitUnknown(_)|SeatError::RollbackUnknown(_)|
        SeatError::HostResourceObservation(_)|SeatError::HostHealthObservation(_)|
        SeatError::InstanceManagement(_)|SeatError::NativeAnswerSource(_)|SeatError::SchemaDrift=>V37Status::Unknown,
    }
}

impl<'root> ProductDatabase<'root> {
    /// Root/H calls this only after authenticating the original live H turn,
    /// including seat, generation and process custody. Request JSON is never
    /// an issuer. The current seat and policy revision are rechecked in SQLite.
    pub(crate) fn dispatch_native_policy(&mut self,request:&V37Request,
        caller:&NativeSeatCall)->Result<Vec<u8>> {
        if request.family!="K-POLICY" {return Err(OrchestrationError::Invalid("policy family"));}
        if request.domain_id!=caller.domain_id() {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        if request.operation=="call-permission-table" {
            // Payload assertions about caller or grants have no authority.
            return self.read_native_policy_table(request,caller);
        }
        if !matches!(request.operation.as_str(),"gate-submit"|"gate-decide"|"stage-transition") {
            return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let policy_revision=match seat::policy_revision_for_native_request(&self.connection,caller,
            &request.operation,&request.request_id) {
            Ok(value)=>value,
            Err(error)=>{
                let status=error_status(&error);
                let mut result=BTreeMap::new();
                if status==V37Status::Unknown {result.insert(key("reason"),text(&format!("native policy revision: {error:?}")));}
                return Ok(encode_receipt(request,status,request.expected_revision,
                    request.expected_revision,result));
            }
        };
        let event=match request.operation.as_str() {
            "gate-submit" if request.payload.is_empty() =>
                seat::gate_submit(&mut self.connection,caller,&request.target_id,policy_revision,
                    gate_revision(request)?,&request.request_id,&request.raw_bytes),
            "gate-decide" if matches!(request.payload.len(),1|2) => {
                let decision=match field(request,"decision")?.as_str() {
                    "PASS"=>GateDecision::Pass,"REJECT"=>GateDecision::Reject,
                    _=>return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                        request.expected_revision,Default::default())),
                };
                let reason=if decision==GateDecision::Reject {field(request,"reason")?}
                    else {String::new()};
                if (decision==GateDecision::Pass&&request.payload.len()!=1)||
                    (decision==GateDecision::Reject&&request.payload.len()!=2) {
                    return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                        request.expected_revision,Default::default()));
                }
                seat::gate_decide(&mut self.connection,caller,&request.target_id,decision,&reason,
                    policy_revision,gate_revision(request)?,&request.request_id,&request.raw_bytes)
            }
            "stage-transition" if request.payload.is_empty() =>
                seat::stage_transition(&mut self.connection,caller,&request.target_id,policy_revision,
                    gate_revision(request)?,&request.request_id,&request.raw_bytes),
            // M3 trigger and external delivery require the existing coordinator
            // and C/H exact receipt. An intent alone cannot be exposed as done.
            _=>return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,Default::default())),
        };
        match event {
            Ok(event)=>Ok(encode_receipt(request,if event.replayed {V37Status::Replayed}
                else {V37Status::Applied},request.expected_revision,
                request.expected_revision.checked_add(1).ok_or(OrchestrationError::Invalid("gate revision"))?,
                result_for(&event))),
            Err(error)=>{
                let status=error_status(&error);
                let mut result=BTreeMap::new();
                if status==V37Status::Unknown {result.insert(key("reason"),text(&format!("native policy store: {error:?}")));}
                Ok(encode_receipt(request,status,request.expected_revision,
                    request.expected_revision,result))
            }
        }
    }

    fn read_native_policy_table(&mut self,request:&V37Request,
        caller:&NativeSeatCall)->Result<Vec<u8>> {
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let read=(||->Result<Vec<u8>> {
            let (revision,entries)=seat::current_call_permission_table(&self.connection,caller)?;
            let revision=u64::try_from(revision).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("policy revision: {error}")))?;
            let result=BTreeMap::from([(key("entries"),Json::Array(entries.into_iter().map(|entry|
                Json::Object(BTreeMap::from([
                    (key("targetId"),text(&entry.target_id)),
                    (key("action"),text(&entry.action)),
                    (key("expiresAtMs"),match entry.expires_at_ms {
                        Some(value)=>text(&value.to_string()),None=>Json::Null,
                    }),
                ]))).collect()))]);
            let prior=Statement::prepare(self.connection.as_ptr(),
                "SELECT lower(hex(request_bytes)),receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-POLICY' AND domain_id=?1 AND request_id=?2")?;
            prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
            if prior.step_row()? {
                let raw:String=request.raw_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
                let saved_raw=prior.column_text(0)?;
                let saved_receipt=prior.column_text(1)?;
                if prior.step_row()? {return Err(OrchestrationError::OperationConflict);}
                if saved_raw!=raw {return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));}
                let saved=crate::store::session_transport::decode_receipt(saved_receipt.as_bytes())
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("native policy receipt: {error:?}")))?;
                if saved.family!="K-POLICY"||saved.operation!="call-permission-table"||
                    saved.target_id!=request.target_id {return Err(OrchestrationError::OperationConflict);}
                if saved.revision!=revision {return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));}
                let old=saved.into_result();
                let same=old.len()==result.len()&&old.iter().all(|(key,value)|
                    result.get(key).is_some_and(|current|current.canonical()==value.canonical()));
                if !same {return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));}
                return Ok(encode_receipt(request,V37Status::Replayed,revision,revision,old));
            }
            drop(prior);
            if request.expected_revision!=revision {
                return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));
            }
            let bytes=encode_receipt(request,V37Status::Applied,revision,revision,result);
            if bytes.len()>crate::ipc::MAX_FRAME_BYTES {return Err(OrchestrationError::Invalid("policy receipt bound"));}
            let insert=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes) VALUES('K-POLICY',?1,?2,?3,?4)")?;
            insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
            insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(bytes)
        })();
        match read {
            Ok(bytes)=>{self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;Ok(bytes)},
            Err(primary)=>{
                if let Err(error)=self.connection.execute("ROLLBACK") {
                    return Err(OrchestrationError::V37StoreFailure(format!("native policy read: {primary:?}; rollback: {error:?}")));
                }
                Err(primary)
            }
        }
    }
}
