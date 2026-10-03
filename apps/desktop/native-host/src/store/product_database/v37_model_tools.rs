//! Fixed CLI server requests enter through H's sealed A/process proof.
//! Model arguments select an operation only; they never select an issuer,
//! domain, request identity, filesystem path or permission.
use super::*;
use crate::store::atomic::Parser;
use crate::store::seat;
use crate::store::ledger;
use crate::store::session_transport::{codex_rpc, model_call, rpc_journal as rpc};
use crate::store::session_transport::{runtime,AdmissionRequest,AdmissionResult};

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

fn session_request(caller:&seat::NativeSeatCall,operation:&str,suffix:&str,
    session:&str,revision:u64,payload:BTreeMap<JsonString,Json>)->Result<V37Request> {
    let id=caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?;
    let frame=Json::Object(BTreeMap::from([
        (key("schema"),Json::String(JsonString::from_str("gogoke.37.operations.v1"))),
        (key("family"),Json::String(JsonString::from_str("K-SESSION"))),
        (key("operation"),Json::String(JsonString::from_str(operation))),
        (key("requestId"),Json::String(JsonString::from_str(&format!("{id}{suffix}")))),
        (key("targetId"),Json::String(JsonString::from_str(session))),
        (key("domainId"),Json::String(JsonString::from_str(caller.domain_id()))),
        (key("expectedRevision"),Json::String(JsonString::from_str(&revision.to_string()))),
        (key("payload"),Json::Object(payload)),
    ])).canonical().into_bytes();
    crate::store::session_transport::decode_request(&frame).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("native child stage request: {error:?}")))
}
fn string(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn applied_revision(result:AdmissionResult)->Result<u64> {
    match result {
        AdmissionResult::Applied(revision)|AdmissionResult::Replayed(revision)=>
            u64::try_from(revision).map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native child admission revision: {error}"))),
        value=>Err(OrchestrationError::V37StoreFailure(format!("native child admission: {value:?}"))),
    }
}

impl<'root> ProductDatabase<'root> {
    fn child_control_stage(&self,caller:&seat::NativeSeatCall,child:&seat::Seat,
        operation:&str,suffix:&str)->Result<Option<V37Request>> {
        let id=format!("{}{suffix}",caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?);
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex,operation FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2")?;
        row.bind_text(1,&child.domain_id)?;row.bind_text(2,&id)?;
        if !row.step_row()? {return Ok(None)};
        let raw=unhex_model(&row.column_text(0)?)?;
        if row.column_text(1)?!=operation || row.step_row()? {return Err(OrchestrationError::OperationConflict)};
        let stored=crate::store::session_transport::decode_request(&raw).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native child control source: {error:?}")))?;
        let generation=user_payload_string(&stored,"generation")?;
        let same_generation=if operation=="admission-release" && child.state==seat::State::Idle {
            generation.parse::<i64>().ok().and_then(|generation|generation.checked_add(1))==Some(child.generation)
        } else {generation==child.generation.to_string()};
        if stored.family!="K-SESSION" || stored.operation!=operation || stored.request_id!=id
            || stored.domain_id!=child.domain_id || stored.payload.len()!=2
            || user_payload_string(&stored,"seatId")?!=child.seat_id
            || !same_generation {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(Some(stored))
    }

    /// Control facts only: no child transcript is read or copied. A new model
    /// call stops its own bound child, then releases the proven stopped claim.
    fn dispatch_model_child_stop(&mut self,request:&V37Request,caller:&seat::NativeSeatCall)
        ->Result<Vec<u8>> {
        if !request.payload.is_empty() {return Err(OrchestrationError::Invalid("native child stop payload"))}
        let child=seat::get(&self.connection,&request.domain_id,&request.target_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        seat::current_child_dispatch_context(&self.connection,caller,&child)?;
        let release=self.child_control_stage(caller,&child,"admission-release","-release")?;
        if release.is_none() && u64::try_from(child.revision).ok()!=Some(request.expected_revision) {
            return Ok(crate::store::session_transport::encode_receipt(request,V37Status::Stale,
                request.expected_revision,u64::try_from(child.revision).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("native child seat revision: {error}")))?,Default::default()));
        }
        let row=Statement::prepare(self.connection.as_ptr(),
            "SELECT a.session_id,a.instance_id,a.home_id,a.generation,a.revision,a.state
               FROM main.gogoke_v37_h_claim a JOIN main.gogoke_v37_h_seat_binding s
                 ON s.domain_id=a.domain_id AND s.session_id=a.session_id AND s.generation=a.generation
              WHERE a.domain_id=?1 AND s.seat_id=?2 AND s.seat_incarnation=?3
                AND a.generation=?4 AND a.instance_id=?5")?;
        let child_generation=match release.as_ref() {
            Some(release)=>user_payload_string(release,"generation")?,None=>child.generation.to_string(),
        };
        for (index,value) in [child.domain_id.as_str(),child.seat_id.as_str(),child.incarnation.as_str(),
            child_generation.as_str(),child.instance_id.as_str()].iter().enumerate() {
            row.bind_text((index+1) as i32,value)?;
        }
        if !row.step_row()? {return Err(OrchestrationError::AccessDenied)};
        let session=row.column_text(0)?;let instance=row.column_text(1)?;let home=row.column_text(2)?;
        let generation=row.column_text(3)?;
        let revision=row.column_text(4)?.parse::<u64>().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native child claim revision: {error}")))?;
        let state=row.column_text(5)?;
        if row.step_row()? {return Err(OrchestrationError::OperationConflict)};
        drop(row);
        let release=if let Some(release)=release {
            if release.target_id!=session {return Err(OrchestrationError::OperationConflict)};
            release
        } else {
            let stop=self.child_control_stage(caller,&child,"stop","-stop")?;
            if state!="STOPPED" || stop.is_some() {
                let stop=match stop {Some(stop)=>stop,None=>session_request(caller,"stop","-stop",&session,revision,
                    BTreeMap::from([(key("seatId"),string(&child.seat_id)),(key("generation"),string(&generation))]))?};
                if stop.target_id!=session {return Err(OrchestrationError::OperationConflict)};
                let bytes=self.dispatch_native_child_stop(&stop,caller)?;
                let receipt=crate::store::session_transport::decode_receipt(&bytes).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("native child stop receipt: {error:?}")))?;
                if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
                    return Ok(crate::store::session_transport::encode_receipt(request,receipt.status,
                        request.expected_revision,request.expected_revision,receipt.into_result()));
                }
            }
            let stopped=runtime::observe_stop_fact(&self.connection,&request.domain_id,&session)?
                .ok_or(OrchestrationError::AccessDenied)?;
            if stopped.generation()!=generation {return Err(OrchestrationError::OperationConflict)};
            let claim=runtime::observe_claim_bound(&self.connection,&request.domain_id,&child.seat_id,&session)?
                .ok_or(OrchestrationError::AccessDenied)?;
            if claim.phase!=runtime::SessionPhase::Stopped || claim.home_id!=home || claim.instance_id!=instance
                || claim.generation!=generation || claim.process_operation_id.as_deref()!=Some(stopped.process_operation_id()) {
                return Err(OrchestrationError::OperationConflict);
            }
            let revision=u64::try_from(claim.revision).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native child stopped revision: {error}")))?;
            session_request(caller,"admission-release","-release",&session,revision,BTreeMap::from([
                (key("seatId"),string(&child.seat_id)),(key("generation"),string(&generation))]))?
        };
        let admission=seat::NativeLeadAdmission::from_model_call(caller)?;
        let origin=seat::NativeOrigin::lead(&admission);
        let input=AdmissionRequest {domain_id:&request.domain_id,session_id:&session,
            request_id:&release.request_id,raw_bytes:&release.raw_bytes,instance_id:&instance,home_id:&home,
            generation:&generation,expected_revision:i64::try_from(release.expected_revision).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native child release revision: {error}")))?};
        let released=runtime::release_native_with_origin(&mut self.connection,&self.owner,&origin,
            &child.seat_id,&input).map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native child release: {error:?}")))?;
        let status=if matches!(released,AdmissionResult::Replayed(_)) {V37Status::Replayed} else {V37Status::Applied};
        applied_revision(released)?;
        let idle=seat::get(&self.connection,&child.domain_id,&child.seat_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if idle.state!=seat::State::Idle {return Err(OrchestrationError::OperationConflict)};
        let revision=u64::try_from(idle.revision).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native child Idle revision: {error}")))?;
        Ok(crate::store::session_transport::encode_receipt(request,status,request.expected_revision,revision,
            BTreeMap::from([(key("state"),string("IDLE")),(key("sessionId"),string(&session)),
                (key("generation"),string(&idle.generation.to_string())),(key("stoppedGeneration"),string(&generation))])))
    }

    /// A local adapter operation composes only the existing E/F/H effects.
    /// Every stage retains a deterministic ID; uncertainty never starts a
    /// second child or uses a new ID to repeat an external operation.
    fn dispatch_model_child(&mut self,request:&V37Request,caller:&seat::NativeSeatCall)
        ->Result<Vec<u8>> {
        if request.payload.len()!=3 {return Err(OrchestrationError::Invalid("native dispatch payload"))}
        let repository=user_payload_string(request,"repositoryId")?;
        let layout=user_payload_string(request,"layout")?;
        let body=user_payload_string(request,"body")?;
        if !matches!(layout.as_str(),"SINGLE"|"MIXED") {return Err(OrchestrationError::Invalid("native dispatch layout"))}
        let child=seat::get(&self.connection,&request.domain_id,&request.target_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        seat::current_child_dispatch_context(&self.connection,caller,&child)?;
        let host_id=caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?;
        let session=format!("native-session-{}",&crate::store::digest::sha256_hex(
            format!("{host_id}\n{}\n{}",request.domain_id,child.seat_id).as_bytes())[..40]);
        let previous=Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 AND operation='admission-reserve'")?;
        previous.bind_text(1,&request.domain_id)?;previous.bind_text(2,host_id)?;
        let reserve=if previous.step_row()? {
            let raw=previous.column_text(0)?;
            if previous.step_row()? {return Err(OrchestrationError::OperationConflict)};
            let bytes=unhex_model(&raw)?;
            let stored=crate::store::session_transport::decode_request(&bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native child original reserve: {error:?}")))?;
            if stored.request_id!=host_id || stored.domain_id!=request.domain_id || stored.target_id!=session
                || user_payload_string(&stored,"seatId")?!=child.seat_id {
                return Err(OrchestrationError::OperationConflict);
            }
            stored
        } else {
            if child.state!=seat::State::Idle || u64::try_from(child.revision).ok()!=Some(request.expected_revision) {
                return Err(OrchestrationError::OperationConflict);
            }
            let generation=child.generation.checked_add(1).ok_or(OrchestrationError::OperationConflict)?;
            session_request(caller,"admission-reserve","",&session,0,
                BTreeMap::from([(key("seatId"),string(&child.seat_id)),
                    (key("generation"),string(&generation.to_string()))]))?
        };
        drop(previous);
        let generation=user_payload_string(&reserve,"generation")?;
        let tree_request=V37Request {raw_bytes:request.raw_bytes.clone(),family:"K-WORKTREE".into(),
            operation:"create".into(),request_id:format!("{host_id}-worktree"),
            target_id:child.seat_id.clone(),domain_id:request.domain_id.clone(),expected_revision:0,
            payload:BTreeMap::from([(key("repositoryId"),string(&repository)),(key("layout"),string(&layout))])};
        let tree=if child.state==seat::State::Busy {
            crate::store::worktree::recover_registered_native_child_worktree(&mut self.connection,
                self.root,caller,&child,&tree_request)
        } else {
            let pin=crate::store::worktree::resolve_registered_git(&mut self.connection,self.root,&self.owner,
                &repository,&mut self.process_custodian).map_err(|error|OrchestrationError::V37StoreFailure(
                    format!("native child Git pin: {error:?}")))?;
            crate::store::worktree::create_and_register_native_child_worktree(&mut self.connection,
                self.root,&self.owner,&pin,&mut self.process_custodian,caller,&child,&tree_request)
        }
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("native child worktree: {error:?}")))?;
        let (instance,home)=self.prepare_native_child_session_home(&reserve,&child.seat_id,&generation,caller)?;
        let admission=seat::NativeLeadAdmission::from_model_call(caller)?;
        let origin=seat::NativeOrigin::lead(&admission);
        let input=AdmissionRequest {domain_id:&request.domain_id,session_id:&session,request_id:host_id,
            raw_bytes:&reserve.raw_bytes,instance_id:&instance,home_id:&home,generation:&generation,
            expected_revision:0};
        let revision=applied_revision(runtime::reserve_native_with_origin(&mut self.connection,&self.owner,
            &origin,&child.seat_id,&input).map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native child reserve: {error:?}")))?)?;
        let commit=session_request(caller,"admission-commit","-commit",&session,revision,
            BTreeMap::from([(key("seatId"),string(&child.seat_id)),(key("generation"),string(&generation))]))?;
        let input=AdmissionRequest {request_id:&commit.request_id,raw_bytes:&commit.raw_bytes,
            expected_revision:i64::try_from(revision).map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native child commit revision: {error}")))?,..input};
        let revision=applied_revision(runtime::commit_native_with_origin(&mut self.connection,&self.owner,
            &origin,&child.seat_id,&input).map_err(|error|OrchestrationError::V37StoreFailure(
                format!("native child commit: {error:?}")))?)?;
        let open=session_request(caller,"open","-open",&session,revision,BTreeMap::from([
            (key("seatId"),string(&child.seat_id)),(key("generation"),string(&generation)),
            (key("repositoryId"),string(&repository)),(key("worktreeId"),string(&tree.worktree_id))]))?;
        let opened=self.dispatch_native_child_open(&open,caller)?;
        let receipt=crate::store::session_transport::decode_receipt(&opened).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native child open receipt: {error:?}")))?;
        if !matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {return Ok(opened)};
        let send=session_request(caller,"send","-send",&session,receipt.revision,
            BTreeMap::from([(key("generation"),string(&generation)),(key("body"),string(&body))]))?;
        self.dispatch_native_send(&send)
    }

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
        let caller=match model_call::recover_model_call_from_source(&self.connection,&custody,
            &raw.key,thread,turn) {
            Ok(caller)=>caller,
            // A server call can precede its turn/start ACK. The captured
            // source grants no effect until the existing H turn proof is
            // complete. Leave these exact bytes in A, without another write.
            Err(model_call::ModelCallError::Denied)=>return Ok(false),
            Err(error)=>return Err(OrchestrationError::V37StoreFailure(
                format!("native tool original source: {error:?}"))),
        };
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
        // Native duplex protocols may deliver a tool call while a preceding
        // steer/append RPC is still waiting for its ACK. Capture the call in
        // A now; complete that original RPC before admitting any tool effect.
        let outstanding=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3 AND ticket=?4 AND custodian_nonce=?5 AND (phase IN ('INTENT','UNKNOWN') OR (phase='WRITTEN' AND requires_response=1)) LIMIT 1")?;
        for (index,value) in [key_pair.0.as_str(),key_pair.1.as_str(),raw.key.operation_id.as_str(),
            custody.ticket.opaque(),custody.custodian_nonce.as_str()].iter().enumerate() {
            outstanding.bind_text((index+1) as i32,value)?;
        }
        let wait=outstanding.step_row()?;drop(outstanding);
        if wait {return Ok(false)};
        let outcome=(||->Result<Vec<u8>> {
            let request=native_request(&caller)?;
            match caller.tool() {
                Some("gogoke_seat") if request.operation=="dispatch"=>self.dispatch_model_child(&request,&caller),
                Some("gogoke_seat") if request.operation=="stop"=>self.dispatch_model_child_stop(&request,&caller),
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

fn unhex_model(value:&str)->Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(OrchestrationError::Invalid("native original reserve hex"))}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let value=std::str::from_utf8(pair).map_err(|error|OrchestrationError::V37StoreFailure(
            format!("native original reserve hex UTF-8: {error}")))?;
        u8::from_str_radix(value,16).map_err(|error|OrchestrationError::V37StoreFailure(
            format!("native original reserve hex: {error}")))
    }).collect()
}
