//! Actual native Codex sessions. The User channel selects logical identities;
//! E/F/H supply all paths, pins, permissions and custody on the same store.
use super::*;
use crate::process::{PreparedCustody, DurableStopConfirmation, StopBudgets, NativeStopProof, OriginBoundFrame};
use crate::store::ledger::{self, SessionPurpose, SessionRegistration};
use crate::store::seat::{self, NativeOrigin};
use crate::store::session_transport::{self as h, runtime, launch::LaunchEvidence,
    codex_rpc::{self, Command, RpcId, Reply}, rpc_journal as rpc};
use crate::store::atomic::Parser;
use std::time::{Duration, Instant};

pub(super) struct NativeSession {
    pub(super) evidence: LaunchEvidence,
    pub(super) custody: PreparedCustody,
    pub(super) operation_id: String,
    open_request_id: String,
    open_request_bytes: Vec<u8>,
    domain_id: String,
    session_id: String,
    model: String,
    effort: String,
    pub(super) thread_id: Option<String>,
    pub(super) turn_id: Option<String>,
    pub(super) raw_capture: super::v37_output::NativeRawCapture,
    stop_proof: Option<NativeStopProof>,
    next_rpc_id: u64,
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
                            Some(Json::Object(thread)) => match thread.get(&JsonString::from_str("id")) {
                                Some(Json::String(id)) => id.to_well_formed_string(),
                                _ => None,
                            },
                            _ => None,
                        },
                        _ => None,
                    }.ok_or(OrchestrationError::Invalid("native prior thread identity"))?;
                    if thread_id.is_empty() || thread_id.contains('\0') {
                        return Err(OrchestrationError::Invalid("native prior thread identity shape"));
                    }
                    result.insert(JsonString::from_str("threadId"), Json::String(JsonString::from_str(&thread_id)));
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
            next_rpc_id: 4 });
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
            failure(run.evidence.verify_in_transaction(&mut self.connection, self.root, &self.owner, Some(&operation_id)))?;
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
                self.confirm_native_stop(&(request.domain_id.clone(), request.target_id.clone()))?;
                let fact = Statement::prepare(self.connection.as_ptr(),
                    "SELECT c.stop_proof_hash FROM main.gogoke_v37_h_claim a JOIN main.gogoke_coordination_process_custody c ON c.operation_id=a.process_operation_id WHERE a.domain_id=?1 AND a.session_id=?2 AND a.state IN ('STOPPED','RELEASED') AND c.state='STOPPED' AND a.stop_fact_id=c.stop_proof_hash")?;
                fact.bind_text(1, &request.domain_id)?;
                fact.bind_text(2, &request.target_id)?;
                if !fact.step_row()? { return Err(OrchestrationError::OperationConflict); }
                let hash = fact.column_text(0)?;
                return Ok(encode_receipt(request, V37Status::Replayed,
                    request.expected_revision, revision, BTreeMap::from([
                        (JsonString::from_str("stopFact"), Json::String(JsonString::from_str(&hash)))])));
            }
            true
        } else { false };
        drop(prior);
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
            failure(h::reconcile_observed_codex_sends(&mut self.connection,
                &request.domain_id,&request.target_id,&generation))?;
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
                let existing = Statement::prepare(self.connection.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 AND operation='stop'")?;
                existing.bind_text(1, &request.domain_id)?;
                existing.bind_text(2, &request.target_id)?;
                if existing.step_row()? { return Err(OrchestrationError::OperationConflict); }
                drop(existing);
                let insert = Statement::prepare(self.connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES(?1,?2,?3,'stop',?4,'UNKNOWN',?5,?5)")?;
                for (index, value) in [request.domain_id.as_str(), request.request_id.as_str(),
                    hex(&request.raw_bytes).as_str(), request.target_id.as_str()].iter().enumerate() {
                    insert.bind_text((index + 1) as i32, value)?;
                }
                insert.bind_i64(5, claim.revision)?;
                insert.step_done()?;
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
            if stored.state != h::JournalState::Receipted {
                let input = h::StdinRequest { domain_id: &request.domain_id,
                    session_id: &request.target_id, ticket: &ticket, generation: &generation,
                    request_bytes: &request.raw_bytes };
                match failure(h::recover_codex_turn_request(&mut self.connection,&input))? {
                    Some(recovered) => stored = recovered.record,
                    None => return Ok(encode_receipt(request, V37Status::Unknown,
                        request.expected_revision, request.expected_revision, Default::default())),
                }
            }
            if let Some(bytes) = stored.receipt_bytes {
                let receipt = failure(h::decode_receipt(&bytes))?;
                return Ok(encode_receipt(request, V37Status::Replayed,
                    receipt.previous_revision, receipt.revision, receipt.into_result()));
            }
            return Ok(encode_receipt(request, V37Status::Unknown, request.expected_revision,
                request.expected_revision, Default::default()));
        }
        drop(prior);
        let key = (request.domain_id.clone(), request.target_id.clone());
        let run = self.native_sessions.get(&key).ok_or(OrchestrationError::Invalid("native send has no live custody"))?;
        let seat_id = run.evidence.seat_id().to_owned();
        let current = failure(runtime::observe_claim(&self.connection, &NativeOrigin::user(&self.owner),
            &request.domain_id, &seat_id, &request.target_id))?.ok_or(OrchestrationError::AccessDenied)?;
        if current.generation != generation || u64::try_from(current.revision).ok() != Some(request.expected_revision) {
            return Ok(encode_receipt(request, V37Status::Stale, request.expected_revision,
                request.expected_revision, Default::default()));
        }
        failure(run.evidence.verify_live(&mut self.connection, self.root, &self.owner,
            &run.operation_id, current.revision))?;
        let thread_id = run.thread_id.clone().ok_or(OrchestrationError::Invalid("native send thread absent"))?;
        let command = Command::TurnStart { thread_id: thread_id.clone(),
            cwd: run.evidence.cwd().to_string_lossy().into_owned(), model: run.model.clone(),
            effort: run.effort.clone(), text };
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
        let step_id = format!("send-{}", &crate::store::digest::sha256_hex(&request.raw_bytes)[..40]);
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

    /// Actual process-owned JSONL, with durable native step intent before
    /// writing and A's original provider bytes before interpreting responses.
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
                | Reply::Turn { .. } | Reply::Ack { .. } => {
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
                    let persisted = rpc::mark_unknown(&mut self.connection, &self.owner, &step, &text);
                    return Err(OrchestrationError::V37StoreFailure(format!("native RPC remote error: {text}; journal: {persisted:?}")));
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
