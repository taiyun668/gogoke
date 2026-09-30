//! Actual native Codex sessions. The User channel selects logical identities;
//! E/F/H supply all paths, pins, permissions and custody on the same store.
use super::*;
use crate::process::{PreparedCustody, DurableStopConfirmation, StopBudgets};
use crate::store::ledger::{self, SessionPurpose, SessionRegistration};
use crate::store::seat::{self, NativeOrigin};
use crate::store::session_transport::{self as h, runtime, launch::LaunchEvidence,
    codex_rpc::{self, Command, RpcId, Reply}, rpc_journal as rpc};
use crate::store::atomic::Parser;
use std::time::{Duration, Instant};

pub(super) struct NativeSession {
    evidence: LaunchEvidence,
    custody: PreparedCustody,
    operation_id: String,
    open_request_id: String,
    open_request_bytes: Vec<u8>,
    domain_id: String,
    session_id: String,
    model: String,
    effort: String,
    thread_id: Option<String>,
    raw_cursor: u64,
}

fn failure<T, E: std::fmt::Debug>(result: std::result::Result<T, E>) -> Result<T> {
    result.map_err(|error| OrchestrationError::V37StoreFailure(format!("native session: {error:?}")))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl<'root> ProductDatabase<'root> {
    /// Observe the original durable outcome before preparing another process.
    /// UNKNOWN cannot be converted into a launch by changing a request ID.
    pub(super) fn dispatch_native_open(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if request.payload.len() != 4 {
            return Ok(encode_receipt(request, V37Status::Denied,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let seat_id = user_payload_string(request, "seatId")?;
        let generation = user_payload_string(request, "generation")?;
        let repository_id = user_payload_string(request, "repositoryId")?;
        let worktree_id = user_payload_string(request, "worktreeId")?;
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
                // A stop fact alone does not prove the earlier handshake
                // succeeded. Only its actual observed thread response does.
                let response = Statement::prepare(self.connection.as_ptr(),
                    "SELECT r.raw_bytes FROM main.gogoke_v37_rpc_steps s JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor WHERE s.domain_id=?1 AND s.session_id=?2 AND s.open_request_id=?3 AND s.step_id='thread-start' AND s.phase='OBSERVED'")?;
                response.bind_text(1, &request.domain_id)?;
                response.bind_text(2, &request.target_id)?;
                response.bind_text(3, &request.request_id)?;
                if response.step_row()? {
                    let Json::Object(fields) = Parser::parse(&response.column_text(0)?)? else {
                        return Err(OrchestrationError::Invalid("native prior thread response"));
                    };
                    let thread_id = match fields.get(&JsonString::from_str("result")) {
                        Some(Json::Object(result)) => match result.get(&JsonString::from_str("thread")) {
                            Some(Json::Object(thread)) => thread.get(&JsonString::from_str("id")).cloned(),
                            _ => None,
                        },
                        _ => None,
                    }.ok_or(OrchestrationError::Invalid("native prior thread identity"))?;
                    if !matches!(thread_id, Json::String(_)) {
                        return Err(OrchestrationError::Invalid("native prior thread identity shape"));
                    }
                    result.insert(JsonString::from_str("threadId"), thread_id);
                } else { status = V37Status::Unknown; }
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
        let evidence = failure(LaunchEvidence::observe(&mut self.connection, self.root,
            &self.owner, &request.domain_id, &seat_id, &request.target_id, &repository_id, &worktree_id))?;
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
            failure(evidence.verify(&mut self.connection, self.root, &self.owner, None))?;
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
            session_id: request.target_id.clone(), model, effort, thread_id: None, raw_cursor: 0 });
        if let Err(error) = authority::record_prepared_process(&mut self.connection, &operation_id, &custody) {
            let abort = self.process_custodian.abort_prepared(&custody);
            return Err(OrchestrationError::V37StoreFailure(format!(
                "native open PREPARED record: {error:?}; abort: {abort:?}")));
        }
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let bind = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify(&mut self.connection, self.root, &self.owner, None))?;
            failure(h::bind_process_operation_in_transaction(&mut self.connection,
                &request.domain_id, &request.target_id, &operation_id))?;
            ledger::register_session(&mut self.connection, &SessionRegistration {
                domain_id: request.domain_id.clone(), seat_id: seat_id.clone(), session_id: request.target_id.clone(),
                purpose: SessionPurpose::Work, side_id: None,
            })?;
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
            self.native_rpc(&key, "initialize", Some(1), &Command::Initialize { client_version: "0.1.0".into() })?;
            self.native_rpc(&key, "initialized", None, &Command::Initialized)?;
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            let cwd = run.evidence.cwd().to_string_lossy().into_owned();
            let model = run.model.clone();
            self.native_rpc(&key, "config-read", Some(2), &Command::ConfigRead { cwd: cwd.clone() })?;
            let thread = self.native_rpc(&key, "thread-start", Some(3), &Command::ThreadStart { cwd, model })?;
            let Some(Reply::Thread { thread_id, .. }) = thread else {
                return Err(OrchestrationError::Invalid("native open thread response"));
            };
            self.native_sessions.get_mut(&key).ok_or(OrchestrationError::AccessDenied)?.thread_id = Some(thread_id);
            Ok(())
        })();
        if let Err(error) = started {
            // Keep the original intention UNKNOWN even if cleanup succeeds;
            // do not make a failed handshake an APPLIED open via a stop receipt.
            let unknown = authority::mark_process_unknown(&mut self.connection, &operation_id, &custody);
            return Err(OrchestrationError::V37StoreFailure(format!("native open handshake: {error:?}; UNKNOWN record: {unknown:?}")));
        }
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let applied = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
            failure(run.evidence.verify(&mut self.connection, self.root, &self.owner, Some(&operation_id)))?;
            let update = Statement::prepare(self.connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET status='APPLIED' WHERE domain_id=?1 AND request_id=?2 AND operation='open' AND raw_hex=?3 AND status='UNKNOWN'")?;
            update.bind_text(1, &request.domain_id)?;
            update.bind_text(2, &request.request_id)?;
            update.bind_text(3, &hex(&request.raw_bytes))?;
            update.step_done()?;
            Ok(())
        })();
        self.finish_native_transaction(applied)?;
        let run = self.native_sessions.get(&key).ok_or(OrchestrationError::AccessDenied)?;
        Ok(encode_receipt(request, V37Status::Applied, request.expected_revision, request.expected_revision,
            BTreeMap::from([(JsonString::from_str("threadId"), Json::String(JsonString::from_str(
                run.thread_id.as_deref().ok_or(OrchestrationError::Invalid("native thread absent"))?)))])))
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
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if request.payload.len() != 2 {
            return Ok(encode_receipt(request, V37Status::Denied,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let seat_id = user_payload_string(request, "seatId")?;
        let generation = user_payload_string(request, "generation")?;
        let key = (request.domain_id.clone(), request.target_id.clone());
        let run = self.native_sessions.get(&key).ok_or(OrchestrationError::Invalid("native stop has no live custody"))?;
        let claim = failure(runtime::observe_claim(&self.connection, &NativeOrigin::user(&self.owner),
            &request.domain_id, &seat_id, &request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        if claim.generation != generation || run.custody.binding.generation != generation
            || claim.process_operation_id.as_deref() != Some(&run.operation_id)
            || u64::try_from(claim.revision).ok() != Some(request.expected_revision) {
            return Ok(encode_receipt(request, V37Status::Conflict,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let custody = run.custody.clone();
        let operation = run.operation_id.clone();
        let close = self.process_custodian.close_child_input(&custody.ticket)
            .map_err(|error| format!("native session stdin close: {error}"));
        let proof = self.process_custodian.stop(&custody.ticket, StopBudgets::production(), move || close)?;
        let durable = authority::mark_process_stopped(&mut self.connection, &operation, &proof)?;
        failure(self.connection.execute("BEGIN IMMEDIATE"))?;
        let stopped = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            failure(h::record_session_stop_in_transaction(&mut self.connection,
                &request.domain_id, &request.target_id, &operation))?;
            Ok(())
        })();
        self.finish_native_transaction(stopped)?;
        self.process_custodian.confirm_stop_durable(&DurableStopConfirmation {
            ticket: custody.ticket.clone(), custodian_nonce: custody.custodian_nonce.clone(),
            identity: custody.identity.clone(), proof_hash: proof.proof_hash(), durable_revision: durable,
        })?;
        self.native_sessions.remove(&key);
        Ok(encode_receipt(request, V37Status::Applied, request.expected_revision,
            request.expected_revision.checked_add(1).ok_or(OrchestrationError::Invalid("stop revision overflow"))?,
            BTreeMap::from([(JsonString::from_str("stopFact"), Json::String(JsonString::from_str(&proof.proof_hash())))])))
    }

    /// Actual process-owned JSONL, with durable native step intent before
    /// writing and A's original provider bytes before interpreting responses.
    fn native_rpc(&mut self, key: &(String, String), step_id: &str,
        number: Option<u64>, command: &Command) -> Result<Option<Reply>> {
        let id = number.map(RpcId::client).transpose().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("native RPC ID: {error:?}")))?;
        let run = self.native_sessions.get_mut(key).ok_or(OrchestrationError::AccessDenied)?;
        let step = rpc::Step { domain_id: &run.domain_id, session_id: &run.session_id,
            open_request_id: &run.open_request_id, open_request_bytes: &run.open_request_bytes,
            step_id, custody: &run.custody, rpc_id: id.as_ref(), command };
        let intention = failure(rpc::prepare(&mut self.connection, &self.owner, &step))?;
        if intention.disposition != rpc::Disposition::NewWrite {
            return Err(OrchestrationError::Invalid("native RPC replay cannot write"));
        }
        let process = self.process_custodian.active(&run.custody.ticket)
            .ok_or(OrchestrationError::Invalid("native RPC process absent"))?;
        if let Err(error) = process.write_persistent_frame(&intention.bytes) {
            let original = self.process_custodian.protocol_error_with_stderr(&run.custody.ticket,
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
            let frame = self.process_custodian.read_persistent_child_frame(&run.custody.ticket, remaining)?;
            run.raw_cursor = run.raw_cursor.checked_add(1).ok_or(OrchestrationError::Invalid("native raw cursor overflow"))?;
            let raw = ledger::capture_raw_source(&mut self.connection, &frame, &run.operation_id,
                &run.custody.custodian_nonce, &run.raw_cursor.to_string())?;
            let observed = failure(codex_rpc::decode(frame.bytes(), id.as_ref().map(|id| (id, command))))?;
            match observed {
                Reply::Initialized { .. } | Reply::MemoryOff { .. } | Reply::Thread { .. }
                | Reply::Turn { .. } | Reply::Ack { .. } => {
                    return failure(rpc::complete_response(&mut self.connection, &self.owner, &step, &frame, &raw.key)).map(Some);
                }
                Reply::RemoteError { raw_frame, .. } => {
                    let text = String::from_utf8_lossy(&raw_frame[raw_frame.len().saturating_sub(4096)..]);
                    let persisted = rpc::mark_unknown(&mut self.connection, &self.owner, &step, &text);
                    return Err(OrchestrationError::V37StoreFailure(format!("native RPC remote error: {text}; journal: {persisted:?}")));
                }
                _ => { failure(rpc::observe_event(&mut self.connection, &frame, &raw.key, &step))?; }
            }
        }
    }
}

#[cfg(all(test, windows))]
#[path = "v37_runtime_tests.rs"]
mod tests;
