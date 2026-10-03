//! Fixed CLI server requests enter through H's sealed A/process proof.
//! Model arguments select an operation only; they never select an issuer,
//! domain, request identity, filesystem path or permission.
use super::*;
use crate::store::atomic::Parser;
use crate::store::seat;
use crate::store::ledger;
use crate::store::session_transport::{codex_rpc, model_call, rpc_journal as rpc};

fn key(name:&str)->JsonString {JsonString::from_str(name)}
fn field(fields:&BTreeMap<JsonString,Json>,name:&str)->Result<String> {
    let Some(Json::String(value))=fields.get(&key(name)) else {
        return Err(OrchestrationError::Invalid("native tool string"));
    };
    value.to_well_formed_string().filter(|value|!value.is_empty()&&!value.contains('\0'))
        .ok_or(OrchestrationError::Invalid("native tool string"))
}
fn native_request(caller:&seat::NativeSeatCall)->Result<V37Request> {
    let family=match caller.tool() {
        Some("gogoke_seat")|Some("gogoke_takeover")=>"K-SEAT",
        Some("gogoke_policy")=>"K-POLICY",
        Some("gogoke_worktree")=>"K-WORKTREE",
        _=>return Err(OrchestrationError::AccessDenied),
    };
    let Some(arguments)=caller.arguments_json() else {return Err(OrchestrationError::AccessDenied)};
    let Json::Object(mut arguments)=Parser::parse(arguments)? else {
        return Err(OrchestrationError::Invalid("native tool arguments"));
    };
    if arguments.len()!=4 || ["operation","targetId","expectedRevision","payload"]
        .iter().any(|name|!arguments.contains_key(&key(name))) {
        return Err(OrchestrationError::Invalid("native tool argument fields"));
    }
    let operation=field(&arguments,"operation")?;
    if caller.tool()==Some("gogoke_takeover") && operation!="takeover-answers" {
        return Err(OrchestrationError::AccessDenied);
    }
    let expected_revision=match arguments.get(&key("expectedRevision")) {
        Some(Json::Null)=>0,
        Some(Json::String(value))=>{
            let value=value.to_well_formed_string().ok_or(OrchestrationError::Invalid("native tool revision"))?;
            let number=value.parse::<u64>().map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native tool revision: {error}")))?;
            if number.to_string()!=value || number>i64::MAX as u64 {
                return Err(OrchestrationError::Invalid("native tool revision"));
            }
            number
        }
        _=>return Err(OrchestrationError::Invalid("native tool revision")),
    };
    let Some(Json::Object(payload))=arguments.remove(&key("payload")) else {
        return Err(OrchestrationError::Invalid("native tool payload"));
    };
    Ok(V37Request {
        raw_bytes:caller.raw_request_bytes().ok_or(OrchestrationError::AccessDenied)?.to_vec(),
        family:family.into(),operation,
        request_id:caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?.into(),
        target_id:field(&arguments,"targetId")?,domain_id:caller.domain_id().into(),
        expected_revision,payload,
    })
}

impl<'root> ProductDatabase<'root> {
    pub(super) fn dispatch_captured_model_tool(&mut self,key_pair:&(String,String),
        raw:&ledger::RawSourceRecord)->Result<bool> {
        let Some(call)=codex_rpc::decode_dynamic_tool_call(&raw.raw_bytes).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native tool decode: {error:?}")))? else {return Ok(false)};
        let run=self.native_sessions.get(key_pair).ok_or(OrchestrationError::AccessDenied)?;
        let Some(turn)=run.turn_id.as_deref() else {return Ok(false)};
        let Some(thread)=run.thread_id.as_deref() else {return Ok(false)};
        let custody=run.custody.clone();
        let open_id=run.open_request_id.clone();
        let open_bytes=run.open_request_bytes.clone();
        let caller=model_call::recover_model_call_from_source(&self.connection,&custody,
            &raw.key,thread,turn).map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native tool original source: {error:?}")))?;
        let step_id=caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?;
        // A written response is final delivery. Read it before invoking an
        // effect again: its APPLIED receipt must not be replaced by REPLAYED.
        let existing=Statement::prepare(self.connection.as_ptr(),
            "SELECT phase FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND step_id=?3 AND process_operation_id=?4 AND ticket=?5 AND custodian_nonce=?6")?;
        for (index,value) in [key_pair.0.as_str(),key_pair.1.as_str(),step_id,
            raw.key.operation_id.as_str(),custody.ticket.opaque(),custody.custodian_nonce.as_str()]
            .iter().enumerate() {existing.bind_text((index+1) as i32,value)?;}
        let prior=if existing.step_row()? {
            let phase=existing.column_text(0)?;
            if existing.step_row()? {return Err(OrchestrationError::OperationConflict)};
            Some(phase)
        } else {None};
        drop(existing);
        if let Some(phase)=prior {
            if phase=="WRITTEN" {
                ledger::resolve_raw_source_no_event(&mut self.connection,&raw.key.operation_id,
                    &raw.key.source_epoch,&raw.key.source_cursor,"NATIVE_HOST_TOOL_REPLY_WRITTEN")?;
                return Ok(true);
            }
            // INTENT/UNKNOWN does not authorize another stdin write.
            return Ok(false);
        }
        let outcome=(||->Result<Vec<u8>> {
            let request=native_request(&caller)?;
            match caller.tool() {
                Some("gogoke_seat")=>self.dispatch_native_seat(&request,&caller),
                Some("gogoke_policy")=>self.dispatch_native_policy(&request,&caller),
                Some("gogoke_worktree")=>self.dispatch_native_worktree(&request,&caller),
                Some("gogoke_takeover")=>self.dispatch_native_takeover_answer(&request,&caller),
                _=>Err(OrchestrationError::AccessDenied),
            }
        })();
        let (text,success)=match outcome {
            Ok(bytes)=>{
                let receipt=crate::store::session_transport::decode_receipt(&bytes)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!("native tool receipt: {error:?}")))?;
                let success=matches!(receipt.status,V37Status::Applied|V37Status::Replayed);
                (String::from_utf8(bytes).map_err(|error|OrchestrationError::V37StoreFailure(
                    format!("native tool receipt UTF-8: {error}")))?,success)
            }
            Err(error)=>(format!("Native host operation failed: {error:?}"),false),
        };
        let command=codex_rpc::Command::DynamicToolResponse {request_id:call.request_id,text,success};
        let step=rpc::Step {domain_id:&key_pair.0,session_id:&key_pair.1,
            open_request_id:&open_id,open_request_bytes:&open_bytes,step_id,
            custody:&custody,rpc_id:None,command:&command};
        let prepared=rpc::prepare_model_tool_response(&mut self.connection,&self.owner,&caller,&step)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("native tool reply intent: {error:?}")))?;
        if prepared.disposition!=rpc::Disposition::NewWrite {return Ok(false)};
        let process=self.process_custodian.active(&custody.ticket).ok_or(OrchestrationError::AccessDenied)?;
        if let Err(error)=process.write_persistent_frame(&prepared.bytes) {
            let original=self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                crate::process::ProcessCustodyError::ProtocolPipe(error));
            let persisted=rpc::mark_unknown(&mut self.connection,&self.owner,&step,&original.to_string());
            return Err(OrchestrationError::V37StoreFailure(format!("native tool reply write: {original}; journal: {persisted:?}")));
        }
        rpc::mark_written(&mut self.connection,&self.owner,&step).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native tool reply written: {error:?}")))?;
        ledger::resolve_raw_source_no_event(&mut self.connection,&raw.key.operation_id,
            &raw.key.source_epoch,&raw.key.source_cursor,"NATIVE_HOST_TOOL_REPLY_WRITTEN")?;
        Ok(true)
    }
}
