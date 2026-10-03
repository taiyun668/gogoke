//! Actual native provider sessions. The User channel selects logical identities;
//! E/F/H supply all paths, pins, permissions and custody on the same store.
use super::*;
use crate::process::{PreparedCustody, DurableStopConfirmation, StopBudgets, NativeStopProof, OriginBoundFrame};
use crate::store::ledger::{self, SessionPurpose, SessionRegistration};
use crate::store::seat::{self, NativeOrigin};
use crate::store::session_transport::{self as h, runtime, launch::LaunchEvidence,
    codex_rpc::{self, Command, RpcId, Reply}, rpc_journal as rpc,
    generation_change as change, provider_evidence::{acp, stream_json, commands as vendor_commands}};
use crate::store::atomic::Parser;
use std::time::{Duration, Instant};

pub(super) struct NativeSession {
    pub(super) evidence: LaunchEvidence,
    pub(super) custody: PreparedCustody,
    pub(super) operation_id: String,
    pub(super) open_request_id: String,
    pub(super) open_request_bytes: Vec<u8>,
    domain_id: String,
    session_id: String,
    model: String,
    effort: String,
    pub(super) thread_id: Option<String>,
    pub(super) turn_id: Option<String>,
    pub(super) raw_capture: super::v37_output::NativeRawCapture,
    stop_proof: Option<NativeStopProof>,
    next_rpc_id: u64,
    pending_acp: Option<(Vec<u8>,h::AcpSendIdentity)>,
    pending_claude: Option<(Vec<u8>,h::ClaudeSendIdentity)>,
}

struct RpcObservation {
    reply: Reply,
    frame: OriginBoundFrame,
}

impl NativeSession {
    pub(super) fn allows_input(&self) -> bool {
        self.stop_proof.is_none() && !self.raw_capture.has_pending() && !self.raw_capture.source_failed()
    }
}

fn failure<T, E: std::fmt::Debug>(result: std::result::Result<T, E>) -> Result<T> {
    result.map_err(|error| OrchestrationError::V37StoreFailure(format!("native session: {error:?}")))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn text(value:&str)->Json {Json::String(JsonString::from_str(value))}
fn compact_method_missing(frame:&[u8])->bool {
    let Some(body)=frame.strip_suffix(b"\n") else {return false;};
    let Ok(text)=std::str::from_utf8(body) else {return false;};
    let Ok(Json::Object(fields))=Parser::parse(text) else {return false;};
    let Some(Json::Object(error))=fields.get(&JsonString::from_str("error")) else {return false;};
    matches!(error.get(&JsonString::from_str("code")),Some(Json::Number(code)) if code=="-32601")
}
fn unhex(bytes: &str) -> Result<Vec<u8>> {
    if bytes.len()%2!=0 {return Err(OrchestrationError::Invalid("native stored hex"));}
    bytes.as_bytes().chunks_exact(2).map(|pair| {
        let value=std::str::from_utf8(pair).map_err(|_|OrchestrationError::Invalid("native stored hex"))?;
        u8::from_str_radix(value,16).map_err(|_|OrchestrationError::Invalid("native stored hex"))
    }).collect()
}

impl<'root> ProductDatabase<'root> {
    fn original_native_continuation(&self, domain: &str, session: &str)
        -> Result<(String,String,String)> {
        let initial=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.process_operation_id,e.raw_hex,e.generation
               FROM main.gogoke_v37_h_process_episode e
               JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id
                 AND g.session_id=e.session_id AND g.generation=e.generation
                 AND g.process_operation_id=e.process_operation_id
              WHERE e.domain_id=?1 AND e.session_id=?2
                AND e.old_generation IS NULL AND e.process_operation_id IS NOT NULL")?;
        initial.bind_text(1,domain)?;initial.bind_text(2,session)?;
        if !initial.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let operation=initial.column_text(0)?;
        let raw=unhex(&initial.column_text(1)?)?;
        let generation=initial.column_text(2)?;
        if initial.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let original=h::decode_request(&raw).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native original open: {error:?}")))?;
        if original.family!="K-SESSION" || original.operation!="open"
            || original.domain_id!=domain || original.target_id!=session {
            return Err(OrchestrationError::OperationConflict);
        }
        let repository=user_payload_string(&original,"repositoryId")?;
        let worktree=user_payload_string(&original,"worktreeId")?;
        let observed=Statement::prepare(self.connection.as_ptr(),
            "SELECT s.ticket,s.custodian_nonce
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id
                 AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.generation=?3
                AND s.process_operation_id=?4 AND s.step_id='thread-start'
                AND s.phase='OBSERVED'")?;
        for (index,value) in [domain,session,generation.as_str(),operation.as_str()].iter().enumerate() {
            observed.bind_text((index+1) as i32,value)?;
        }
        if !observed.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let ticket=observed.column_text(0)?;
        let nonce=observed.column_text(1)?;
        if observed.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let thread=failure(rpc::observed_thread_id(&self.connection,domain,session,&operation,
            &generation,&original.request_id,&ticket,&nonce))?;
        Ok((repository,worktree,thread))
    }

    fn observed_resume_thread(&self,domain:&str,session:&str,operation:&str,
        generation:&str,request_id:&str)->Result<Option<String>> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT c.ticket,c.custodian_nonce FROM main.gogoke_coordination_process_custody c
              JOIN main.gogoke_v37_rpc_steps s ON s.process_operation_id=c.operation_id
                AND s.domain_id=c.domain_id AND s.generation=c.generation
                AND s.ticket=c.ticket AND s.custodian_nonce=c.custodian_nonce
              WHERE c.operation_id=?1 AND c.domain_id=?2 AND c.generation=?3
                AND s.session_id=?4 AND s.open_request_id=?5 AND s.phase='OBSERVED'
                AND s.step_id IN (?6,?7)")?;
        q.bind_text(1,operation)?;q.bind_text(2,domain)?;q.bind_text(3,generation)?;
        q.bind_text(4,session)?;q.bind_text(5,request_id)?;
        q.bind_text(6,&format!("{operation}-thread-resume"))?;
        q.bind_text(7,&format!("{operation}-session-resume"))?;
        if !q.step_row()? {return Ok(None);}
        let ticket=q.column_text(0)?;let nonce=q.column_text(1)?;
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(q);
        failure(rpc::observed_thread_id(&self.connection,domain,session,operation,generation,
            request_id,&ticket,&nonce)).map(Some)
    }

    pub(super) fn dispatch_native_resume(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        self.dispatch_native_resume_at(request,request.expected_revision)
    }

    // The same original compact/renew request may continue after its single
    // UNKNOWN revision. This is an internal revision, never a rewritten wire
    // request or a second authority to launch.
    fn dispatch_native_resume_at(&mut self, request:&V37Request,
        effective_revision:u64) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if let Some(refusal)=self.formal_review_continuation_refusal(request)? {
            return Ok(refusal);
        }
        if request.payload.len()!=1 {
            return Ok(encode_receipt(request,V37Status::Denied,effective_revision,
                effective_revision,Default::default()));
        }
        let old_generation=user_payload_string(request,"generation")?;
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex,session_id,old_generation,generation,phase,
                    previous_revision,COALESCE(result_revision,previous_revision),
                    COALESCE(process_operation_id,'')
               FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND request_id=?2 AND old_generation IS NOT NULL")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        if prior.step_row()? {
            let same=prior.column_text(0)?==hex(&request.raw_bytes)
                && prior.column_text(1)?==request.target_id
                && prior.column_text(2)?==old_generation;
            let new_generation=prior.column_text(3)?;
            let phase=prior.column_text(4)?;
            let previous=prior.column_text(5)?.parse::<u64>().map_err(|_|
                OrchestrationError::Invalid("native resume prior revision"))?;
            let revision=prior.column_text(6)?.parse::<u64>().map_err(|_|
                OrchestrationError::Invalid("native resume result revision"))?;
            let operation=prior.column_text(7)?;
            if prior.step_row()? {return Err(OrchestrationError::OperationConflict);}
            drop(prior);
            if !same {return Ok(encode_receipt(request,V37Status::Conflict,
                effective_revision,effective_revision,Default::default()));}
            if phase=="ACTIVE" || phase=="STOPPED" {
                let receipt_id=self.resume_source_receipt(&request.domain_id,&request.target_id,
                    &operation,&new_generation,&request.request_id)?;
                return Ok(encode_receipt(request,V37Status::Replayed,previous,revision,
                    BTreeMap::from([
                        (JsonString::from_str("state"),text("RUNNING")),
                        (JsonString::from_str("oldGeneration"),text(&old_generation)),
                        (JsonString::from_str("newGeneration"),text(&new_generation)),
                        (JsonString::from_str("receiptId"),text(&receipt_id))])));
            }
            if !operation.is_empty() && matches!(phase.as_str(),"PREPARED"|"UNKNOWN") {
                let key=(request.domain_id.clone(),request.target_id.clone());
                if let Some(run)=self.native_sessions.get(&key) {
                    if run.operation_id==operation
                        && self.process_custodian.active(&run.custody.ticket).is_some() {
                        // A captured response may have survived an H commit
                        // fault with its original RPC step still WRITTEN.
                        // Resolve that exact source; never resend the command.
                        failure(rpc::reconcile_written_resume_from_a(&mut self.connection,
                            &self.owner,&request.domain_id,&request.target_id,
                            &request.request_id,&request.raw_bytes,&operation,&new_generation))?;
                        let expected=self.original_native_continuation(&request.domain_id,
                            &request.target_id)?.2;
                        if self.observed_resume_thread(&request.domain_id,&request.target_id,
                            &operation,&new_generation,&request.request_id)?
                            .as_deref()==Some(expected.as_str()) {
                            let current=failure(runtime::observe_claim(&self.connection,
                                &NativeOrigin::user(&self.owner),&request.domain_id,
                                run.evidence.seat_id(),&request.target_id))?
                                .ok_or(OrchestrationError::AccessDenied)?;
                            failure(self.connection.execute("BEGIN IMMEDIATE"))?;
                            let promoted=(|| -> Result<i64> {
                                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                                failure(h::promote_resume(&self.connection,&request.domain_id,
                                    &request.target_id,&request.request_id,&operation,current.revision))
                            })();
                            let next=match promoted {
                                Ok(next)=>{self.finish_native_transaction(Ok(()))?;next},
                                Err(error)=>{self.finish_native_transaction(Err(error))?;unreachable!()}
                            };
                            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
                                .thread_id=Some(expected);
                            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
                                .evidence.adopt_resume(&self.connection,&self.owner,&operation)
                                .map_err(OrchestrationError::V37StoreFailure)?;
                            self.process_native_pending_output(&key)?;
                            let receipt_id=self.resume_source_receipt(&request.domain_id,
                                &request.target_id,&operation,&new_generation,&request.request_id)?;
                            return Ok(encode_receipt(request,V37Status::Replayed,revision,
                                u64::try_from(next).map_err(|_|OrchestrationError::OperationConflict)?,
                                BTreeMap::from([
                                    (JsonString::from_str("state"),text("RUNNING")),
                                    (JsonString::from_str("oldGeneration"),text(&old_generation)),
                                    (JsonString::from_str("newGeneration"),text(&new_generation)),
                                    (JsonString::from_str("receiptId"),text(&receipt_id))])));
                        }
                    }
                }
            }
            return Ok(encode_receipt(request,V37Status::Unknown,previous,revision,
                BTreeMap::from([
                    (JsonString::from_str("state"),text("RESUME_UNKNOWN")),
                    (JsonString::from_str("oldGeneration"),text(&old_generation))])));
        }
        drop(prior);
        let pending=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND session_id=?2 AND old_generation IS NOT NULL
                AND phase IN ('INTENT','PREPARED','UNKNOWN') LIMIT 1")?;
        pending.bind_text(1,&request.domain_id)?;pending.bind_text(2,&request.target_id)?;
        if pending.step_row()? {
            return Ok(encode_receipt(request,V37Status::Conflict,effective_revision,
                effective_revision,Default::default()));
        }
        drop(pending);
        if self.native_sessions.contains_key(&(request.domain_id.clone(),request.target_id.clone())) {
            return Ok(encode_receipt(request,V37Status::Conflict,effective_revision,
                effective_revision,Default::default()));
        }
        let binding=Statement::prepare(self.connection.as_ptr(),
            "SELECT seat_id FROM main.gogoke_v37_h_seat_binding
              WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
        binding.bind_text(1,&request.domain_id)?;
        binding.bind_text(2,&request.target_id)?;
        binding.bind_text(3,&old_generation)?;
        if !binding.step_row()? {
            return Ok(encode_receipt(request,V37Status::Conflict,effective_revision,
                effective_revision,Default::default()));
        }
        let seat_id=binding.column_text(0)?;
        if binding.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(binding);
        let Some(old)=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &request.domain_id,&seat_id,&request.target_id))? else {
            return Ok(encode_receipt(request,V37Status::Conflict,effective_revision,
                effective_revision,Default::default()));
        };
        if old.generation!=old_generation || old.phase!=runtime::SessionPhase::Stopped {
            return Ok(encode_receipt(request,V37Status::Conflict,effective_revision,
                effective_revision,Default::default()));
        }
        if u64::try_from(old.revision).ok()!=Some(effective_revision) {
            return Ok(encode_receipt(request,V37Status::Stale,effective_revision,
                u64::try_from(old.revision).map_err(|_|OrchestrationError::OperationConflict)?,
                Default::default()));
        }
        if runtime::observe_stop_fact(&self.connection,&request.domain_id,&request.target_id)?
            .is_none() {
            return Ok(encode_receipt(request,V37Status::Unknown,effective_revision,
                effective_revision,Default::default()));
        }
        // A declared unsupported provider must not acquire a candidate home,
        // episode or process while trying the Codex/ACP recovery path.
        let driver=Statement::prepare(self.connection.as_ptr(),
            "SELECT driver_id FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
        driver.bind_text(1,&old.instance_id)?;
        if !driver.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let driver_id=driver.column_text(0)?;
        if driver.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(driver);
        if !matches!(driver_id.as_str(),"codex"|"opencode"|"grok") {
            return Ok(encode_receipt(request,V37Status::Unsupported,effective_revision,
                effective_revision,BTreeMap::from([(JsonString::from_str("reason"),
                    text("This fixed provider has no integrated native metadata resume"))])));
        }
        let old_number=old_generation.parse::<i64>().map_err(|_|
            OrchestrationError::Invalid("native resume old generation"))?;
        let new_generation=old_number.checked_add(1).ok_or(
            OrchestrationError::Invalid("native resume generation overflow"))?.to_string();
        let (repository_id,worktree_id,thread_id)=self.original_native_continuation(
            &request.domain_id,&request.target_id)?;
        let (instance_id,home_id)=self.prepare_resume_session_home(request,&seat_id,&new_generation)?;
        if instance_id!=old.instance_id {return Err(OrchestrationError::OperationConflict);}
        let owner_binding=Statement::prepare(self.connection.as_ptr(),
            "SELECT binding_id FROM main.gogoke_v37_h_owner_binding
              WHERE instance_id=?1 AND domain_id=?2 AND kind='SESSION'
                AND owner_id=?3 AND generation=?4 AND state='ACTIVE'")?;
        for (index,value) in [instance_id.as_str(),request.domain_id.as_str(),
            request.target_id.as_str(),new_generation.as_str()].iter().enumerate() {
            owner_binding.bind_text((index+1) as i32,value)?;
        }
        if !owner_binding.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let binding_id=owner_binding.column_text(0)?;
        if owner_binding.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(owner_binding);
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let intended=(|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            failure(h::begin_resume(&self.connection,&request.domain_id,&request.target_id,
                &request.request_id,&request.raw_bytes,&old_generation,&new_generation,
                old.revision,&home_id,&binding_id))?;
            Ok(())
        })();
        self.finish_native_transaction(intended)?;
        let evidence=failure(LaunchEvidence::observe_resume(&mut self.connection,self.root,
            &self.owner,&request.domain_id,&seat_id,&request.target_id,&repository_id,
            &worktree_id,&request.request_id))?;
        let (model,effort)=failure(evidence.settings())?;
        let launch=failure(evidence.request())?;
        let custody=self.process_custodian.prepare(&launch)?;
        let key=(request.domain_id.clone(),request.target_id.clone());
        let digest=crate::store::digest::sha256_hex(&request.raw_bytes);
        let operation_id=format!("h-resume-{}",&digest[..40]);
        self.native_sessions.insert(key.clone(),NativeSession {
            evidence,custody:custody.clone(),operation_id:operation_id.clone(),
            open_request_id:request.request_id.clone(),open_request_bytes:request.raw_bytes.clone(),
            domain_id:request.domain_id.clone(),session_id:request.target_id.clone(),
            model,effort,thread_id:None,turn_id:None,raw_capture:Default::default(),
            stop_proof:None,next_rpc_id:4,pending_acp:None,pending_claude:None,
        });
        authority::record_prepared_process(&mut self.connection,&operation_id,&custody)?;
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let attached=(|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify_in_transaction(&mut self.connection,self.root,
                &self.owner,None))?;
            failure(h::attach_resume_process(&self.connection,&request.domain_id,
                &request.request_id,&operation_id))?;
            Ok(())
        })();
        self.finish_native_transaction(attached)?;
        let started=(|| -> Result<()> {
            let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify(&mut self.connection,self.root,&self.owner,
                Some(&operation_id)))?;
            self.process_custodian.activate(&custody)?;
            authority::mark_process_active(&mut self.connection,&operation_id,&custody)?;
            let driver=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?
                .evidence.driver_id().to_owned();
            if matches!(driver.as_str(),"opencode"|"grok") {
                let init=vendor_commands::AcpCommand::Initialize {client_version:"0.1.0"};
                let init_step=failure(rpc::acp_candidate_step_id(&operation_id,&init))?;
                self.native_acp_rpc(&key,&init_step,Some(1),&init)?;
                let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
                let cwd=run.evidence.cwd().to_string_lossy().into_owned();
                let model=run.model.clone();let effort=run.effort.clone();
                let resume=if driver=="opencode" {
                    vendor_commands::AcpCommand::SessionResume {session_id:&thread_id,cwd:&cwd,advertised:true}
                } else {vendor_commands::AcpCommand::SessionLoad {session_id:&thread_id,cwd:&cwd,advertised:true}};
                let resume_step=failure(rpc::acp_candidate_step_id(&operation_id,&resume))?;
                if !matches!(self.native_acp_rpc(&key,&resume_step,Some(3),&resume)?,
                    Some(acp::Observation::SessionLoad {..}|acp::Observation::SessionResume {..})) {
                    return Err(OrchestrationError::Invalid("native ACP resume acknowledgement absent"));
                }
                if driver=="opencode" {
                    for (number,config_id,value) in [(4,"model",model.as_str()),(5,"effort",effort.as_str())] {
                        let setting=vendor_commands::AcpCommand::SetConfigOption {session_id:&thread_id,config_id,value};
                        let step=failure(rpc::acp_candidate_step_id(&operation_id,&setting))?;
                        if !matches!(self.native_acp_rpc(&key,&step,Some(number),&setting)?,
                            Some(acp::Observation::SessionConfigOption {..})) {
                            return Err(OrchestrationError::Invalid("native ACP resume setting acknowledgement absent"));
                        }
                    }
                    self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?.next_rpc_id=6;
                }
                let observed=self.observed_resume_thread(&key.0,&key.1,&operation_id,&new_generation,
                    &request.request_id)?.ok_or(OrchestrationError::OperationConflict)?;
                if observed!=thread_id {return Err(OrchestrationError::OperationConflict);}
                self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?.thread_id=Some(thread_id.clone());
                return Ok(());
            }
            if driver!="codex" {return Err(OrchestrationError::Invalid("native provider metadata resume unsupported"));}
            let host_tools=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?
                .evidence.host_tools_enabled();
            let initialize=if host_tools {Command::InitializeHostTools {client_version:"0.1.0".into()}}
                else {Command::Initialize {client_version:"0.1.0".into()}};
            self.native_rpc(&key,&format!("{operation_id}-initialize"),Some(1),
                &initialize)?;
            self.native_rpc(&key,&format!("{operation_id}-initialized"),None,
                &Command::Initialized)?;
            let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            let cwd=run.evidence.cwd().to_string_lossy().into_owned();
            let model=run.model.clone();
            self.native_rpc(&key,&format!("{operation_id}-config-read"),Some(2),
                &Command::ConfigRead {cwd:cwd.clone()})?;
            let response=self.native_rpc(&key,&format!("{operation_id}-thread-resume"),
                Some(3),&Command::ThreadResume {thread_id:thread_id.clone(),cwd,model})?;
            let Some(Reply::Thread {thread_id:observed,..})=response else {
                return Err(OrchestrationError::Invalid("native resume thread response"));
            };
            if observed!=thread_id {return Err(OrchestrationError::OperationConflict);}
            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?.thread_id=
                Some(thread_id.clone());
            Ok(())
        })();
        if let Err(error)=started {
            if self.native_sessions.get(&key).is_some_and(|run|run.raw_capture.has_pending()) {
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "native resume source retained under original request: {error:?}")));
            }
            let custody_unknown=authority::mark_process_unknown(&mut self.connection,
                &operation_id,&custody);
            failure(self.connection.execute("BEGIN IMMEDIATE"))?;
            let unknown=(|| -> Result<i64> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(h::mark_resume_unknown(&self.connection,&request.domain_id,
                    &request.target_id,&request.request_id,&operation_id))
            })();
            let next=match unknown {
                Ok(next)=>{self.finish_native_transaction(Ok(()))?;next},
                Err(error)=>{self.finish_native_transaction(Err(error))?;unreachable!()}
            };
            return Ok(encode_receipt(request,V37Status::Unknown,effective_revision,
                u64::try_from(next).map_err(|_|OrchestrationError::OperationConflict)?,
                BTreeMap::from([
                    (JsonString::from_str("state"),text("RESUME_UNKNOWN")),
                    (JsonString::from_str("oldGeneration"),text(&old_generation)),
                    (JsonString::from_str("reason"),text(&format!(
                        "native resume original error: {error:?}; custody: {custody_unknown:?}")))])));
        }
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let promoted=(|| -> Result<i64> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify_active_in_transaction(&mut self.connection,self.root,
                &self.owner,Some(&operation_id)))?;
            failure(h::promote_resume(&self.connection,&request.domain_id,&request.target_id,
                &request.request_id,&operation_id,old.revision))
        })();
        let next=match promoted {
            Ok(next)=>{self.finish_native_transaction(Ok(()))?;next},
            Err(error)=>{self.finish_native_transaction(Err(error))?;unreachable!()}
        };
        self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
            .evidence.adopt_resume(&self.connection,&self.owner,&operation_id)
            .map_err(OrchestrationError::V37StoreFailure)?;
        self.process_native_pending_output(&key)?;
        let receipt_id=self.resume_source_receipt(&request.domain_id,&request.target_id,
            &operation_id,&new_generation,&request.request_id)?;
        Ok(encode_receipt(request,V37Status::Applied,effective_revision,
            u64::try_from(next).map_err(|_|OrchestrationError::OperationConflict)?,
            BTreeMap::from([
                (JsonString::from_str("state"),text("RUNNING")),
                (JsonString::from_str("oldGeneration"),text(&old_generation)),
                (JsonString::from_str("newGeneration"),text(&new_generation)),
                (JsonString::from_str("receiptId"),text(&receipt_id))])))
    }

    fn resume_source_receipt(&self,domain:&str,session:&str,operation:&str,
        generation:&str,request_id:&str)->Result<String> {
        // Verify the typed original ACK and custody before projecting its A
        // source locator; ACP and Codex own distinct recovery step names.
        self.observed_resume_thread(domain,session,operation,generation,request_id)?
            .ok_or(OrchestrationError::OperationConflict)?;
        let codex_step=format!("{operation}-thread-resume");
        let acp_step=format!("{operation}-session-resume");
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_epoch,source_cursor FROM main.gogoke_v37_rpc_steps
              WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
                AND generation=?4 AND open_request_id=?5 AND step_id IN (?6,?7)
                AND phase='OBSERVED'")?;
        for (index,value) in [domain,session,operation,generation,request_id,
            codex_step.as_str(),acp_step.as_str()].iter().enumerate() {
            q.bind_text((index+1) as i32,value)?;
        }
        if !q.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let receipt=format!("{}:{}:{}",operation,q.column_text(0)?,q.column_text(1)?);
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
        Ok(receipt)
    }

    fn generation_unknown(&mut self,request:&V37Request)->Result<Vec<u8>> {
        let prior=failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
            .ok_or(OrchestrationError::OperationConflict)?;
        if prior.stage=="CANCELLED" || prior.owner_stop_request_id.is_some() {
            let revision=prior.unknown_revision.unwrap_or(prior.previous_revision);
            return Ok(encode_receipt(request,V37Status::Unknown,
                u64::try_from(prior.previous_revision).map_err(|_|OrchestrationError::OperationConflict)?,
                u64::try_from(revision).map_err(|_|OrchestrationError::OperationConflict)?,
                BTreeMap::from([(JsonString::from_str("oldGeneration"),text(&prior.old_generation)),
                    (JsonString::from_str("state"),text("GENERATION_UNKNOWN"))])));
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let pending=(||->Result<i64> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            failure(change::mark_unknown(&self.connection,&request.domain_id,&request.request_id))
        })();
        let revision=match pending {
            Ok(value)=>{self.finish_native_transaction(Ok(()))?;value},
            Err(error)=>{self.finish_native_transaction(Err(error))?;unreachable!()}
        };
        let change=failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
            .ok_or(OrchestrationError::OperationConflict)?;
        let mut result=BTreeMap::from([(JsonString::from_str("oldGeneration"),text(&change.old_generation)),
            (JsonString::from_str("state"),text("GENERATION_UNKNOWN"))]);
        if let Some(error)=&change.original_error {
            result.insert(JsonString::from_str("reason"),text(error));
        }
        Ok(encode_receipt(request,V37Status::Unknown,
            u64::try_from(change.previous_revision).map_err(|_|OrchestrationError::OperationConflict)?,
            u64::try_from(revision).map_err(|_|OrchestrationError::OperationConflict)?,
            result))
    }

    fn generation_error(&mut self,request:&V37Request,reason:&str)->Result<Vec<u8>> {
        let c=failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
            .ok_or(OrchestrationError::OperationConflict)?;
        if c.original_error.is_none() {
            let bounded=reason.chars().skip(reason.chars().count().saturating_sub(1024)).collect::<String>();
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let noted=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::note_error(&self.connection,&request.domain_id,&request.request_id,
                    &bounded))
            })();
            self.finish_native_transaction(noted)?;
        }
        self.generation_unknown(request)
    }

    fn original_compaction_item(&self,domain:&str,c:&change::Change)
        ->Result<Option<(String,String,String)>> {
        Ok(failure(rpc::observed_compaction_completion(&self.connection,domain,
            &c.session_id,&c.old_operation,&c.old_generation,&c.old_ticket,&c.old_nonce,
            &c.thread_id,c.source_watermark))?.map(|(key,item)|
                (key.source_epoch,key.source_cursor,item)))
    }

    fn stop_native_for_generation_change(&mut self,request:&V37Request,c:&change::Change)
        ->Result<()> {
        let key=(request.domain_id.clone(),request.target_id.clone());
        let Some(run)=self.native_sessions.get(&key) else {
            // A restart after the actual stop proof can settle this original
            // request from custody; it cannot execute another OS stop.
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let recovered=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(h::record_generation_change_stop_in_transaction(&mut self.connection,
                    &request.domain_id,&request.target_id,&c.old_operation,&request.request_id))?;
                Ok(())
            })();
            return self.finish_native_transaction(recovered);
        };
        if run.operation_id!=c.old_operation || run.custody.ticket.opaque()!=c.old_ticket
            || run.custody.custodian_nonce!=c.old_nonce || run.thread_id.as_deref()!=Some(&c.thread_id)
            || run.turn_id.is_some() || (!run.allows_input() && run.stop_proof.is_none()) {
            return Err(OrchestrationError::OperationConflict);
        }
        let custody=run.custody.clone();
        let proof=if let Some(proof)=run.stop_proof.as_ref() {proof.clone()} else {
            let close=self.process_custodian.close_child_input(&custody.ticket)
                .map_err(|error|format!("native generation stdin close: {error}"));
            let proof=self.process_custodian.stop(&custody.ticket,StopBudgets::production(),move||close)?;
            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
                .stop_proof=Some(proof.clone());
            proof
        };
        self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
            .raw_capture.capture(&mut self.connection,&c.old_operation,&c.old_nonce)?;
        self.drain_native_output(&key)?;
        if !self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?
            .raw_capture.source_exhausted() {
            return Err(OrchestrationError::Invalid("native generation stopped stdout is not exhausted"));
        }
        authority::mark_process_stopped(&mut self.connection,&c.old_operation,&proof)?;
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let stopped=(||->Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            failure(h::record_generation_change_stop_in_transaction(&mut self.connection,
                &request.domain_id,&request.target_id,&c.old_operation,&request.request_id))?;
            Ok(())
        })();
        self.finish_native_transaction(stopped)?;
        self.confirm_native_stop(&key)
    }

    fn stop_candidate_for_owner(&mut self,key:&(String,String),operation:&str,
        generation:&str)->Result<()> {
        let custody_state=Statement::prepare(self.connection.as_ptr(),
            "SELECT state,COALESCE(stop_proof_hash,'')
               FROM main.gogoke_coordination_process_custody
              WHERE operation_id=?1 AND domain_id=?2 AND generation=?3")?;
        custody_state.bind_text(1,operation)?;custody_state.bind_text(2,&key.0)?;
        custody_state.bind_text(3,generation)?;
        if !custody_state.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let state=custody_state.column_text(0)?;let saved_proof=custody_state.column_text(1)?;
        if custody_state.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(custody_state);
        if state=="STOPPED" {
            if saved_proof.is_empty() {return Err(OrchestrationError::OperationConflict);}
        } else {
            let run=self.native_sessions.get(key).ok_or(OrchestrationError::Invalid(
                "candidate stop has no retained native custody"))?;
            if run.operation_id!=operation || run.custody.binding.generation!=generation {
                return Err(OrchestrationError::OperationConflict);
            }
            let custody=run.custody.clone();
            let proof=if let Some(proof)=run.stop_proof.as_ref() {proof.clone()} else {
                let close=self.process_custodian.close_child_input(&custody.ticket)
                    .map_err(|error|format!("candidate stdin close: {error}"));
                let proof=self.process_custodian.stop(&custody.ticket,
                    StopBudgets::production(),move||close)?;
                self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?
                    .stop_proof=Some(proof.clone());
                proof
            };
            self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?
                .raw_capture.capture(&mut self.connection,operation,&custody.custodian_nonce)?;
            // The candidate can have a captured response before adoption. A
            // source must remain under its own process identity even if it is
            // not yet displayable as a current generation.
            self.drain_native_output(key)?;
            if !self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?
                .raw_capture.source_exhausted() {
                return Err(OrchestrationError::Invalid("candidate stop has no stdout terminal boundary"));
            }
            authority::mark_process_stopped(&mut self.connection,operation,&proof)?;
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let recorded=(||->Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let q=Statement::prepare(self.connection.as_ptr(),
                "SELECT c.stop_proof_hash,e.phase FROM main.gogoke_v37_h_process_episode e
                   JOIN main.gogoke_coordination_process_custody c
                     ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
                     AND c.generation=e.generation
                  WHERE e.process_operation_id=?1 AND e.domain_id=?2
                    AND e.session_id=?3 AND e.generation=?4
                    AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL")?;
            for (index,value) in [operation,key.0.as_str(),key.1.as_str(),generation].iter().enumerate() {
                q.bind_text((index+1) as i32,value)?;
            }
            if !q.step_row()? {return Err(OrchestrationError::OperationConflict);}
            let proof=q.column_text(0)?;let phase=q.column_text(1)?;
            if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
            drop(q);
            if phase!="STOPPED" {failure(h::mark_episode_stopped(&self.connection,operation,&proof))?;}
            Ok(())
        })();
        self.finish_native_transaction(recorded)?;
        if self.native_sessions.get(key).is_some_and(|run|run.operation_id==operation) {
            self.confirm_native_stop(key)?;
        }
        Ok(())
    }

    fn dispatch_owner_stop_generation_change(&mut self,request:&V37Request,
        c:&change::Change,previously_intended:bool)->Result<Vec<u8>> {
        let generation=user_payload_string(request,"generation")?;
        let seat_id=user_payload_string(request,"seatId")?;
        if generation!=c.old_generation || seat_id!=c.seat_id {
            return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let current=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &request.domain_id,&seat_id,&request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        if current.generation!=generation || current.process_operation_id.as_deref()!=Some(&c.old_operation) {
            return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        if !previously_intended && u64::try_from(current.revision).ok()!=Some(request.expected_revision) {
            let now=u64::try_from(current.revision).map_err(|_|OrchestrationError::OperationConflict)?;
            return Ok(encode_receipt(request,V37Status::Stale,now,now,Default::default()));
        }
        if !previously_intended {
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let fenced=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::begin_owner_stop(&self.connection,&request.domain_id,
                    &c.request_id,&request.request_id))?;
                let operation=Statement::prepare(self.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,
                      operation,session_id,status,previous_revision,revision)
                     VALUES(?1,?2,?3,'stop',?4,'UNKNOWN',?5,?5)")?;
                for (index,value) in [request.domain_id.as_str(),request.request_id.as_str(),
                    hex(&request.raw_bytes).as_str(),request.target_id.as_str()].iter().enumerate() {
                    operation.bind_text((index+1) as i32,value)?;
                }
                operation.bind_i64(5,current.revision)?;operation.step_done()?;
                let old=Statement::prepare(self.connection.as_ptr(),
                    "UPDATE main.gogoke_v37_h_process_episode SET stop_request_id=?1
                      WHERE domain_id=?2 AND session_id=?3 AND process_operation_id=?4
                        AND (stop_request_id IS NULL OR stop_request_id=?1)")?;
                old.bind_text(1,&request.request_id)?;old.bind_text(2,&request.domain_id)?;
                old.bind_text(3,&request.target_id)?;old.bind_text(4,&c.old_operation)?;
                old.step_done()?;Ok(())
            })();
            self.finish_native_transaction(fenced)?;
        } else if c.owner_stop_request_id.as_deref()!=Some(&request.request_id) {
            return Err(OrchestrationError::OperationConflict);
        }
        let key=(request.domain_id.clone(),request.target_id.clone());
        if current.phase!=runtime::SessionPhase::Stopped {
            let Some(run)=self.native_sessions.get(&key) else {
                // No process handle and no trusted STOPPED proof is UNKNOWN.
                let state=Statement::prepare(self.connection.as_ptr(),
                    "SELECT state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody
                      WHERE operation_id=?1 AND domain_id=?2 AND generation=?3")?;
                state.bind_text(1,&c.old_operation)?;state.bind_text(2,&request.domain_id)?;
                state.bind_text(3,&generation)?;
                if !state.step_row()? || state.column_text(0)?!="STOPPED"
                    || state.column_text(1)?.is_empty() {
                    return Ok(encode_receipt(request,V37Status::Unknown,request.expected_revision,
                        request.expected_revision,Default::default()));
                }
                drop(state);
                self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
                let recovered=(||->Result<()> {
                    authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                    failure(h::record_session_stop_in_transaction(&mut self.connection,
                        &request.domain_id,&request.target_id,&c.old_operation))?;
                    Ok(())
                })();
                self.finish_native_transaction(recovered)?;
                // Continue below to check an original candidate too.
                return self.dispatch_owner_stop_generation_change(request,c,true);
            };
            if run.operation_id!=c.old_operation || run.custody.ticket.opaque()!=c.old_ticket
                || run.custody.custodian_nonce!=c.old_nonce {
                return Err(OrchestrationError::OperationConflict);
            }
            let custody=run.custody.clone();
            let proof=if let Some(proof)=run.stop_proof.as_ref() {proof.clone()} else {
                let close=self.process_custodian.close_child_input(&custody.ticket)
                    .map_err(|error|format!("owner stop stdin close: {error}"));
                let proof=self.process_custodian.stop(&custody.ticket,
                    StopBudgets::production(),move||close)?;
                self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
                    .stop_proof=Some(proof.clone());
                proof
            };
            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
                .raw_capture.capture(&mut self.connection,&c.old_operation,&c.old_nonce)?;
            self.drain_native_output(&key)?;
            if !self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?
                .raw_capture.source_exhausted() {
                return Ok(encode_receipt(request,V37Status::Unknown,request.expected_revision,
                    request.expected_revision,Default::default()));
            }
            authority::mark_process_stopped(&mut self.connection,&c.old_operation,&proof)?;
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let stopped=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(h::record_session_stop_in_transaction(&mut self.connection,
                    &request.domain_id,&request.target_id,&c.old_operation))?;
                Ok(())
            })();
            self.finish_native_transaction(stopped)?;
            self.confirm_native_stop(&key)?;
        }
        let candidate=Statement::prepare(self.connection.as_ptr(),
            "SELECT COALESCE(process_operation_id,''),generation,phase
               FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                AND old_generation=?4")?;
        for (index,value) in [request.domain_id.as_str(),c.request_id.as_str(),
            request.target_id.as_str(),generation.as_str()].iter().enumerate() {
            candidate.bind_text((index+1) as i32,value)?;
        }
        let candidate_row=if candidate.step_row()? {
            let result=Some((candidate.column_text(0)?,candidate.column_text(1)?,candidate.column_text(2)?));
            if candidate.step_row()? {return Err(OrchestrationError::OperationConflict);}
            result
        } else {None};
        drop(candidate);
        if let Some((operation,new_generation,_))=&candidate_row {
            if !operation.is_empty() {
                if let Err(error)=self.stop_candidate_for_owner(&key,operation,new_generation) {
                    return Ok(encode_receipt(request,V37Status::Unknown,request.expected_revision,
                        request.expected_revision,BTreeMap::from([(JsonString::from_str("reason"),
                            text(&format!("Owner stop original candidate error: {error:?}")))])));
                }
            }
        }
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let completed=(||->Result<(i64,String)> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            if let Some((operation,_,phase))=&candidate_row {
                if operation.is_empty() {
                    if phase!="INTENT" {return Err(OrchestrationError::OperationConflict);}
                    let failed=Statement::prepare(self.connection.as_ptr(),
                        "UPDATE main.gogoke_v37_h_process_episode SET phase='FAILED'
                          WHERE domain_id=?1 AND request_id=?2 AND phase='INTENT'
                            AND process_operation_id IS NULL")?;
                    failed.bind_text(1,&request.domain_id)?;failed.bind_text(2,&c.request_id)?;
                    failed.step_done()?;
                } else {
                    let stopped=Statement::prepare(self.connection.as_ptr(),
                        "SELECT 1 FROM main.gogoke_v37_h_process_episode e
                           JOIN main.gogoke_coordination_process_custody p
                             ON p.operation_id=e.process_operation_id AND p.domain_id=e.domain_id
                             AND p.generation=e.generation
                          WHERE e.domain_id=?1 AND e.request_id=?2
                            AND e.process_operation_id=?3 AND e.phase='STOPPED'
                            AND e.stop_fact_id=p.stop_proof_hash AND p.state='STOPPED'")?;
                    stopped.bind_text(1,&request.domain_id)?;stopped.bind_text(2,&c.request_id)?;
                    stopped.bind_text(3,operation)?;
                    if !stopped.step_row()? {return Err(OrchestrationError::OperationConflict);}
                }
            }
            let old=Statement::prepare(self.connection.as_ptr(),
                "SELECT a.revision,a.stop_fact_id FROM main.gogoke_v37_h_claim a
                   JOIN main.gogoke_v37_h_process_episode e
                     ON e.process_operation_id=a.process_operation_id AND e.domain_id=a.domain_id
                     AND e.generation=a.generation
                   JOIN main.gogoke_coordination_process_custody p
                     ON p.operation_id=e.process_operation_id AND p.domain_id=e.domain_id
                     AND p.generation=e.generation
                  WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3
                    AND a.state='STOPPED' AND e.phase='STOPPED'
                    AND a.stop_fact_id=e.stop_fact_id AND e.stop_fact_id=p.stop_proof_hash
                    AND p.state='STOPPED'")?;
            old.bind_text(1,&request.domain_id)?;old.bind_text(2,&request.target_id)?;
            old.bind_text(3,&generation)?;
            if !old.step_row()? {return Err(OrchestrationError::OperationConflict);}
            let revision=old.column_text(0)?.parse::<i64>().map_err(|_|OrchestrationError::OperationConflict)?;
            let proof=old.column_text(1)?;
            if old.step_row()? {return Err(OrchestrationError::OperationConflict);}
            drop(old);
            let intended=Statement::prepare(self.connection.as_ptr(),
                "SELECT previous_revision FROM main.gogoke_v37_h_operation
                  WHERE domain_id=?1 AND request_id=?2 AND raw_hex=?3
                    AND operation='stop' AND status='UNKNOWN'")?;
            intended.bind_text(1,&request.domain_id)?;intended.bind_text(2,&request.request_id)?;
            intended.bind_text(3,&hex(&request.raw_bytes))?;
            if !intended.step_row()? {return Err(OrchestrationError::OperationConflict);}
            let before=intended.column_text(0)?.parse::<i64>().map_err(|_|OrchestrationError::OperationConflict)?;
            if intended.step_row()? {return Err(OrchestrationError::OperationConflict);}
            drop(intended);
            let next=if revision==before {
                let value=revision.checked_add(1).ok_or(OrchestrationError::OperationConflict)?;
                let bump=Statement::prepare(self.connection.as_ptr(),
                    "UPDATE main.gogoke_v37_h_claim SET revision=?4
                      WHERE domain_id=?1 AND session_id=?2 AND generation=?3
                        AND revision=?5 AND state='STOPPED'")?;
                bump.bind_text(1,&request.domain_id)?;bump.bind_text(2,&request.target_id)?;
                bump.bind_text(3,&generation)?;bump.bind_i64(4,value)?;
                bump.bind_i64(5,revision)?;bump.step_done()?;value
            } else if revision==before+1 {revision}
            else {return Err(OrchestrationError::OperationConflict)};
            let applied=Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET status='APPLIED',revision=?3
                  WHERE domain_id=?1 AND request_id=?2 AND status='UNKNOWN'")?;
            applied.bind_text(1,&request.domain_id)?;applied.bind_text(2,&request.request_id)?;
            applied.bind_i64(3,next)?;applied.step_done()?;
            failure(change::cancel_for_owner_stop(&self.connection,&request.domain_id,
                &c.request_id,&request.request_id))?;
            Ok((next,proof))
        })();
        let (revision,proof)=match completed {
            Ok(value)=>{self.finish_native_transaction(Ok(()))?;value},
            Err(error)=>{self.finish_native_transaction(Err(error))?;unreachable!()}
        };
        Ok(encode_receipt(request,V37Status::Applied,request.expected_revision,
            u64::try_from(revision).map_err(|_|OrchestrationError::OperationConflict)?,
            BTreeMap::from([(JsonString::from_str("stopFact"),text(&proof))])))
    }

    pub(super) fn dispatch_native_generation_change(&mut self,request:&V37Request)
        ->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if let Some(refusal)=self.formal_review_continuation_refusal(request)? {
            return Ok(refusal);
        }
        if !matches!(request.operation.as_str(),"compact"|"renew-session")
            || request.payload.len()!=1 {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let generation=user_payload_string(request,"generation")?;
        let key=(request.domain_id.clone(),request.target_id.clone());
        let mut c=if let Some(prior)=failure(change::read(&self.connection,&request.domain_id,
            &request.request_id))? {
            if prior.raw_hex!=hex(&request.raw_bytes) || prior.operation!=request.operation
                || prior.session_id!=request.target_id || prior.old_generation!=generation {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,Default::default()));
            }
            prior
        } else {
            let Some(run)=self.native_sessions.get(&key) else {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,Default::default()));
            };
            if run.evidence.driver_id()!="codex" {
                return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                    request.expected_revision,BTreeMap::from([(JsonString::from_str("reason"),
                        text("This fixed provider's generation change is not integrated"))])));
            }
            let thread=run.thread_id.clone().ok_or(OrchestrationError::AccessDenied)?;
            let original_thread=self.original_native_continuation(&request.domain_id,
                &request.target_id)?.2;
            if original_thread!=thread {return Err(OrchestrationError::OperationConflict);}
            let seat=run.evidence.seat_id().to_owned();
            let process=run.operation_id.clone();
            let ticket=run.custody.ticket.opaque().to_owned();
            let nonce=run.custody.custodian_nonce.clone();
            if run.custody.binding.generation!=generation || run.turn_id.is_some()
                || !run.allows_input() {
                return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                    request.expected_revision,Default::default()));
            }
            let claim=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
                &request.domain_id,&seat,&request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
            let revision=u64::try_from(claim.revision).map_err(|_|OrchestrationError::OperationConflict)?;
            if claim.generation!=generation || claim.process_operation_id.as_deref()!=Some(&process) {
                return Ok(encode_receipt(request,V37Status::Conflict,revision,revision,Default::default()));
            }
            if revision!=request.expected_revision {
                return Ok(encode_receipt(request,V37Status::Stale,revision,revision,Default::default()));
            }
            run.evidence.verify_live(&mut self.connection,self.root,&self.owner,&process,claim.revision)
                .map_err(OrchestrationError::V37StoreFailure)?;
            let q=Statement::prepare(self.connection.as_ptr(),
                "SELECT COALESCE(MAX(CAST(source_cursor AS INTEGER)),0)
                   FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2")?;
            q.bind_text(1,&process)?;q.bind_text(2,&nonce)?;
            if !q.step_row()? {return Err(OrchestrationError::OperationConflict);}
            let watermark=q.column_text(0)?.parse::<i64>().map_err(|_|OrchestrationError::OperationConflict)?;
            drop(q);
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let begun=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::begin(&self.connection,&request.domain_id,&request.request_id,
                    &request.raw_bytes,&request.operation,&request.target_id,&generation,
                    &process,&ticket,&nonce,&thread,&seat,claim.revision,watermark))?;
                Ok(())
            })();
            self.finish_native_transaction(begun)?;
            failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
                .ok_or(OrchestrationError::OperationConflict)?
        };
        if c.owner_stop_request_id.is_some() || c.stage=="CANCELLED" {
            return self.generation_unknown(request);
        }
        if c.stage=="UNSUPPORTED" {
            return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        if c.stage=="APPLIED" {
            let new=(c.old_generation.parse::<u64>().map_err(|_|OrchestrationError::OperationConflict)?+1).to_string();
            let episode=Statement::prepare(self.connection.as_ptr(),
                "SELECT process_operation_id,result_revision FROM main.gogoke_v37_h_process_episode
                  WHERE domain_id=?1 AND request_id=?2 AND generation=?3 AND phase IN ('ACTIVE','STOPPED')")?;
            episode.bind_text(1,&request.domain_id)?;episode.bind_text(2,&request.request_id)?;
            episode.bind_text(3,&new)?;
            if !episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
            let operation=episode.column_text(0)?;
            let revision=episode.column_text(1)?.parse::<u64>().map_err(|_|OrchestrationError::OperationConflict)?;
            if episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
            let receipt=self.resume_source_receipt(&request.domain_id,&request.target_id,
                &operation,&new,&request.request_id)?;
            return Ok(encode_receipt(request,V37Status::Replayed,
                u64::try_from(c.unknown_revision.unwrap_or(c.previous_revision))
                    .map_err(|_|OrchestrationError::OperationConflict)?,revision,
                BTreeMap::from([(JsonString::from_str("state"),text("RUNNING")),
                    (JsonString::from_str("oldGeneration"),text(&generation)),
                    (JsonString::from_str("newGeneration"),text(&new)),
                    (JsonString::from_str("receiptId"),text(&receipt))])));
        }
        if request.operation=="compact" && c.stage=="INTENT" {
            let step=format!("compact-{}",&crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
            let existing=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND step_id=?3")?;
            existing.bind_text(1,&request.domain_id)?;existing.bind_text(2,&request.target_id)?;
            existing.bind_text(3,&step)?;
            let has_step=existing.step_row()?;drop(existing);
            let mut send_error=None;
            if !has_step {
                let run=self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?;
                let number=run.next_rpc_id;
                run.next_rpc_id=number.checked_add(1).ok_or(OrchestrationError::OperationConflict)?;
                let sent=self.native_rpc(&key,&step,Some(number),
                    &Command::ThreadCompactStart {thread_id:c.thread_id.clone()});
                if !matches!(sent,Ok(Some(Reply::Ack {..}))) {
                    // A remote error can already be durably OBSERVED. Read
                    // that original response before assigning UNKNOWN so a
                    // definite method-not-found keeps its UNSUPPORTED meaning.
                    send_error=Some(format!("original compact RPC: {sent:?}"));
                }
            } else {
                failure(rpc::reconcile_written_compact_from_a(&mut self.connection,&self.owner,
                    &request.domain_id,&request.target_id,&c.old_operation,&generation,
                    &c.old_ticket,&c.old_nonce,&step,&c.thread_id))?;
            }
            let ack=rpc::observed_compact_ack(&self.connection,&request.domain_id,
                &request.target_id,&c.old_operation,&generation,&c.old_ticket,&c.old_nonce,
                &step,&c.thread_id);
            if let Err(rpc::RpcJournalError::Codec(codex_rpc::RpcError::RemoteResponse(frame)))=&ack {
                if compact_method_missing(frame) && c.unknown_revision.is_none() {
                    self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
                    let unsupported=(||->Result<()> {
                        authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                        failure(change::mark_unsupported(&self.connection,&request.domain_id,
                            &request.request_id,&step))
                    })();
                    self.finish_native_transaction(unsupported)?;
                    return Ok(encode_receipt(request,V37Status::Unsupported,
                        request.expected_revision,request.expected_revision,Default::default()));
                }
            }
            if !matches!(ack,Ok(Some(_))) {
                let reason=send_error.unwrap_or_else(||format!("original compact ACK source: {ack:?}"));
                return self.generation_error(request,&reason);
            }
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let marked=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::mark_ack(&self.connection,&request.domain_id,&request.request_id,&step))
            })();
            self.finish_native_transaction(marked)?;
            c=failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
                .ok_or(OrchestrationError::OperationConflict)?;
        }
        if request.operation=="compact" && c.stage=="ACKED" {
            if self.native_sessions.get(&key).is_some() {self.drain_native_output(&key)?;}
            let Some((epoch,cursor,item))=self.original_compaction_item(&request.domain_id,&c)? else {
                return self.generation_unknown(request);
            };
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let marked=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::mark_item(&self.connection,&request.domain_id,&request.request_id,
                    &epoch,&cursor,&item))
            })();
            self.finish_native_transaction(marked)?;
            c=failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
                .ok_or(OrchestrationError::OperationConflict)?;
        }
        if matches!(c.stage.as_str(),"INTENT"|"ITEM_OBSERVED") {
            if let Err(error)=self.stop_native_for_generation_change(request,&c) {
                return self.generation_error(request,&format!("original generation stop: {error:?}"));
            }
            c=failure(change::read(&self.connection,&request.domain_id,&request.request_id))?
                .ok_or(OrchestrationError::OperationConflict)?;
        }
        if c.stage!="OLD_STOPPED" {return self.generation_unknown(request);}
        let effective=u64::try_from(c.unknown_revision.unwrap_or(c.previous_revision))
            .map_err(|_|OrchestrationError::OperationConflict)?;
        let bytes=match self.dispatch_native_resume_at(request,effective) {
            Ok(bytes)=>bytes,
            Err(error)=>return self.generation_error(request,&format!(
                "original continuation: {error:?}")),
        };
        let receipt=h::decode_receipt(&bytes).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native generation receipt: {error:?}")))?;
        if matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
            let completed_revision=receipt.revision;
            let final_bytes=if c.unknown_revision.is_some() && receipt.status==V37Status::Applied {
                encode_receipt(request,V37Status::Replayed,effective,receipt.revision,
                    receipt.into_result())
            } else {bytes};
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let applied=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::mark_applied(&self.connection,&request.domain_id,&request.request_id,
                    i64::try_from(completed_revision).map_err(|_|OrchestrationError::OperationConflict)?))
            })();
            self.finish_native_transaction(applied)?;
            return Ok(final_bytes);
        }
        if receipt.status==V37Status::Unknown {return self.generation_unknown(request);}
        Ok(bytes)
    }

    /// Reconnect reads or commits only an already captured original outcome.
    /// It never owns a provider writer, a Job start, or an OS stop.
    pub(super) fn dispatch_native_reconnect(&mut self,request:&V37Request)
        ->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection,&self.owner)?;
        if let Some(refusal)=self.formal_review_continuation_refusal(request)? {
            return Ok(refusal);
        }
        if request.payload.len()!=1 {
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let old_generation=user_payload_string(request,"generation")?;
        let prior=Statement::prepare(self.connection.as_ptr(),
            "SELECT request_bytes,receipt_bytes FROM main.v37_ledger_receipt
              WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1,&request.domain_id)?;prior.bind_text(2,&request.request_id)?;
        if prior.step_row()? {
            let same=prior.column_text(0)?.as_bytes()==request.raw_bytes;
            let bytes=prior.column_text(1)?.into_bytes();
            if prior.step_row()? {return Err(OrchestrationError::OperationConflict);}
            if !same {return Ok(encode_receipt(request,V37Status::Conflict,
                request.expected_revision,request.expected_revision,Default::default()));}
            let receipt=failure(h::decode_receipt(&bytes))?;
            return Ok(encode_receipt(request,V37Status::Replayed,receipt.previous_revision,
                receipt.revision,receipt.into_result()));
        }
        drop(prior);
        let binding=Statement::prepare(self.connection.as_ptr(),
            "SELECT seat_id FROM main.gogoke_v37_h_seat_binding
              WHERE domain_id=?1 AND session_id=?2")?;
        binding.bind_text(1,&request.domain_id)?;binding.bind_text(2,&request.target_id)?;
        if !binding.step_row()? {return Ok(encode_receipt(request,V37Status::Conflict,
            request.expected_revision,request.expected_revision,Default::default()));}
        let seat_id=binding.column_text(0)?;
        if binding.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(binding);
        let before=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &request.domain_id,&seat_id,&request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        let before_revision=u64::try_from(before.revision).map_err(|_|OrchestrationError::OperationConflict)?;
        if before_revision!=request.expected_revision {
            return Ok(encode_receipt(request,V37Status::Stale,before_revision,
                before_revision,Default::default()));
        }
        let active=failure(change::active_for_session(&self.connection,&request.domain_id,
            &request.target_id))?;
        let mut promoted=false;
        if let Some(c)=&active {
            if c.old_generation!=old_generation || c.owner_stop_request_id.is_some() {
                return Ok(encode_receipt(request,V37Status::Conflict,before_revision,
                    before_revision,Default::default()));
            }
            if c.stage!="OLD_STOPPED" {
                return Ok(encode_receipt(request,V37Status::Unknown,before_revision,
                    before_revision,BTreeMap::from([
                        (JsonString::from_str("oldGeneration"),text(&old_generation)),
                        (JsonString::from_str("state"),text("GENERATION_UNKNOWN"))])));
            }
            let candidate=Statement::prepare(self.connection.as_ptr(),
                "SELECT process_operation_id,generation,phase FROM main.gogoke_v37_h_process_episode
                  WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                    AND old_generation=?4 AND process_operation_id IS NOT NULL
                    AND phase IN ('PREPARED','UNKNOWN','ACTIVE')")?;
            for (index,value) in [request.domain_id.as_str(),c.request_id.as_str(),
                request.target_id.as_str(),old_generation.as_str()].iter().enumerate() {
                candidate.bind_text((index+1) as i32,value)?;
            }
            if !candidate.step_row()? {
                return Ok(encode_receipt(request,V37Status::Unknown,before_revision,
                    before_revision,Default::default()));
            }
            let candidate_operation=candidate.column_text(0)?;
            let candidate_generation=candidate.column_text(1)?;
            let candidate_phase=candidate.column_text(2)?;
            if candidate.step_row()? {return Err(OrchestrationError::OperationConflict);}
            drop(candidate);
            let key=(request.domain_id.clone(),request.target_id.clone());
            if !self.native_sessions.get(&key).is_some_and(|run|
                run.operation_id==candidate_operation
                    && self.process_custodian.active(&run.custody.ticket).is_some()) {
                return Ok(encode_receipt(request,V37Status::Unknown,before_revision,
                    before_revision,Default::default()));
            }
            let settled_revision=if candidate_phase=="ACTIVE" {
                if before.generation!=candidate_generation
                    || before.process_operation_id.as_deref()!=Some(&candidate_operation) {
                    return Err(OrchestrationError::OperationConflict);
                }
                let run=self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
                let observed=rpc::observed_thread_id(&self.connection,&request.domain_id,
                    &request.target_id,&candidate_operation,&candidate_generation,
                    &c.request_id,run.custody.ticket.opaque(),&run.custody.custodian_nonce)
                    .map_err(|error|OrchestrationError::V37StoreFailure(format!(
                        "reconnect original candidate: {error:?}")))?;
                if run.thread_id.as_deref()!=Some(&observed) {
                    return Err(OrchestrationError::OperationConflict);
                }
                before.revision
            } else {
                let original=h::decode_request(&unhex(&c.raw_hex)?).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("original generation request: {error:?}")))?;
                let result=self.dispatch_native_resume_at(&original,before_revision)?;
                let settled=failure(h::decode_receipt(&result))?;
                if !matches!(settled.status,V37Status::Applied|V37Status::Replayed) {
                    return Ok(encode_receipt(request,V37Status::Unknown,before_revision,
                        before_revision,Default::default()));
                }
                promoted=true;
                i64::try_from(settled.revision).map_err(|_|OrchestrationError::OperationConflict)?
            };
            self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
            let recorded=(||->Result<()> {
                authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
                failure(change::mark_applied(&self.connection,&request.domain_id,&c.request_id,
                    settled_revision))
            })();
            self.finish_native_transaction(recorded)?;
        }
        let now=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &request.domain_id,&seat_id,&request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        let old_to_new=if now.generation==old_generation {None} else {
            let id=if let Some(c)=&active {c.request_id.clone()} else {
                let q=Statement::prepare(self.connection.as_ptr(),
                    "SELECT request_id FROM main.gogoke_v37_h_generation_change
                      WHERE domain_id=?1 AND session_id=?2 AND old_generation=?3
                        AND stage='APPLIED'")?;
                q.bind_text(1,&request.domain_id)?;q.bind_text(2,&request.target_id)?;
                q.bind_text(3,&old_generation)?;
                if !q.step_row()? {return Ok(encode_receipt(request,V37Status::Conflict,
                    before_revision,before_revision,Default::default()));}
                let id=q.column_text(0)?;
                if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
                id
            };
            let c=failure(change::read(&self.connection,&request.domain_id,&id))?;
            if c.as_ref().map_or(true,|c|c.stage!="APPLIED" || c.old_generation!=old_generation
                || c.result_revision.is_none()) {
                return Ok(encode_receipt(request,V37Status::Conflict,before_revision,
                    before_revision,Default::default()));
            }
            let linked=Statement::prepare(self.connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_h_process_episode
                  WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                    AND generation=?4 AND process_operation_id=?5")?;
            linked.bind_text(1,&request.domain_id)?;linked.bind_text(2,&id)?;
            linked.bind_text(3,&request.target_id)?;linked.bind_text(4,&now.generation)?;
            linked.bind_text(5,now.process_operation_id.as_deref().unwrap_or(""))?;
            if !linked.step_row()? || linked.step_row()? {
                return Ok(encode_receipt(request,V37Status::Conflict,before_revision,
                    before_revision,Default::default()));
            }
            c
        };
        if now.phase!=runtime::SessionPhase::Committed || now.process_operation_id.is_none() {
            return Ok(encode_receipt(request,V37Status::Unknown,before_revision,
                before_revision,Default::default()));
        }
        let key=(request.domain_id.clone(),request.target_id.clone());
        let Some(run)=self.native_sessions.get(&key) else {
            return Ok(encode_receipt(request,V37Status::Unknown,before_revision,
                before_revision,Default::default()));
        };
        if now.process_operation_id.as_deref()!=Some(&run.operation_id) {
            return Err(OrchestrationError::OperationConflict);
        }
        run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,now.revision).map_err(OrchestrationError::V37StoreFailure)?;
        let thread=rpc::observed_thread_id(&self.connection,&request.domain_id,&request.target_id,
            &run.operation_id,&now.generation,&run.open_request_id,
            run.custody.ticket.opaque(),&run.custody.custodian_nonce).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("reconnect original thread: {error:?}")))?;
        if run.thread_id.as_deref()!=Some(&thread) {return Err(OrchestrationError::OperationConflict);}
        let episode=Statement::prepare(self.connection.as_ptr(),
            "SELECT old_generation FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND request_id=?2 AND process_operation_id=?3")?;
        episode.bind_text(1,&request.domain_id)?;episode.bind_text(2,&run.open_request_id)?;
        episode.bind_text(3,&run.operation_id)?;
        if !episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let resumed=!episode.column_text(0)?.is_empty();
        if episode.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(episode);
        let step=if resumed {format!("{}-thread-resume",run.operation_id)} else {"thread-start".into()};
        let source=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_epoch,source_cursor FROM main.gogoke_v37_rpc_steps
              WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
                AND generation=?4 AND step_id=?5 AND phase='OBSERVED'")?;
        for (index,value) in [request.domain_id.as_str(),request.target_id.as_str(),
            run.operation_id.as_str(),now.generation.as_str(),step.as_str()].iter().enumerate() {
            source.bind_text((index+1) as i32,value)?;
        }
        if !source.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let receipt_id=format!("{}:{}:{}",run.operation_id,source.column_text(0)?,source.column_text(1)?);
        if source.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(source);
        let final_revision=if promoted {now.revision} else {
            now.revision.checked_add(1).ok_or(OrchestrationError::OperationConflict)?
        };
        let previous=if promoted {before.revision} else {now.revision};
        let result=BTreeMap::from([
            (JsonString::from_str("oldGeneration"),text(&old_generation)),
            (JsonString::from_str("newGeneration"),text(&now.generation)),
            (JsonString::from_str("receiptId"),text(&receipt_id)),
            (JsonString::from_str("state"),text("RUNNING")),
        ]);
        let bytes=encode_receipt(request,V37Status::Applied,
            u64::try_from(previous).map_err(|_|OrchestrationError::OperationConflict)?,
            u64::try_from(final_revision).map_err(|_|OrchestrationError::OperationConflict)?,result);
        self.connection.execute("BEGIN IMMEDIATE").map_err(OrchestrationError::CommitUnknownWithCause)?;
        let committed=(||->Result<()> {
            authority::check_owner_in_current_transaction(&self.connection,&self.owner)?;
            let current=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
                &request.domain_id,&seat_id,&request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
            if current!=now {return Err(OrchestrationError::OperationConflict);}
            if !promoted {
                let bump=Statement::prepare(self.connection.as_ptr(),
                    "UPDATE main.gogoke_v37_h_claim SET revision=?3
                      WHERE domain_id=?1 AND session_id=?2 AND revision=?4")?;
                bump.bind_text(1,&request.domain_id)?;bump.bind_text(2,&request.target_id)?;
                bump.bind_i64(3,final_revision)?;bump.bind_i64(4,now.revision)?;bump.step_done()?;
            }
            let insert=Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.v37_ledger_receipt(family,domain_id,request_id,request_bytes,receipt_bytes)
                  VALUES('K-SESSION',?1,?2,?3,?4)")?;
            insert.bind_text(1,&request.domain_id)?;insert.bind_text(2,&request.request_id)?;
            insert.bind_blob(3,&request.raw_bytes)?;insert.bind_blob(4,&bytes)?;insert.step_done()?;
            Ok(())
        })();
        self.finish_native_transaction(committed)?;
        let _=old_to_new;
        Ok(bytes)
    }

    /// Observe the original durable outcome before preparing another process.
    /// UNKNOWN cannot be converted into a launch by changing a request ID.
    pub(super) fn dispatch_native_open(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        let purpose=match request.payload.get(&JsonString::from_str("purpose")) {
            None=>SessionPurpose::Work,
            Some(Json::String(value)) if value.to_well_formed_string().as_deref()==Some("FORMAL_REVIEW")=>
                SessionPurpose::FormalReview,
            _=>return Ok(encode_receipt(request,V37Status::Denied,
                request.expected_revision,request.expected_revision,Default::default())),
        };
        self.dispatch_native_open_registered(request, purpose, None, None)
    }

    /// A formal review is a fresh native session, never a recovered or forked
    /// context. Check the persisted purpose before any continuation side effect.
    fn formal_review_continuation_refusal(&self,request:&V37Request)->Result<Option<Vec<u8>>> {
        if ledger::read_registered_session(&self.connection,&request.target_id)?
            .is_some_and(|session|session.purpose==SessionPurpose::FormalReview) {
            return Ok(Some(encode_receipt(request,V37Status::Denied,
                request.expected_revision,request.expected_revision,BTreeMap::from([
                    (JsonString::from_str("reason"),text("formal review requires a fresh native session; context inheritance is refused")),
                ]))));
        }
        Ok(None)
    }

    pub(super) fn dispatch_native_child_open(&mut self,request:&V37Request,
        caller:&seat::NativeSeatCall)->Result<Vec<u8>> {
        let admission=seat::NativeLeadAdmission::from_model_call(caller)?;
        self.dispatch_native_open_registered(request,SessionPurpose::Work,None,Some(&admission))
    }

    /// Only the native User side-open composition chooses this registration.
    pub(super) fn dispatch_native_side_open(&mut self, request: &V37Request,
        side_id: &str) -> Result<Vec<u8>> {
        if !self.user_session_request_identity_matches(request)? {
            return Ok(encode_receipt(request,V37Status::Conflict,
                request.expected_revision,request.expected_revision,Default::default()));
        }
        self.dispatch_native_open_registered(request, SessionPurpose::SideChat, Some(side_id), None)
    }

    fn dispatch_native_open_registered(&mut self, request: &V37Request,
        purpose: SessionPurpose, side_id: Option<&str>,admission:Option<&seat::NativeLeadAdmission>) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let expected_fields=if purpose==SessionPurpose::FormalReview {5} else {4};
        if request.payload.len() != expected_fields {
            return Ok(encode_receipt(request, V37Status::Denied,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let seat_id = user_payload_string(request, "seatId")?;
        let generation = user_payload_string(request, "generation")?;
        let repository_id = user_payload_string(request, "repositoryId")?;
        let worktree_id = user_payload_string(request, "worktreeId")?;
        let registration = SessionRegistration {
            domain_id: request.domain_id.clone(), seat_id: seat_id.clone(), session_id: request.target_id.clone(),
            purpose, side_id: side_id.map(str::to_owned),
        };
        if ledger::read_registered_session(&self.connection, &request.target_id)?
            .is_some_and(|existing| existing != registration) {
            return Err(OrchestrationError::OperationConflict);
        }
        let prior = Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex,operation,session_id,status,previous_revision,revision FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1, &request.domain_id)?;
        prior.bind_text(2, &request.request_id)?;
        if prior.step_row()? {
            let same = prior.column_text(0)? == hex(&request.raw_bytes)
                && prior.column_text(1)? == "open" && prior.column_text(2)? == request.target_id;
            let mut status = if !same { V37Status::Conflict }
                else if prior.column_text(3)? != "APPLIED" { V37Status::Unknown }
                else { V37Status::Replayed };
            let revision = prior.column_text(5)?.parse::<u64>().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native open prior revision: {error}")))?;
            drop(prior);
            let mut result = BTreeMap::new();
            if status == V37Status::Replayed {
                // Replay reads the original H command and its exact captured A
                // reply. A process stop alone cannot establish a handshake.
                if self.original_claude_open_ready(request)? {
                    result.insert(JsonString::from_str("readinessBasis"),text("ORIGINAL_CLAUDE_INITIALIZE_ACK"));
                } else if let Some(thread)=self.observed_native_open_thread(
                    &request.domain_id,&request.target_id,&request.request_id)? {
                    result.insert(JsonString::from_str("threadId"),text(&thread));
                } else {status=V37Status::Unknown;}
            }
            return Ok(encode_receipt(request, status, request.expected_revision, revision, result));
        }
        drop(prior);
        let fenced = Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='open'")?;
        fenced.bind_text(1, &request.domain_id)?;
        fenced.bind_text(2, &request.target_id)?;
        if fenced.step_row()? {
            return Ok(encode_receipt(request, V37Status::Unknown,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        drop(fenced);
        let origin=match admission {Some(admission)=>NativeOrigin::lead(admission),None=>NativeOrigin::user(&self.owner)};
        let evidence = failure(LaunchEvidence::observe_with_origin(&mut self.connection, self.root,
            &self.owner,&origin,&request.domain_id,&seat_id,&request.target_id,&repository_id,&worktree_id))?;
        let (model, effort) = failure(evidence.settings())?;
        let current = failure(runtime::observe_claim(&self.connection, &NativeOrigin::user(&self.owner),
            &request.domain_id, &seat_id, &request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        if current.generation != generation || u64::try_from(current.revision).ok() != Some(request.expected_revision) {
            return Ok(encode_receipt(request, V37Status::Stale,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let key = (request.domain_id.clone(), request.target_id.clone());
        if self.native_sessions.contains_key(&key) {
            return Ok(encode_receipt(request, V37Status::Conflict,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        // A committed original intention fences every subsequent open before
        // the first OS side effect. OS creation is not represented as SQL atomic.
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let intention = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            failure(evidence.verify_in_transaction(&mut self.connection, self.root, &self.owner, None))?;
            let insert = Statement::prepare(self.connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,'open',?4,'UNKNOWN',?5,?5)")?;
            for (index, value) in [request.domain_id.as_str(), request.request_id.as_str(),
                hex(&request.raw_bytes).as_str(), request.target_id.as_str()].iter().enumerate() {
                insert.bind_text((index + 1) as i32, value)?;
            }
            insert.bind_i64(5, current.revision)?;
            insert.step_done()?;
            Ok(())
        })();
        self.finish_native_transaction(intention)?;
        let launch = failure(evidence.request())?;
        let custody = self.process_custodian.prepare(&launch)?;
        let digest = crate::store::digest::sha256_hex(&request.raw_bytes);
        let operation_id = format!("h-open-{}", &digest[..40]);
        // Hold the complete witness before any fallible persistence/activation.
        self.native_sessions.insert(key.clone(), NativeSession { evidence, custody: custody.clone(),
            operation_id: operation_id.clone(), open_request_id: request.request_id.clone(),
            open_request_bytes: request.raw_bytes.clone(), domain_id: request.domain_id.clone(),
            session_id: request.target_id.clone(), model, effort, thread_id: None, turn_id: None, raw_capture: Default::default(), stop_proof: None,
            next_rpc_id: 4, pending_acp:None,pending_claude:None });
        if let Err(error) = authority::record_prepared_process(&mut self.connection, &operation_id, &custody) {
            let abort = self.process_custodian.abort_prepared(&custody);
            return Err(OrchestrationError::V37StoreFailure(format!(
                "native open PREPARED record: {error:?}; abort: {abort:?}")));
        }
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let bind = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify_in_transaction(&mut self.connection, self.root, &self.owner, None))?;
            failure(h::bind_process_operation_in_transaction(&mut self.connection,
                &request.domain_id, &request.target_id, &operation_id))?;
            failure(h::record_initial(&self.connection,&request.domain_id,
                &request.target_id,&request.request_id,&operation_id))?;
            ledger::register_session(&mut self.connection, &registration)?;
            Ok(())
        })();
        if let Err(error) = self.finish_native_transaction(bind) {
            let abort = self.process_custodian.abort_prepared(&custody);
            return Err(OrchestrationError::V37StoreFailure(format!("native open bind: {error:?}; abort: {abort:?}")));
        }
        let started = (|| -> Result<()> {
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify(&mut self.connection, self.root, &self.owner, Some(&operation_id)))?;
            self.process_custodian.activate(&custody)?;
            authority::mark_process_active(&mut self.connection, &operation_id, &custody)?;
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            let driver=run.evidence.driver_id().to_owned();
            let cwd=run.evidence.cwd().to_string_lossy().into_owned();
            let model=run.model.clone();
            let host_tools=run.evidence.host_tools_enabled();
            let thread_id=match driver.as_str() {
                "codex" => {
                    let initialize=if host_tools {Command::InitializeHostTools {client_version:"0.1.0".into()}}
                        else {Command::Initialize {client_version:"0.1.0".into()}};
                    self.native_rpc(&key,"initialize",Some(1),&initialize)?;
                    self.native_rpc(&key,"initialized",None,&Command::Initialized)?;
                    self.native_rpc(&key,"config-read",Some(2),&Command::ConfigRead {cwd:cwd.clone()})?;
                    let start=if host_tools {Command::ThreadStartHostTools {cwd,model}}
                        else {Command::ThreadStart {cwd,model}};
                    let Some(Reply::Thread {thread_id,..})=self.native_rpc(&key,"thread-start",Some(3),&start)? else {
                        return Err(OrchestrationError::Invalid("native open thread response"));
                    };
                    Some(thread_id)
                },
                "claude" => {
                    self.native_claude_initialize(&key)?;
                    // A control ACK establishes readiness only. The actual
                    // vendor session arrives with the first real user turn.
                    self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?.thread_id.clone()
                },
                "opencode" | "grok" => {
                    self.native_acp_rpc(&key,"initialize",Some(1),
                        &vendor_commands::AcpCommand::Initialize {client_version:"0.1.0"})?;
                    let Some(acp::Observation::SessionNew {session_id,..})=self.native_acp_rpc(
                        &key,"thread-start",Some(3),&vendor_commands::AcpCommand::SessionNew {cwd:&cwd})? else {
                        return Err(OrchestrationError::Invalid("native ACP session/new response"));
                    };
                    if driver=="opencode" {
                        // The original H session/new ACK establishes the vendor
                        // session. Each setting is a separate original RPC ACK,
                        // checked by H against its requested option and value.
                        let effort=self.native_sessions.get(&key)
                            .ok_or(OrchestrationError::AccessDenied)?.effort.clone();
                        for (step,number,config_id,value) in [
                            ("setting-model",4,"model",model.as_str()),
                            ("setting-effort",5,"effort",effort.as_str()),
                        ] {
                            if !matches!(self.native_acp_rpc(&key,step,Some(number),
                                &vendor_commands::AcpCommand::SetConfigOption {
                                    session_id:&session_id,config_id,value})?,
                                Some(acp::Observation::SessionConfigOption {..})) {
                                return Err(OrchestrationError::Invalid("native ACP setting acknowledgement absent"));
                            }
                        }
                        self.native_sessions.get_mut(&key)
                            .ok_or(OrchestrationError::AccessDenied)?.next_rpc_id=6;
                    }
                    Some(session_id)
                },
                _ => return Err(OrchestrationError::Invalid("native provider handshake not integrated")),
            };
            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?.thread_id=thread_id;
            self.process_native_pending_output(&key)?;
            Ok(())
        })();
        if let Err(error) = started {
            // Keep the original intention UNKNOWN even if cleanup succeeds;
            // do not make a failed handshake an APPLIED open via a stop receipt.
            if self.native_sessions.get(&key).is_some_and(|run|run.raw_capture.has_pending()) {
                return Err(OrchestrationError::V37StoreFailure(format!("native open capture retained; original open remains UNKNOWN: {error:?}")));
            }
            let unknown = authority::mark_process_unknown(&mut self.connection, &operation_id, &custody);
            return Err(OrchestrationError::V37StoreFailure(format!("native open handshake: {error:?}; UNKNOWN record: {unknown:?}")));
        }
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let applied = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify_active_in_transaction(&mut self.connection, self.root, &self.owner, Some(&operation_id)))?;
            let next=current.revision.checked_add(1).ok_or(OrchestrationError::Invalid("native open revision overflow"))?;
            let advance=Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_claim SET revision=?1 WHERE domain_id=?2 AND session_id=?3 AND state='COMMITTED' AND revision=?4 AND process_operation_id=?5")?;
            advance.bind_i64(1,next)?;advance.bind_text(2,&request.domain_id)?;
            advance.bind_text(3,&request.target_id)?;advance.bind_i64(4,current.revision)?;
            advance.bind_text(5,&operation_id)?;advance.step_done()?;
            let changed=Statement::prepare(self.connection.as_ptr(),"SELECT changes()")?;
            if !changed.step_row()? || changed.column_text(0)?!="1" {return Err(OrchestrationError::OperationConflict);}
            drop(changed);drop(advance);
            let update = Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET status='APPLIED',revision=?4 WHERE domain_id=?1 AND request_id=?2 AND operation='open' AND raw_hex=?3 AND status='UNKNOWN'")?;
            update.bind_text(1, &request.domain_id)?;
            update.bind_text(2, &request.request_id)?;
            update.bind_text(3, &hex(&request.raw_bytes))?;
            update.bind_i64(4,next)?;
            update.step_done()?;
            failure(h::mark_active(&self.connection,&operation_id))?;
            Ok(())
        })();
        self.finish_native_transaction(applied)?;
        let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
        let result=if run.evidence.driver_id()=="claude" {
            BTreeMap::from([
                (JsonString::from_str("threadId"),run.thread_id.as_deref().map(text).unwrap_or(Json::Null)),
                (JsonString::from_str("readinessBasis"),text("ORIGINAL_CLAUDE_INITIALIZE_ACK")),
            ])
        } else {
            BTreeMap::from([(JsonString::from_str("threadId"),text(
                run.thread_id.as_deref().ok_or(OrchestrationError::Invalid("native thread absent"))?))])
        };
        Ok(encode_receipt(request, V37Status::Applied, request.expected_revision,
            request.expected_revision.checked_add(1).ok_or(OrchestrationError::Invalid("native open receipt revision overflow"))?,
            result))
    }

    fn finish_native_transaction(&mut self, result: Result<()>) -> Result<()> {
        match result {
            Ok(()) => self.connection.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause),
            Err(primary) => match self.connection.execute("ROLLBACK") {
                Ok(()) => Err(primary),
                Err(rollback) => Err(OrchestrationError::V37StoreFailure(format!("native transaction: {primary:?}; rollback: {rollback:?}"))),
            },
        }
    }

    pub(super) fn dispatch_native_stop(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        self.dispatch_native_stop_with_caller(request,None)
    }

    pub(super) fn dispatch_native_child_stop(&mut self,request:&V37Request,
        caller:&seat::NativeSeatCall)->Result<Vec<u8>> {
        // A later control call must not impersonate the original reservation.
        seat::NativeLeadAdmission::from_model_call(caller)?;
        self.dispatch_native_stop_with_caller(request,Some(caller))
    }

    fn dispatch_native_stop_with_caller(&mut self,request:&V37Request,
        caller:Option<&seat::NativeSeatCall>)->Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if request.payload.len() != 2 {
            return Ok(encode_receipt(request, V37Status::Denied,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let seat_id = user_payload_string(request, "seatId")?;
        let generation = user_payload_string(request, "generation")?;
        if let Some(caller)=caller {
            let host_id=caller.host_request_id().ok_or(OrchestrationError::AccessDenied)?;
            if request.request_id!=format!("{host_id}-stop") || request.domain_id!=caller.domain_id() {
                return Err(OrchestrationError::AccessDenied);
            }
            let child=seat::get(&self.connection,&request.domain_id,&seat_id)?
                .ok_or(OrchestrationError::AccessDenied)?;
            seat::current_child_dispatch_context(&self.connection,caller,&child)?;
            if child.generation.to_string()!=generation {return Err(OrchestrationError::OperationConflict);}
        }
        let prior = Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex,operation,session_id,status,revision FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1, &request.domain_id)?;
        prior.bind_text(2, &request.request_id)?;
        let previously_intended = if prior.step_row()? {
            if prior.column_text(0)? != hex(&request.raw_bytes) || prior.column_text(1)? != "stop"
                || prior.column_text(2)? != request.target_id {
                return Ok(encode_receipt(request, V37Status::Conflict,
                    request.expected_revision, request.expected_revision, Default::default()));
            }
            if prior.column_text(3)? == "APPLIED" {
                let revision = prior.column_text(4)?.parse::<u64>().map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("stop prior revision: {error}")))?;
                drop(prior);
                let fact = Statement::prepare(self.connection.as_ptr(),
                    "SELECT e.process_operation_id,c.stop_proof_hash
                       FROM main.gogoke_v37_h_process_episode e
                       JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id
                         AND g.session_id=e.session_id AND g.generation=e.generation
                         AND g.process_operation_id=e.process_operation_id
                       JOIN main.gogoke_coordination_process_custody c
                         ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
                         AND c.generation=e.generation
                      WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3
                        AND e.phase='STOPPED' AND c.state='STOPPED'
                        AND e.stop_fact_id=c.stop_proof_hash")?;
                fact.bind_text(1, &request.domain_id)?;
                fact.bind_text(2, &request.target_id)?;
                fact.bind_text(3, &generation)?;
                if !fact.step_row()? { return Err(OrchestrationError::OperationConflict); }
                let stopped_operation=fact.column_text(0)?;
                let hash = fact.column_text(1)?;
                if fact.step_row()? {return Err(OrchestrationError::OperationConflict);}
                drop(fact);
                let key=(request.domain_id.clone(),request.target_id.clone());
                if self.native_sessions.get(&key).is_some_and(|run|run.operation_id==stopped_operation) {
                    self.confirm_native_stop(&key)?;
                }
                return Ok(encode_receipt(request, V37Status::Replayed,
                    request.expected_revision, revision, BTreeMap::from([
                        (JsonString::from_str("stopFact"), Json::String(JsonString::from_str(&hash)))])));
            }
            true
        } else { false };
        drop(prior);
        if let Some(compound)=failure(change::active_for_session(&self.connection,
            &request.domain_id,&request.target_id))? {
            if caller.is_some() {
                return Ok(encode_receipt(request,V37Status::Conflict,
                    request.expected_revision,request.expected_revision,BTreeMap::from([
                        (JsonString::from_str("reason"),Json::String(JsonString::from_str(
                            "child generation change is unresolved")))])));
            }
            return self.dispatch_owner_stop_generation_change(request,&compound,previously_intended);
        }
        let key = (request.domain_id.clone(), request.target_id.clone());
        let Some(run) = self.native_sessions.get(&key) else {
            if previously_intended {
                // A host restart may occur after the actual stop proof was
                // committed and before the H receipt. Complete only from that
                // exact durable fact; absence/UNKNOWN never permits OS replay.
                let stopped = Statement::prepare(self.connection.as_ptr(),
                    "SELECT c.operation_id,c.stop_proof_hash FROM main.gogoke_v37_h_claim a JOIN main.gogoke_coordination_process_custody c ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id AND c.generation=a.generation JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=a.domain_id AND s.session_id=a.session_id AND s.generation=a.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3 AND s.seat_id=?4 AND c.state='STOPPED' AND c.stop_proof_hash IS NOT NULL")?;
                for (index, value) in [request.domain_id.as_str(), request.target_id.as_str(),
                    generation.as_str(), seat_id.as_str()].iter().enumerate() {
                    stopped.bind_text((index + 1) as i32, value)?;
                }
                if stopped.step_row()? {
                    let operation = stopped.column_text(0)?;
                    let hash = stopped.column_text(1)?;
                    if stopped.step_row()? { return Err(OrchestrationError::OperationConflict); }
                    drop(stopped);
                    failure(self.connection.execute("BEGIN IMMEDIATE"))?;
                    let recovered = (|| -> Result<()> {
                        authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
                        if runtime::observe_stop_fact(&self.connection, &request.domain_id, &request.target_id)?.is_none() {
                            failure(h::record_session_stop_in_transaction(&mut self.connection,
                                &request.domain_id, &request.target_id, &operation))?;
                        }
                        let update = Statement::prepare(self.connection.as_ptr(),
                            "UPDATE main.gogoke_v37_h_operation SET status='APPLIED',revision=(SELECT revision FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2) WHERE domain_id=?1 AND session_id=?2 AND request_id=?3 AND operation='stop' AND raw_hex=?4 AND status='UNKNOWN'")?;
                        update.bind_text(1, &request.domain_id)?;
                        update.bind_text(2, &request.target_id)?;
                        update.bind_text(3, &request.request_id)?;
                        update.bind_text(4, &hex(&request.raw_bytes))?;
                        update.step_done()?;
                        Ok(())
                    })();
                    self.finish_native_transaction(recovered)?;
                    return Ok(encode_receipt(request, V37Status::Replayed, request.expected_revision,
                        request.expected_revision.checked_add(1).ok_or(OrchestrationError::Invalid("stop revision overflow"))?,
                        BTreeMap::from([(JsonString::from_str("stopFact"), Json::String(JsonString::from_str(&hash)))])));
                }
            }
            return Ok(encode_receipt(request, V37Status::Unknown,
                request.expected_revision, request.expected_revision, Default::default()));
        };
        let custody = run.custody.clone();
        let operation = run.operation_id.clone();
        if !previously_intended {
            // Recover a trustworthy already-observed send before the stop
            // increments this claim. An old stop revision remains stale.
            if run.evidence.driver_id()=="codex" {
                failure(h::reconcile_observed_codex_sends(&mut self.connection,
                    &request.domain_id,&request.target_id,&generation))?;
            } else {self.drain_native_output(&key)?;}
            let claim = failure(runtime::observe_claim(&self.connection, &NativeOrigin::user(&self.owner),
                &request.domain_id, &seat_id, &request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
            if claim.generation != generation || custody.binding.generation != generation
                || claim.process_operation_id.as_deref() != Some(&operation) {
                return Ok(encode_receipt(request, V37Status::Conflict,
                    request.expected_revision, request.expected_revision, Default::default()));
            }
            if u64::try_from(claim.revision).ok() != Some(request.expected_revision) {
                return Ok(encode_receipt(request,V37Status::Stale,request.expected_revision,
                    u64::try_from(claim.revision).map_err(|error|
                        OrchestrationError::V37StoreFailure(format!("stop current revision: {error}")))?,Default::default()));
            }
            failure(self.connection.execute("BEGIN IMMEDIATE"))?;
            let intended = (|| -> Result<()> {
                authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
                if let Some(caller)=caller {
                    let child=seat::get(&self.connection,&request.domain_id,&seat_id)?
                        .ok_or(OrchestrationError::AccessDenied)?;
                    seat::current_child_dispatch_context(&self.connection,caller,&child)?;
                    let current=failure(runtime::observe_claim_bound(&self.connection,
                        &request.domain_id,&seat_id,&request.target_id))?
                        .ok_or(OrchestrationError::AccessDenied)?;
                    if current.generation!=generation || child.generation.to_string()!=generation
                        || current.instance_id!=child.instance_id
                        || current.process_operation_id.as_deref()!=Some(operation.as_str())
                        || u64::try_from(current.revision).ok()!=Some(request.expected_revision) {
                        return Err(OrchestrationError::OperationConflict);
                    }
                    let held=Statement::prepare(self.connection.as_ptr(),
                        "SELECT 1 FROM main.gogoke_coordination_process_custody
                          WHERE operation_id=?1 AND ticket=?2 AND custodian_nonce=?3
                            AND domain_id=?4 AND generation=?5 AND state='ACTIVE'")?;
                    for (index,value) in [operation.as_str(),custody.ticket.opaque(),
                        custody.custodian_nonce.as_str(),request.domain_id.as_str(),generation.as_str()]
                        .iter().enumerate() {held.bind_text((index+1) as i32,value)?;}
                    if !held.step_row()? || held.step_row()? {return Err(OrchestrationError::AccessDenied);}
                }
                let existing = Statement::prepare(self.connection.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_h_process_episode
                      WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
                        AND generation=?4 AND stop_request_id IS NULL")?;
                existing.bind_text(1, &request.domain_id)?;
                existing.bind_text(2, &request.target_id)?;
                existing.bind_text(3,&operation)?;
                existing.bind_text(4,&generation)?;
                if !existing.step_row()? || existing.step_row()? {
                    return Err(OrchestrationError::OperationConflict);
                }
                drop(existing);
                let insert = Statement::prepare(self.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,'stop',?4,'UNKNOWN',?5,?5)")?;
                for (index, value) in [request.domain_id.as_str(), request.request_id.as_str(),
                    hex(&request.raw_bytes).as_str(), request.target_id.as_str()].iter().enumerate() {
                    insert.bind_text((index + 1) as i32, value)?;
                }
                insert.bind_i64(5, claim.revision)?;
                insert.step_done()?;
                let fence=Statement::prepare(self.connection.as_ptr(),
                    "UPDATE main.gogoke_v37_h_process_episode SET stop_request_id=?1
                      WHERE domain_id=?2 AND session_id=?3 AND process_operation_id=?4
                        AND generation=?5 AND stop_request_id IS NULL")?;
                for (index,value) in [request.request_id.as_str(),request.domain_id.as_str(),
                    request.target_id.as_str(),operation.as_str(),generation.as_str()].iter().enumerate() {
                    fence.bind_text((index+1) as i32,value)?;
                }
                fence.step_done()?;
                Ok(())
            })();
            self.finish_native_transaction(intended)?;
        }
        let proof = if let Some(proof) = self.native_sessions.get(&key).and_then(|run| run.stop_proof.as_ref()) {
            proof.clone()
        } else {
            let close = self.process_custodian.close_child_input(&custody.ticket)
                .map_err(|error| format!("native session stdin close: {error}"));
            let proof = self.process_custodian.stop(&custody.ticket, StopBudgets::production(), move || close)?;
            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?.stop_proof = Some(proof.clone());
            proof
        };
        // Stop the actual Job even when A's INSERT is still failing. Keep
        // its proof, original frame and guards until that exact frame can be
        // captured; neither durable STOPPED nor custody release precedes it.
        self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?
            .raw_capture.capture(&mut self.connection,&operation,&custody.custodian_nonce)?;
        // The stopped process object and its original stdout reader still
        // belong to this custody. Project its captured tail before changing
        // the H binding or releasing any guard. A quiet reader is not EOF:
        // retain the same stop proof for readback, without another OS stop.
        self.drain_native_output(&key)?;
        if !self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?.raw_capture.source_exhausted() {
            return Err(OrchestrationError::Invalid("native stopped stdout terminal boundary not yet observed; custody retained"));
        }
        authority::mark_process_stopped(&mut self.connection, &operation, &proof)?;
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let stopped = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            // If a previous commit succeeded but the receipt was lost, read
            // the same stop fact instead of repeating an external stop.
            if runtime::observe_stop_fact(&self.connection, &request.domain_id, &request.target_id)?.is_none() {
                failure(h::record_session_stop_in_transaction(&mut self.connection,
                    &request.domain_id, &request.target_id, &operation))?;
            }
            let update = Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET status='APPLIED',revision=?1 WHERE domain_id=?2 AND request_id=?3 AND raw_hex=?4 AND status='UNKNOWN'")?;
            update.bind_i64(1, i64::try_from(request.expected_revision.checked_add(1)
                .ok_or(OrchestrationError::Invalid("stop revision overflow"))?)
                .map_err(|error| OrchestrationError::V37StoreFailure(format!("stop revision: {error}")))?)?;
            update.bind_text(2, &request.domain_id)?;
            update.bind_text(3, &request.request_id)?;
            update.bind_text(4, &hex(&request.raw_bytes))?;
            update.step_done()?;
            Ok(())
        })();
        self.finish_native_transaction(stopped)?;
        self.confirm_native_stop(&key)?;
        Ok(encode_receipt(request, V37Status::Applied, request.expected_revision,
            request.expected_revision.checked_add(1).ok_or(OrchestrationError::Invalid("stop revision overflow"))?,
            BTreeMap::from([(JsonString::from_str("stopFact"), Json::String(JsonString::from_str(&proof.proof_hash())))])))
    }

    fn confirm_native_stop(&mut self, key: &(String, String)) -> Result<()> {
        let Some(run) = self.native_sessions.get(key) else { return Ok(()); };
        let proof = run.stop_proof.as_ref().ok_or(OrchestrationError::Invalid("native durable stop proof absent"))?;
        let row = Statement::prepare(self.connection.as_ptr(),
            "SELECT rowid FROM main.gogoke_coordination_process_custody WHERE operation_id=?1 AND ticket=?2 AND custodian_nonce=?3 AND state='STOPPED' AND stop_proof_hash=?4")?;
        row.bind_text(1, &run.operation_id)?;
        row.bind_text(2, run.custody.ticket.opaque())?;
        row.bind_text(3, &run.custody.custodian_nonce)?;
        row.bind_text(4, &proof.proof_hash())?;
        if !row.step_row()? { return Err(OrchestrationError::OperationConflict); }
        let revision = row.column_text(0)?.parse::<u64>().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native stop durable revision: {error}")))?;
        drop(row);
        if self.process_custodian.active(&run.custody.ticket).is_some() {
            self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
                ticket: run.custody.ticket.clone(), custodian_nonce: run.custody.custodian_nonce.clone(),
                identity: run.custody.identity.clone(), proof_hash: proof.proof_hash(), durable_revision: revision,
            })?;
        }
        self.native_sessions.remove(key);
        Ok(())
    }

    pub(super) fn dispatch_native_send(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if request.payload.len() != 2 {
            return Ok(encode_receipt(request, V37Status::Denied, request.expected_revision,
                request.expected_revision, Default::default()));
        }
        let generation = user_payload_string(request, "generation")?;
        let text = user_payload_string(request, "body")?;
        // Read an already observed outcome before requiring a new live child.
        let prior = Statement::prepare(self.connection.as_ptr(),
            "SELECT request_hex,ticket,generation FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1, &request.domain_id)?;
        prior.bind_text(2, &request.request_id)?;
        if prior.step_row()? {
            if prior.column_text(0)? != hex(&request.raw_bytes) || prior.column_text(2)? != generation {
                return Ok(encode_receipt(request, V37Status::Conflict, request.expected_revision,
                    request.expected_revision, Default::default()));
            }
            let ticket = prior.column_text(1)?;
            drop(prior);
            let mut stored = failure(h::read_stdin_journal(&self.connection, &h::StdinJournalKey {
                domain_id: &request.domain_id, request_id: &request.request_id,
                session_id: &request.target_id, ticket: &ticket, generation: &generation,
            }))?.ok_or(OrchestrationError::Invalid("native send journal disappeared"))?;
            let driver=Statement::prepare(self.connection.as_ptr(),
                    "SELECT i.driver_id FROM main.gogoke_v37_h_process_episode e
                       JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
                      WHERE e.domain_id=?1 AND e.session_id=?2 AND e.process_operation_id=?3 AND e.generation=?4")?;
                for (index,value) in [request.domain_id.as_str(),request.target_id.as_str(),
                    stored.process_operation_id.as_str(),generation.as_str()].iter().enumerate() {driver.bind_text((index+1) as i32,value)?;}
                if !driver.step_row()? {return Err(OrchestrationError::AccessDenied);}
                let driver_id=driver.column_text(0)?;
                if driver.step_row()? {return Err(OrchestrationError::OperationConflict);}
            drop(driver);
            if stored.state != h::JournalState::Receipted {
                let input = h::StdinRequest { domain_id: &request.domain_id,
                    session_id: &request.target_id, ticket: &ticket, generation: &generation,
                    request_bytes: &request.raw_bytes };
                if driver_id=="codex" {
                    if let Some(recovered)=failure(h::recover_codex_turn_request(&mut self.connection,&input))? {
                        stored=recovered.record;
                    }
                } else {
                    let key=(request.domain_id.clone(),request.target_id.clone());
                    if self.native_sessions.get(&key).is_some_and(|run|
                        run.custody.ticket.opaque()==ticket && run.custody.binding.generation==generation) {
                        self.drain_native_output(&key)?;
                        stored=failure(h::read_stdin_journal(&self.connection,&h::StdinJournalKey {
                            domain_id:&request.domain_id,request_id:&request.request_id,
                            session_id:&request.target_id,ticket:&ticket,generation:&generation}))?
                            .ok_or(OrchestrationError::OperationConflict)?;
                    }
                }
            }
            if matches!(driver_id.as_str(),"opencode"|"grok") {
                self.verify_acp_input_receipt(&stored)?;
            } else if driver_id=="claude" {
                self.verify_claude_input_receipt(&stored)?;
            }
            if let Some(bytes) = stored.receipt_bytes {
                let receipt = failure(h::decode_receipt(&bytes))?;
                let status=if matches!(receipt.status,V37Status::Applied|V37Status::Replayed) {
                    V37Status::Replayed
                } else {receipt.status};
                return Ok(encode_receipt(request,status,
                    receipt.previous_revision,receipt.revision,receipt.into_result()));
            }
            return Ok(encode_receipt(request, V37Status::Unknown, request.expected_revision,
                request.expected_revision, Default::default()));
        }
        drop(prior);
        if failure(change::active_for_session(&self.connection,&request.domain_id,
            &request.target_id))?.is_some() {
            return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        let key = (request.domain_id.clone(), request.target_id.clone());
        if self.native_sessions.get(&key).is_some_and(|run|(run.pending_acp.is_some() || run.pending_claude.is_some())) {
            self.drain_native_output(&key)?;
        }
        let run = self.native_sessions.get(&key).ok_or(OrchestrationError::Invalid("native send has no live custody"))?;
        let seat_id = run.evidence.seat_id().to_owned();
        let current = failure(runtime::observe_claim(&self.connection, &NativeOrigin::user(&self.owner),
            &request.domain_id, &seat_id, &request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        if current.generation != generation {
            return Ok(encode_receipt(request,V37Status::Conflict,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        if u64::try_from(current.revision).ok() != Some(request.expected_revision) {
            let revision=u64::try_from(current.revision).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native input current revision: {error}")))?;
            return Ok(encode_receipt(request, V37Status::Stale, revision,
                revision, Default::default()));
        }
        failure(run.evidence.verify_live(&mut self.connection, self.root, &self.owner,
            &run.operation_id, current.revision))?;
        if matches!(run.evidence.driver_id(),"opencode"|"grok") {
            if request.operation!="send" {
                return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                    request.expected_revision,BTreeMap::from([(JsonString::from_str("reason"),
                        Json::String(JsonString::from_str("This fixed ACP transport has no append-without-turn operation")))])));
            }
            return self.dispatch_native_acp_send(request,&key);
        }
        if run.evidence.driver_id()=="claude" {
            if request.operation!="send" {
                return Ok(encode_receipt(request,V37Status::Unsupported,request.expected_revision,
                    request.expected_revision,BTreeMap::from([(JsonString::from_str("reason"),
                        Json::String(JsonString::from_str("The fixed Claude transport has no verified append-without-turn operation")))])));
            }
            return self.dispatch_native_claude_send(request,&key);
        }
        let thread_id = run.thread_id.clone().ok_or(OrchestrationError::Invalid("native send thread absent"))?;
        let command = if request.operation=="append-without-turn" {Command::AppendWithoutTurn {thread_id:thread_id.clone(),text}} else {Command::TurnStart { thread_id: thread_id.clone(),
            cwd: run.evidence.cwd().to_string_lossy().into_owned(), model: run.model.clone(),
            effort: run.effort.clone(), text, network_access: Some(run.evidence.network_access()) }};
        // Reject a command which cannot be encoded before occupying the H
        // intent. The largest legal ID bounds every ID the runtime allocates.
        failure(command.encode(Some(&failure(RpcId::client(9_007_199_254_740_991))?)))?;
        let custody = run.custody.clone();
        let input = h::StdinRequest { domain_id: &request.domain_id, session_id: &request.target_id,
            ticket: custody.ticket.opaque(), generation: &generation, request_bytes: &request.raw_bytes };
        let intention = failure(h::prepare_codex_request(&mut self.connection, &input))?;
        if intention.disposition != h::PrepareDisposition::Prepared {
            return Ok(encode_receipt(request, V37Status::Unknown, request.expected_revision,
                request.expected_revision, Default::default()));
        }
        let run = self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?;
        let number = run.next_rpc_id;
        run.next_rpc_id = number.checked_add(1).ok_or(OrchestrationError::Invalid("native RPC ordinal overflow"))?;
        let step_id = format!("{}-{}",if request.operation=="append-without-turn" {"append"} else {"send"}, &crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
        let observed = self.native_rpc_observation(&key, &step_id, Some(number), &command);
        let observation = match observed {
            Ok(Some(observation)) => observation,
            other => {
                let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
                let original = match other {
                    Err(error) => format!("{error:?}"),
                    _ => "native turn response absent".into(),
                };
                let unknown = if run.raw_capture.has_pending() { Ok(()) }
                    else { authority::mark_process_unknown(&mut self.connection, &run.operation_id, &custody) };
                let journal = h::mark_codex_write_unknown(&mut self.connection, &input);
                return Err(OrchestrationError::V37StoreFailure(format!(
                    "native send: {original}; custody UNKNOWN: {unknown:?}; original request UNKNOWN: {journal:?}")));
            }
        };
        let id = failure(RpcId::client(number))?;
        let completed = failure(h::complete_codex_turn_request(&mut self.connection, &input,
            &observation.frame, &id, &command, &thread_id))?;
        completed.record.receipt_bytes.ok_or(OrchestrationError::Invalid("native turn receipt absent"))
    }

    /// C calls this only after its same-store first-send permission commits.
    /// Derive the RPC ID and current physical/grant binding here; callers
    /// cannot redirect a message to a different thread or a later turn.
    pub(super) fn native_steer_rpc(&mut self,key:&(String,String),step_id:&str,
        expected_thread:&str,expected_turn:&str,text:String)->Result<Option<Reply>> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if run.evidence.driver_id()!="codex" {
            return Err(OrchestrationError::Invalid("fixed provider has no native in-turn/append operation"));
        }
        if !run.allows_input() || run.thread_id.as_deref()!=Some(expected_thread)
            || run.turn_id.as_deref()!=Some(expected_turn)
            || self.process_custodian.active(&run.custody.ticket).is_none() {
            return Err(OrchestrationError::AccessDenied);
        }
        let seat_id=run.evidence.seat_id();
        let claim=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,seat_id,&key.1))?.ok_or(OrchestrationError::AccessDenied)?;
        failure(run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,claim.revision))?;
        let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
        let number=run.next_rpc_id;
        run.next_rpc_id=number.checked_add(1).ok_or(OrchestrationError::Invalid("native steer RPC ordinal overflow"))?;
        self.native_rpc(key,step_id,Some(number),&Command::TurnSteer {
            thread_id:expected_thread.to_owned(),expected_turn_id:expected_turn.to_owned(),text})
    }

    /// Inbox delivery uses the same bound native writer without starting a
    /// model turn. Its C owner settles only from the original observed ACK.
    pub(super) fn native_append_rpc(&mut self,key:&(String,String),step_id:&str,
        expected_thread:&str,text:String)->Result<Option<Reply>> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if run.evidence.driver_id()!="codex" {
            return Err(OrchestrationError::Invalid("fixed provider has no native in-turn/append operation"));
        }
        if !run.allows_input() || run.thread_id.as_deref()!=Some(expected_thread)
            || self.process_custodian.active(&run.custody.ticket).is_none() {
            return Err(OrchestrationError::AccessDenied);
        }
        let claim=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,run.evidence.seat_id(),&key.1))?.ok_or(OrchestrationError::AccessDenied)?;
        failure(run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,claim.revision))?;
        let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
        let number=run.next_rpc_id;
        run.next_rpc_id=number.checked_add(1).ok_or(OrchestrationError::Invalid("native append RPC ordinal overflow"))?;
        self.native_rpc(key,step_id,Some(number),&Command::AppendWithoutTurn {thread_id:expected_thread.to_owned(),text})
    }

    /// Read only the loaded native thread's effective feature flags. The
    /// caller supplies no process, thread, cwd or grant authority.
    pub(super) fn native_feature_rpc(&mut self,key:&(String,String),step_id:&str,
        cursor:Option<String>)->Result<Option<Reply>> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if !run.allows_input() || self.process_custodian.active(&run.custody.ticket).is_none() {
            return Err(OrchestrationError::AccessDenied);
        }
        let thread_id=run.thread_id.clone().ok_or(OrchestrationError::AccessDenied)?;
        let claim=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,run.evidence.seat_id(),&key.1))?.ok_or(OrchestrationError::AccessDenied)?;
        failure(run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,claim.revision))?;
        let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
        let number=run.next_rpc_id;
        run.next_rpc_id=number.checked_add(1).ok_or(
            OrchestrationError::Invalid("native feature RPC ordinal overflow"))?;
        let unique_step=format!("{step_id}-rpc{number}");
        self.native_rpc(key,&unique_step,Some(number),&Command::FeatureList {thread_id,cursor})
    }

    /// Actual process-owned JSONL, with durable native step intent before
    /// writing and A's original provider bytes before interpreting responses.
    fn native_claude_initialize(&mut self,key:&(String,String)) -> Result<()> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let custody=run.custody.clone();let operation=run.operation_id.clone();
        let open_id=run.open_request_id.clone();let open_bytes=run.open_request_bytes.clone();
        let (step_id,request_id)=rpc::claude_initialize_identity(&open_bytes);
        let command=vendor_commands::ClaudeCommand::Initialize {request_id:&request_id};
        let step=rpc::ClaudeStep {domain_id:&key.0,session_id:&key.1,
            open_request_id:&open_id,open_request_bytes:&open_bytes,step_id:&step_id,
            custody:&custody,command:&command};
        let prepared=failure(rpc::prepare_claude(&mut self.connection,&self.owner,&step))?;
        match prepared.disposition {
            rpc::Disposition::NewWrite=>{
                let process=self.process_custodian.active(&custody.ticket)
                    .ok_or(OrchestrationError::OperationConflict)?;
                if let Err(error)=process.write_persistent_frame(&prepared.bytes) {
                    let original=self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                        crate::process::ProcessCustodyError::ProtocolPipe(error));
                    let marked=rpc::mark_claude_unknown(&mut self.connection,&self.owner,&step,&original.to_string());
                    return Err(OrchestrationError::V37StoreFailure(format!("Claude initialize write: {original}; UNKNOWN: {marked:?}")));
                }
                failure(rpc::mark_claude_written(&mut self.connection,&self.owner,&step))?;
            },
            rpc::Disposition::Existing(rpc::Phase::Written|rpc::Phase::Observed)=>{},
            _=>return Err(OrchestrationError::OperationConflict),
        }
        if let Some((observed,raw))=failure(rpc::read_observed_claude_ack(&mut self.connection,&self.owner,&step))? {
            return if matches!(observed,stream_json::ClaudeData::ControlResponse {success:true,..}) {
                Ok(())
            } else {Err(OrchestrationError::V37StoreFailure(format!("Claude original initialize failure: {}",String::from_utf8_lossy(&raw))))};
        }
        let start=Instant::now();
        loop {
            let remaining=Duration::from_secs(30).saturating_sub(start.elapsed());
            if remaining.is_zero() {return Err(OrchestrationError::Invalid("Claude initialize response deadline"));}
            let frame=self.process_custodian.read_persistent_child_frame(&custody.ticket,remaining)?;
            let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
            run.raw_capture.retain(frame)?;
            let (frame,raw)=run.raw_capture.capture(&mut self.connection,&operation,&custody.custodian_nonce)?
                .ok_or(OrchestrationError::OperationConflict)?;
            let decoded=stream_json::decode_claude_line(frame.bytes()).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("Claude initialize source: {error:?}; raw: {}",String::from_utf8_lossy(frame.bytes()))))?;
            if let stream_json::ClaudeData::ControlResponse {request_id:actual,..}=&decoded {
                if actual!=&request_id {return Err(OrchestrationError::OperationConflict);}
                let observed=failure(rpc::observe_claude_ack(&mut self.connection,&self.owner,&step,&frame,&raw.key))?;
                return if matches!(observed,stream_json::ClaudeData::ControlResponse {success:true,..}) {Ok(())}
                    else {Err(OrchestrationError::V37StoreFailure(format!("Claude initialize rejected: {}",String::from_utf8_lossy(frame.bytes()))))};
            }
        }
    }

    fn dispatch_native_claude_send(&mut self,request:&V37Request,key:&(String,String)) -> Result<Vec<u8>> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if !run.allows_input() || run.pending_claude.is_some() {return Err(OrchestrationError::OperationConflict);}
        let custody=run.custody.clone();let open_id=run.open_request_id.clone();let open_bytes=run.open_request_bytes.clone();
        let input=h::ClaudeSendInput {user:h::StdinRequest {domain_id:&request.domain_id,
            session_id:&request.target_id,ticket:custody.ticket.opaque(),generation:&custody.binding.generation,
            request_bytes:&request.raw_bytes},custody:&custody,open_request_id:&open_id,open_request_bytes:&open_bytes};
        let prepared=failure(h::prepare_claude_send_request(&mut self.connection,&self.owner,&input))?;
        if !prepared.write_permitted {return Ok(encode_receipt(request,V37Status::Unknown,
            request.expected_revision,request.expected_revision,Default::default()));}
        self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?
            .pending_claude=Some((request.raw_bytes.clone(),prepared.identity));
        let process=self.process_custodian.active(&custody.ticket).ok_or(OrchestrationError::OperationConflict)?;
        if let Err(error)=process.write_persistent_frame(&prepared.bytes) {
            let original=self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                crate::process::ProcessCustodyError::ProtocolPipe(error));
            let custody_unknown=authority::mark_process_unknown(&mut self.connection,
                &self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?.operation_id,&custody);
            let marked=h::mark_claude_send_write_unknown(&mut self.connection,&self.owner,&input,&original.to_string());
            return Err(OrchestrationError::V37StoreFailure(format!("Claude input write: {original}; custody: {custody_unknown:?}; UNKNOWN: {marked:?}")));
        }
        failure(h::mark_claude_send_written(&mut self.connection,&self.owner,&input))?;
        Ok(encode_receipt(request,V37Status::Unknown,request.expected_revision,request.expected_revision,
            BTreeMap::from([(JsonString::from_str("deliveryBasis"),text("CLAUDE_ORIGINAL_RESPONSE_PENDING"))])))
    }

    pub(super) fn complete_pending_native_claude_send(&mut self,key:&(String,String)) -> Result<()> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let Some((bytes,identity))=run.pending_claude.clone() else {return Ok(());};
        let custody=run.custody.clone();let operation=run.operation_id.clone();
        let open_id=run.open_request_id.clone();let open_bytes=run.open_request_bytes.clone();
        let input=h::ClaudeSendInput {user:h::StdinRequest {domain_id:&key.0,session_id:&key.1,
            ticket:custody.ticket.opaque(),generation:&custody.binding.generation,request_bytes:&bytes},
            custody:&custody,open_request_id:&open_id,open_request_bytes:&open_bytes};
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_cursor FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2
               AND process_ticket=?3 AND custodian_nonce=?2 AND domain_id=?4 AND session_id=?5
               AND generation=?6 ORDER BY CAST(source_cursor AS INTEGER)")?;
        for (index,value) in [operation.as_str(),custody.custodian_nonce.as_str(),custody.ticket.opaque(),
            key.0.as_str(),key.1.as_str(),custody.binding.generation.as_str()].iter().enumerate() {query.bind_text((index+1) as i32,value)?;}
        let mut cursors=Vec::new();while query.step_row()? {cursors.push(query.column_text(0)?);}drop(query);
        let mut echoed=false;
        for cursor in cursors {
            let raw=ledger::read_captured_raw_source(&self.connection,&operation,&custody.custodian_nonce,&cursor)?
                .ok_or(OrchestrationError::OperationConflict)?;
            let decoded=stream_json::decode_claude_line(&raw.raw_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("Claude pending source: {error:?}; raw: {}",String::from_utf8_lossy(&raw.raw_bytes))))?;
            match decoded {
                stream_json::ClaudeData::Init {session_id,..}=>{
                    let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
                    if run.thread_id.as_ref().is_some_and(|id|id!=&session_id) {return Err(OrchestrationError::OperationConflict);}
                    run.thread_id=Some(session_id);
                },
                stream_json::ClaudeData::UserReplay {uuid,session_id,..} if uuid==identity.uuid=>{
                    failure(h::observe_claude_send_echo_from_source(&mut self.connection,&self.owner,&input,&raw.key))?;
                    let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
                    if run.thread_id.as_ref().is_some_and(|id|id!=&session_id) {return Err(OrchestrationError::OperationConflict);}
                    run.thread_id=Some(session_id);echoed=true;
                },
                stream_json::ClaudeData::Result {..} if echoed=>{
                    let completed=failure(h::complete_claude_send_from_source(&mut self.connection,&self.owner,&input,&raw.key))?;
                    if completed.user.record.receipt_bytes.is_none() {return Err(OrchestrationError::OperationConflict);}
                    let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
                    run.thread_id=Some(completed.vendor_session_id);run.pending_claude=None;return Ok(());
                },
                _=>{},
            }
        }
        Ok(())
    }

    fn dispatch_native_acp_send(&mut self,request:&V37Request,key:&(String,String)) -> Result<Vec<u8>> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if !run.allows_input() || run.pending_acp.is_some() {return Err(OrchestrationError::OperationConflict);}
        let custody=run.custody.clone();let open_id=run.open_request_id.clone();
        let open_bytes=run.open_request_bytes.clone();let operation=run.operation_id.clone();
        let input=h::AcpSendInput {user:h::StdinRequest {domain_id:&key.0,session_id:&key.1,
            ticket:custody.ticket.opaque(),generation:&custody.binding.generation,
            request_bytes:&request.raw_bytes},custody:&custody,
            open_request_id:&open_id,open_request_bytes:&open_bytes};
        let prepared=failure(h::prepare_acp_send_request(&mut self.connection,&self.owner,&input))?;
        if !prepared.write_permitted {
            return Ok(encode_receipt(request,V37Status::Unknown,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        // Hold the exact original input before touching the physical pipe.
        self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?
            .pending_acp=Some((request.raw_bytes.clone(),prepared.identity));
        let process=self.process_custodian.active(&custody.ticket)
            .ok_or(OrchestrationError::Invalid("native ACP process absent"))?;
        if let Err(error)=process.write_persistent_frame(&prepared.bytes) {
            let original=self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                crate::process::ProcessCustodyError::ProtocolPipe(error));
            let unknown=authority::mark_process_unknown(&mut self.connection,&operation,&custody);
            let journal=h::mark_acp_send_write_unknown(&mut self.connection,&self.owner,&input,&original.to_string());
            return Err(OrchestrationError::V37StoreFailure(format!(
                "native ACP send write: {original}; custody UNKNOWN: {unknown:?}; request UNKNOWN: {journal:?}")));
        }
        failure(h::mark_acp_send_written(&mut self.connection,&self.owner,&input))?;
        // Waiting is a pending original response, never an uncertain write.
        Ok(encode_receipt(request,V37Status::Unknown,request.expected_revision,request.expected_revision,
            BTreeMap::from([(JsonString::from_str("deliveryBasis"),text("ACP_PROMPT_RESPONSE_PENDING"))])))
    }

    pub(super) fn complete_pending_native_acp_send(&mut self,key:&(String,String)) -> Result<()> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let Some((request_bytes,identity))=run.pending_acp.clone() else {return Ok(());};
        let custody=run.custody.clone();let operation=run.operation_id.clone();
        let open_id=run.open_request_id.clone();let open_bytes=run.open_request_bytes.clone();
        let expected=match &identity.rpc_id {
            acp::RpcId::Number(value)=>Json::Number(value.to_string()),
            acp::RpcId::String(value)=>text(value),
        }.canonical();
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT source_cursor FROM main.v37_ledger_raw_source
              WHERE operation_id=?1 AND source_epoch=?2
                AND (state='PENDING' OR (state='NO_EVENT' AND no_event_reason='ACP_RPC_RESPONSE'))
              ORDER BY CAST(source_cursor AS INTEGER)")?;
        query.bind_text(1,&operation)?;query.bind_text(2,&custody.custodian_nonce)?;
        let mut cursors=Vec::new();while query.step_row()? {cursors.push(query.column_text(0)?);}drop(query);
        let mut matching=None;
        for cursor in cursors {
            let raw=ledger::read_captured_raw_source(&self.connection,&operation,&custody.custodian_nonce,&cursor)?
                .ok_or(OrchestrationError::OperationConflict)?;
            let value=std::str::from_utf8(&raw.raw_bytes).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native ACP original source UTF-8: {error}")))?;
            let Json::Object(fields)=Parser::parse(value)? else {continue;};
            if !fields.contains_key(&JsonString::from_str("method"))
                && fields.get(&JsonString::from_str("id")).map(Json::canonical)==Some(expected.clone())
                && (fields.contains_key(&JsonString::from_str("result")) || fields.contains_key(&JsonString::from_str("error"))) {
                if matching.is_some() {return Err(OrchestrationError::OperationConflict);}
                matching=Some(raw);
            }
        }
        let Some(raw)=matching else {return Ok(());};
        let user=h::StdinRequest {domain_id:&key.0,session_id:&key.1,ticket:custody.ticket.opaque(),
            generation:&custody.binding.generation,request_bytes:&request_bytes};
        let completed=if raw.state==ledger::RawSourceState::NoEvent {
            failure(h::read_acp_send_completed(&self.connection,&user,&raw.key))?
                .ok_or(OrchestrationError::OperationConflict)?
        } else {
            failure(h::complete_acp_send_from_source(&mut self.connection,&self.owner,
                &h::AcpSendInput {user,custody:&custody,open_request_id:&open_id,
                    open_request_bytes:&open_bytes},&raw.key))?
        };
        if completed.user.record.receipt_bytes.is_none() {return Err(OrchestrationError::OperationConflict);}
        self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?.pending_acp=None;
        Ok(())
    }

    pub(super) fn native_claude_readiness(&self,key:&(String,String)) -> Result<bool> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        Ok(matches!(failure(rpc::read_original_claude_initialize_ack(&self.connection,
            &key.0,&key.1,&run.open_request_id,&run.open_request_bytes,
            &run.operation_id,&run.custody.binding.generation))?,
            Some(stream_json::ClaudeData::ControlResponse {success:true,..})))
    }

    fn original_claude_open_ready(&self,request:&V37Request) -> Result<bool> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT e.process_operation_id,e.generation,i.driver_id
               FROM main.gogoke_v37_h_process_episode e
               JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
              WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
                AND e.raw_hex=?4 AND e.old_generation IS NULL")?;
        for (index,value) in [request.domain_id.as_str(),request.target_id.as_str(),
            request.request_id.as_str(),hex(&request.raw_bytes).as_str()].iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
        if !q.step_row()? {return Ok(false);}
        let operation=q.column_text(0)?;let generation=q.column_text(1)?;let driver=q.column_text(2)?;
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}drop(q);
        if driver!="claude" {return Ok(false);}
        Ok(matches!(failure(rpc::read_original_claude_initialize_ack(&self.connection,
            &request.domain_id,&request.target_id,&request.request_id,&request.raw_bytes,
            &operation,&generation))?,Some(stream_json::ClaudeData::ControlResponse {success:true,..})))
    }

    pub(super) fn verify_claude_input_receipt(&self,record:&h::StdinJournalRecord) -> Result<()> {
        if record.state!=h::JournalState::Receipted {return Ok(());}
        let original=h::StdinRequest {domain_id:&record.domain_id,session_id:&record.session_id,
            ticket:&record.ticket,generation:&record.generation,request_bytes:&record.request_bytes};
        let verified=failure(h::read_original_claude_send_completed(&self.connection,&original))?
            .ok_or(OrchestrationError::OperationConflict)?;
        if verified.user.record.receipt_bytes!=record.receipt_bytes {return Err(OrchestrationError::OperationConflict);}
        Ok(())
    }

    // Public replay/output must use the same original User/A derivation as
    // pending completion. Receipt source fields are lookup hints only: H checks
    // the original command, typed ID, episode, revisions and receipt digest.
    pub(super) fn verify_acp_input_receipt(&self,record:&h::StdinJournalRecord) -> Result<()> {
        if record.state!=h::JournalState::Receipted {return Ok(());}
        let receipt=failure(h::decode_receipt(record.receipt_bytes.as_ref()
            .ok_or(OrchestrationError::OperationConflict)?))?;
        let result=receipt.into_result();
        let field=|name|->Result<String> {
            match result.get(&JsonString::from_str(name)) {
                Some(Json::String(value))=>value.to_well_formed_string()
                    .filter(|value|!value.is_empty())
                    .ok_or(OrchestrationError::OperationConflict),
                _=>Err(OrchestrationError::OperationConflict),
            }
        };
        let key=ledger::RawSourceKey {operation_id:record.process_operation_id.clone(),
            source_epoch:field("sourceEpoch")?,source_cursor:field("sourceCursor")?};
        let original=h::StdinRequest {domain_id:&record.domain_id,session_id:&record.session_id,
            ticket:&record.ticket,generation:&record.generation,request_bytes:&record.request_bytes};
        let verified=failure(h::read_acp_send_completed(&self.connection,&original,&key))?
            .ok_or(OrchestrationError::OperationConflict)?;
        if verified.user.record.receipt_bytes!=record.receipt_bytes {
            return Err(OrchestrationError::OperationConflict);
        }
        Ok(())
    }

    pub(super) fn native_acp_declaration(&self,key:&(String,String)) -> Result<(Json,Json)> {
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes),r.source_epoch,r.source_cursor
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
                AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.generation=?6
                AND s.step_id IN ('initialize',?7) AND s.phase='OBSERVED'
                AND r.state='NO_EVENT' AND r.no_event_reason='ACP_RPC_RESPONSE'")?;
        for (index,value) in [key.0.as_str(),key.1.as_str(),run.operation_id.as_str(),
            run.custody.ticket.opaque(),run.custody.custodian_nonce.as_str(),
            run.custody.binding.generation.as_str()].iter().enumerate() {query.bind_text((index+1) as i32,value)?;}
        query.bind_text(7,&format!("{}-initialize",run.operation_id))?;
        if !query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let command=unhex(&query.column_text(0)?)?;let response=unhex(&query.column_text(1)?)?;
        let source_epoch=query.column_text(2)?;let source_cursor=query.column_text(3)?;
        if query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        let Json::Object(fields)=Parser::parse(std::str::from_utf8(&command).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native ACP initialize command: {error}")))?)? else {
            return Err(OrchestrationError::Invalid("native ACP initialize object"));
        };
        if fields.get(&JsonString::from_str("method")).map(Json::canonical)!=Some(text("initialize").canonical()) {
            return Err(OrchestrationError::OperationConflict);
        }
        let id=match fields.get(&JsonString::from_str("id")) {
            Some(Json::Number(value))=>acp::RpcId::Number(value.parse::<i64>().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native ACP initialize ID: {error}")))?),
            Some(Json::String(value))=>acp::RpcId::String(value.to_well_formed_string()
                .ok_or(OrchestrationError::Invalid("native ACP initialize ID"))?),
            _=>return Err(OrchestrationError::Invalid("native ACP initialize ID")),
        };
        let pending=acp::Pending {id:&id,method:acp::PendingMethod::Initialize,requested_session_id:None};
        let acp::Observation::Initialize {declared_capabilities,..}=acp::decode(&response,Some(&pending))
            .map_err(|error|OrchestrationError::V37StoreFailure(format!(
                "native ACP initialize response: {}; raw: {}",error.reason,String::from_utf8_lossy(&error.raw_frame))))?
            else {return Err(OrchestrationError::OperationConflict);};
        let source=Json::Object(BTreeMap::from([
            (JsonString::from_str("operationId"),text(&run.operation_id)),
            (JsonString::from_str("sourceEpoch"),text(&source_epoch)),
            (JsonString::from_str("sourceCursor"),text(&source_cursor)),
            (JsonString::from_str("rawResponseSha256"),text(&crate::store::digest::sha256_hex(&response))),
        ]));
        Ok((declared_capabilities,source))
    }

    fn observed_native_open_thread(&self,domain:&str,session:&str,open_request:&str)
        -> Result<Option<String>> {
        let query=Statement::prepare(self.connection.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes),i.driver_id,r.no_event_reason
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
               JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=s.process_operation_id
                 AND e.domain_id=s.domain_id AND e.session_id=s.session_id AND e.generation=s.generation
               JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.open_request_id=?3
                AND s.step_id='thread-start' AND s.phase='OBSERVED' AND r.state='NO_EVENT'")?;
        query.bind_text(1,domain)?;query.bind_text(2,session)?;query.bind_text(3,open_request)?;
        if !query.step_row()? {return Ok(None);}
        let command=unhex(&query.column_text(0)?)?;
        let response=unhex(&query.column_text(1)?)?;
        let driver=query.column_text(2)?;let basis=query.column_text(3)?;
        if query.step_row()? {return Err(OrchestrationError::OperationConflict);}
        if driver=="codex" && basis=="CODEX_RPC_RESPONSE" {
            return failure(codex_rpc::decode_stored_thread_start(&command,&response)).map(Some);
        }
        if !matches!(driver.as_str(),"opencode"|"grok") || basis!="ACP_RPC_RESPONSE" {
            return Err(OrchestrationError::OperationConflict);
        }
        let Json::Object(fields)=Parser::parse(std::str::from_utf8(&command).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native ACP original command: {error}")))?)? else {
            return Err(OrchestrationError::Invalid("native ACP command object"));
        };
        if fields.get(&JsonString::from_str("method")).map(Json::canonical)!=Some(text("session/new").canonical()) {
            return Err(OrchestrationError::OperationConflict);
        }
        let id=match fields.get(&JsonString::from_str("id")) {
            Some(Json::Number(value))=>acp::RpcId::Number(value.parse::<i64>().map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native ACP original ID: {error}")))?),
            Some(Json::String(value))=>acp::RpcId::String(value.to_well_formed_string()
                .ok_or(OrchestrationError::Invalid("native ACP original ID"))?),
            _=>return Err(OrchestrationError::Invalid("native ACP original ID")),
        };
        let pending=acp::Pending {id:&id,method:acp::PendingMethod::SessionNew,requested_session_id:None};
        match acp::decode(&response,Some(&pending)).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native ACP original reply: {}; raw: {}",
                error.reason,String::from_utf8_lossy(&error.raw_frame))))? {
            acp::Observation::SessionNew {session_id,..}=>Ok(Some(session_id)),
            acp::Observation::RemoteError {..}=>Ok(None),
            _=>Err(OrchestrationError::OperationConflict),
        }
    }

    fn native_acp_write(&mut self,key:&(String,String),step_id:&str,
        number:Option<u64>,command:&vendor_commands::AcpCommand<'_>) -> Result<()> {
        let id=number.map(|number|i64::try_from(number).map(acp::RpcId::Number))
            .transpose().map_err(|error|OrchestrationError::V37StoreFailure(format!("native ACP ID: {error}")))?;
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if !run.allows_input() {return Err(OrchestrationError::OperationConflict);}
        let claim=failure(runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
            &key.0,run.evidence.seat_id(),&key.1))?.ok_or(OrchestrationError::AccessDenied)?;
        failure(run.evidence.verify_live(&mut self.connection,self.root,&self.owner,
            &run.operation_id,claim.revision))?;
        let custody=run.custody.clone();let open_id=run.open_request_id.clone();
        let open_bytes=run.open_request_bytes.clone();
        let step=rpc::AcpStep {domain_id:&key.0,session_id:&key.1,
            open_request_id:&open_id,open_request_bytes:&open_bytes,
            step_id,custody:&custody,rpc_id:id.as_ref(),command};
        let intention=failure(rpc::prepare_acp(&mut self.connection,&self.owner,&step))?;
        match intention.disposition {
            rpc::Disposition::NewWrite=>{},
            rpc::Disposition::Existing(rpc::Phase::Written|rpc::Phase::Observed)=>return Ok(()),
            _=>return Err(OrchestrationError::Invalid("native ACP uncertain replay cannot write")),
        }
        let process=self.process_custodian.active(&custody.ticket)
            .ok_or(OrchestrationError::Invalid("native ACP process absent"))?;
        if let Err(error)=process.write_persistent_frame(&intention.bytes) {
            let original=self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                crate::process::ProcessCustodyError::ProtocolPipe(error));
            let persisted=rpc::mark_acp_unknown(&mut self.connection,&self.owner,&step,&original.to_string());
            return Err(OrchestrationError::V37StoreFailure(format!("native ACP write: {original}; UNKNOWN: {persisted:?}")));
        }
        failure(rpc::mark_acp_written(&mut self.connection,&self.owner,&step))
    }

    // Only finite metadata RPCs wait here. A model prompt is written once and
    // completed later from A by output polling; it never uses this deadline.
    fn native_acp_rpc(&mut self,key:&(String,String),step_id:&str,number:Option<u64>,
        command:&vendor_commands::AcpCommand<'_>) -> Result<Option<acp::Observation>> {
        self.native_acp_write(key,step_id,number,command)?;
        let Some(number)=number else {return Ok(None);};
        let id=acp::RpcId::Number(i64::try_from(number).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native ACP ID: {error}")))?);
        let (method,requested)=match command {
            vendor_commands::AcpCommand::Initialize {..}=>(acp::PendingMethod::Initialize,None),
            vendor_commands::AcpCommand::SessionNew {..}=>(acp::PendingMethod::SessionNew,None),
            vendor_commands::AcpCommand::SessionLoad {session_id,..}=>(acp::PendingMethod::SessionLoad,Some(*session_id)),
            vendor_commands::AcpCommand::SessionResume {session_id,..}=>(acp::PendingMethod::SessionResume,Some(*session_id)),
            vendor_commands::AcpCommand::SetConfigOption {session_id,..}=>(acp::PendingMethod::SessionSetConfigOption,Some(*session_id)),
            _=>return Err(OrchestrationError::Invalid("native ACP metadata method")),
        };
        let pending=acp::Pending {id:&id,method,requested_session_id:requested};
        let run=self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        let custody=run.custody.clone();let operation=run.operation_id.clone();
        let open_id=run.open_request_id.clone();let open_bytes=run.open_request_bytes.clone();
        let step=rpc::AcpStep {domain_id:&key.0,session_id:&key.1,open_request_id:&open_id,
            open_request_bytes:&open_bytes,step_id,custody:&custody,rpc_id:Some(&id),command};
        if let Some((observed,_raw))=failure(rpc::read_observed_acp_response(
            &mut self.connection,&self.owner,&step))? {
            if let acp::Observation::RemoteError {raw_frame,..}=&observed {
                return Err(OrchestrationError::V37StoreFailure(format!("native ACP original remote error: {}",
                    String::from_utf8_lossy(&raw_frame[raw_frame.len().saturating_sub(4096)..]))));
            }
            return Ok(Some(observed));
        }
        let start=Instant::now();
        loop {
            let remaining=Duration::from_secs(30).saturating_sub(start.elapsed());
            if remaining.is_zero() {return Err(OrchestrationError::Invalid("native ACP metadata response deadline"));}
            let frame=self.process_custodian.read_persistent_child_frame(&custody.ticket,remaining)?;
            let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
            run.raw_capture.retain(frame)?;
            let (frame,raw)=run.raw_capture.capture(&mut self.connection,&operation,&custody.custodian_nonce)?
                .ok_or(OrchestrationError::Invalid("native ACP source absent"))?;
            let observed=acp::decode(frame.bytes(),Some(&pending)).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("native ACP metadata reply: {}; raw: {}",
                    error.reason,String::from_utf8_lossy(&error.raw_frame))))?;
            if matches!(&observed,acp::Observation::Initialize {..}|acp::Observation::SessionNew {..}
                |acp::Observation::SessionLoad {..}|acp::Observation::SessionResume {..}
                |acp::Observation::SessionConfigOption {..}|acp::Observation::RemoteError {..}) {
                let observed=failure(rpc::observe_acp_response(&mut self.connection,&self.owner,&step,&frame,&raw.key))?;
                if let acp::Observation::RemoteError {raw_frame,..}=&observed {
                    return Err(OrchestrationError::V37StoreFailure(format!("native ACP remote error: {}",
                        String::from_utf8_lossy(&raw_frame[raw_frame.len().saturating_sub(4096)..]))));
                }
                return Ok(Some(observed));
            }
            // Source notifications remain in the same A stream. A session/new
            // reply, not an early update, establishes the vendor binding.
        }
    }

    pub(super) fn native_rpc(&mut self, key: &(String, String), step_id: &str,
        number: Option<u64>, command: &Command) -> Result<Option<Reply>> {
        self.native_rpc_observation(key, step_id, number, command).map(|result| result.map(|result| result.reply))
    }

    fn native_rpc_observation(&mut self, key: &(String, String), step_id: &str,
        number: Option<u64>, command: &Command) -> Result<Option<RpcObservation>> {
        let id = number.map(RpcId::client).transpose().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native RPC ID: {error:?}")))?;
        let run = self.native_sessions.get(key).ok_or(OrchestrationError::AccessDenied)?;
        if run.stop_proof.is_some() {
            return Err(OrchestrationError::Invalid("native RPC stop is awaiting durable reconciliation"));
        }
        if run.raw_capture.has_pending() {
            return Err(OrchestrationError::Invalid("native RPC has retained uncaptured source"));
        }
        let custody=run.custody.clone();
        let operation=run.operation_id.clone();
        let open_id=run.open_request_id.clone();
        let open_bytes=run.open_request_bytes.clone();
        let step = rpc::Step { domain_id: &key.0, session_id: &key.1,
            open_request_id: &open_id, open_request_bytes: &open_bytes,
            step_id, custody: &custody, rpc_id: id.as_ref(), command };
        let intention = failure(rpc::prepare(&mut self.connection, &self.owner, &step))?;
        if intention.disposition != rpc::Disposition::NewWrite {
            return Err(OrchestrationError::Invalid("native RPC replay cannot write"));
        }
        let process = self.process_custodian.active(&custody.ticket)
            .ok_or(OrchestrationError::Invalid("native RPC process absent"))?;
        if let Err(error) = process.write_persistent_frame(&intention.bytes) {
            let original = self.process_custodian.protocol_error_with_stderr(&custody.ticket,
                crate::process::ProcessCustodyError::ProtocolPipe(error));
            let persisted = rpc::mark_unknown(&mut self.connection, &self.owner, &step, &original.to_string());
            return Err(OrchestrationError::V37StoreFailure(format!("native RPC write: {original}; UNKNOWN: {persisted:?}")));
        }
        failure(rpc::mark_written(&mut self.connection, &self.owner, &step))?;
        if id.is_none() { return Ok(None); }
        let start = Instant::now();
        loop {
            let remaining = Duration::from_secs(30).saturating_sub(start.elapsed());
            if remaining.is_zero() { return Err(OrchestrationError::Invalid("native RPC response deadline")); }
            let frame = self.process_custodian.read_persistent_child_frame(&custody.ticket, remaining)?;
            let run=self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
            run.raw_capture.retain(frame)?;
            let (frame,raw)=run.raw_capture.capture(&mut self.connection,&operation,&custody.custodian_nonce)?
                .ok_or(OrchestrationError::Invalid("native RPC capture absent"))?;
            let observed = failure(codex_rpc::decode(frame.bytes(), id.as_ref().map(|id| (id, command))))?;
            match observed {
                Reply::Initialized { .. } | Reply::MemoryOff { .. } | Reply::Thread { .. }
                | Reply::Turn { .. } | Reply::Ack { .. } | Reply::FeaturePage { .. } => {
                    if let Reply::Thread {cwd,..}=&observed {
                        failure(run.evidence.verify_observed_cwd(cwd))?;
                    }
                    let reply = failure(rpc::complete_response(&mut self.connection, &self.owner, &step, &frame, &raw.key))?;
                    if let Reply::Turn {turn_id,status:codex_rpc::TurnStatus::InProgress,..}=&reply {
                        self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?.turn_id=Some(turn_id.clone());
                        self.process_native_pending_output(key)?;
                    }
                    return Ok(Some(RpcObservation { reply, frame }));
                }
                Reply::RemoteError { raw_frame, .. } => {
                    let text = String::from_utf8_lossy(&raw_frame[raw_frame.len().saturating_sub(4096)..]);
                    // A provider error is an observed response. Preserve its
                    // original A source and the exact step correlation before
                    // reporting it; no UNKNOWN resend is authorized.
                    let persisted=rpc::complete_response(&mut self.connection,&self.owner,
                        &step,&frame,&raw.key);
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "native RPC remote error: {text}; observed journal: {persisted:?}")));
                }
                _ => {
                    failure(rpc::observe_event(&mut self.connection, &frame, &raw.key, &step))?;
                    let thread_observed=run.thread_id.is_some();
                    if thread_observed { self.process_native_pending_output(key)?; }
                }
            }
        }
    }
}

#[cfg(all(test, windows))]
#[path = "v37_runtime_tests.rs"]
mod tests;
