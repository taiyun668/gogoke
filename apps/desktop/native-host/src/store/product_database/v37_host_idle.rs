//! A positive current-episode Codex idle observation for automatic Host input.
//! A missing in-memory turn, a stopped process, or an old generation is not idle.
use super::*;
use crate::store::atomic::Parser;
use crate::store::ledger;
use crate::store::session_transport::{self as h, codex_rpc};

fn value<'a>(fields: &'a BTreeMap<JsonString, Json>, name: &str) -> Option<&'a Json> {
    fields.get(&JsonString::from_str(name))
}
fn object(value: &Json) -> Option<&BTreeMap<JsonString, Json>> {
    if let Json::Object(fields) = value { Some(fields) } else { None }
}
fn string(value: Option<&Json>) -> Option<String> {
    if let Some(Json::String(text)) = value { text.to_well_formed_string() } else { None }
}
fn fields(bytes: &[u8]) -> Result<BTreeMap<JsonString, Json>> {
    let source = std::str::from_utf8(bytes).map_err(|error|
        OrchestrationError::V37StoreFailure(format!("original Host idle UTF-8: {error}")))?;
    let Json::Object(fields) = Parser::parse(source)? else {
        return Err(OrchestrationError::Invalid("original Host idle source object"));
    };
    Ok(fields)
}
fn bytes(encoded: &str) -> Result<Vec<u8>> {
    if encoded.len() % 2 != 0 { return Err(OrchestrationError::Invalid("Host idle command hex")); }
    encoded.as_bytes().chunks_exact(2).map(|pair| {
        let text = std::str::from_utf8(pair).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("Host idle command hex UTF-8: {error}")))?;
        u8::from_str_radix(text, 16).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("Host idle command hex: {error}")))
    }).collect()
}
fn positive_idle(status: Option<&Json>) -> bool {
    let Some(status) = status.and_then(object) else { return false; };
    status.len() == 1 && string(value(status, "type")).as_deref() == Some("idle")
}
fn positive_thread_idle(thread: &BTreeMap<JsonString, Json>) -> bool {
    if !positive_idle(value(thread, "status")) { return false; }
    let Some(Json::Array(turns)) = value(thread, "turns") else { return false; };
    // A resumed rollout containing an unfinished old turn is not permission
    // to append a new Host turn, even if its new physical process is idle.
    turns.iter().all(|turn| object(turn).is_some_and(|turn|
        matches!(string(value(turn, "status")).as_deref(),
            Some("completed" | "failed" | "interrupted"))))
}

impl<'root> ProductDatabase<'root> {
    /// Re-read original A at the authority safe point. Every observation is
    /// tied to the retained process ticket/nonce and this exact generation.
    pub(super) fn host_rule_recipient_idle(&mut self, key: &(String, String)) -> Result<bool> {
        let Some(run) = self.native_sessions.get(key) else { return Ok(false); };
        if run.evidence.driver_id() != "codex" || !run.allows_input() || run.turn_id.is_some() {
            return Ok(false);
        }
        let Some(thread) = run.thread_id.clone() else { return Ok(false); };
        let operation = run.operation_id.clone();
        let custody = run.custody.clone();
        let seat = run.evidence.seat_id().to_owned();
        if ledger::read_registered_session(&self.connection, &key.1)?
            .is_none_or(|registration| registration.domain_id != key.0 || registration.seat_id != seat
                || registration.purpose != ledger::SessionPurpose::Work) {
            return Ok(false);
        }
        if h::generation_change::active_for_session(&self.connection, &key.0, &key.1)?.is_some() {
            return Ok(false);
        }
        let live = Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_claim c
               JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=c.domain_id
                 AND e.session_id=c.session_id AND e.generation=c.generation
                 AND e.process_operation_id=c.process_operation_id AND e.phase='ACTIVE'
               JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id
                 AND p.domain_id=c.domain_id AND p.generation=c.generation AND p.state='ACTIVE'
              WHERE c.domain_id=?1 AND c.session_id=?2 AND c.generation=?3
                AND c.process_operation_id=?4 AND c.state='COMMITTED'
                AND p.ticket=?5 AND p.custodian_nonce=?6")?;
        for (index, text) in [key.0.as_str(), key.1.as_str(), custody.binding.generation.as_str(),
            operation.as_str(), custody.ticket.opaque(), custody.custodian_nonce.as_str()].iter().enumerate() {
            live.bind_text((index + 1) as i32, text)?;
        }
        if !live.step_row()? || live.step_row()? { return Ok(false); }
        drop(live);
        let questions = Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_qcard_native
              WHERE domain_id=?1 AND seat_id=?2 AND generation=?3
                AND state IN ('OPEN','ANSWER_UNKNOWN') LIMIT 1")?;
        questions.bind_text(1, &key.0)?; questions.bind_text(2, &seat)?;
        questions.bind_text(3, &custody.binding.generation)?;
        if questions.step_row()? { return Ok(false); }
        drop(questions);
        let query = Statement::prepare(self.connection.as_ptr(),
            "SELECT r.source_cursor,COALESCE(s.command_hex,'')
               FROM main.v37_ledger_raw_source r
               LEFT JOIN main.gogoke_v37_rpc_steps s ON s.domain_id=r.domain_id
                 AND s.session_id=r.session_id AND s.generation=r.generation
                 AND s.process_operation_id=r.operation_id AND s.ticket=r.process_ticket
                 AND s.custodian_nonce=r.custodian_nonce AND s.source_epoch=r.source_epoch
                 AND s.source_cursor=r.source_cursor AND s.phase='OBSERVED'
              WHERE r.domain_id=?1 AND r.session_id=?2 AND r.generation=?3
                AND r.operation_id=?4 AND r.process_ticket=?5 AND r.custodian_nonce=?6
                AND r.source_epoch=?6 ORDER BY CAST(r.source_cursor AS INTEGER)")?;
        for (index, text) in [key.0.as_str(), key.1.as_str(), custody.binding.generation.as_str(),
            operation.as_str(), custody.ticket.opaque(), custody.custodian_nonce.as_str()].iter().enumerate() {
            query.bind_text((index + 1) as i32, text)?;
        }
        let mut rows = Vec::new();
        while query.step_row()? { rows.push((query.column_text(0)?, query.column_text(1)?)); }
        drop(query);
        let mut idle = false;
        for (cursor, command_hex) in rows {
            let source = ledger::read_captured_raw_source(&self.connection, &operation,
                &custody.custodian_nonce, &cursor)?.ok_or(OrchestrationError::OperationConflict)?;
            let raw = fields(&source.raw_bytes)?;
            if let Some(result) = value(&raw, "result").and_then(object) {
                if let Some(observed_thread) = value(result, "thread").and_then(object) {
                    if command_hex.is_empty() { return Ok(false); }
                    let command = bytes(&command_hex)?;
                    let encoded = fields(&command)?;
                    let observed = match string(value(&encoded, "method")).as_deref() {
                        Some("thread/start") => codex_rpc::decode_stored_thread_start(&command, &source.raw_bytes),
                        Some("thread/resume") => codex_rpc::decode_stored_thread_resume(&command, &source.raw_bytes),
                        _ => return Ok(false),
                    }.map_err(|error| OrchestrationError::V37StoreFailure(
                        format!("original Host idle thread ACK: {error:?}")))?;
                    if observed != thread { return Ok(false); }
                    idle = positive_thread_idle(observed_thread);
                    continue;
                }
                // A later admitted input cannot inherit the opening idle ACK.
                if value(result, "turn").is_some() { idle = false; }
            }
            let Some(params) = value(&raw, "params").and_then(object) else { continue; };
            if string(value(params, "threadId")).as_deref() != Some(thread.as_str()) { continue; }
            let method = string(value(&raw, "method"));
            if method.as_deref() == Some("thread/status/changed") {
                idle = positive_idle(value(params, "status"));
            } else if method.as_deref() == Some("turn/started") {
                idle = false;
            }
            if source.state == ledger::RawSourceState::Pending {
                if value(&raw, "id").is_some() { return Ok(false); }
                if matches!(method.as_deref(), Some("item/started" | "item/completed")) {
                    let item = value(params, "item").and_then(object);
                    if !matches!(item.and_then(|item| string(value(item, "type"))).as_deref(),
                        Some("agentMessage" | "reasoning" | "userMessage" | "contextCompaction")) {
                        return Ok(false);
                    }
                }
            }
        }
        Ok(idle)
    }
}
