//! A positive current-episode Codex idle observation for automatic Host input.
//! A missing in-memory turn, a stopped process, or an old generation is not idle.
use super::*;
use crate::store::atomic::Parser;
use crate::store::ledger;
use crate::store::session_transport::{self as h, codex_rpc};
use std::collections::BTreeSet;

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
fn original_thread_turns(thread: &BTreeMap<JsonString, Json>)
    -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let status = value(thread, "status").and_then(object)?;
    if !matches!(string(value(status, "type")).as_deref(), Some("idle" | "active")) {
        return None;
    }
    let Json::Array(turns) = value(thread, "turns")? else { return None; };
    let mut seen = BTreeSet::new();
    let mut unfinished = BTreeSet::new();
    for turn in turns {
        let turn = object(turn)?;
        let id = string(value(turn, "id"))?;
        if id.is_empty() || !seen.insert(id.clone()) { return None; }
        match string(value(turn, "status"))?.as_str() {
            "inProgress" => { unfinished.insert(id); }
            "completed" | "failed" | "interrupted" => {}
            _ => return None,
        }
    }
    Some((seen, unfinished))
}

#[derive(Clone)]
struct IdleFrame {
    cursor: u64,
    command: Vec<u8>,
    raw: Vec<u8>,
    pending: bool,
}
struct EmptyInitialization {
    start: u64,
    end: u64,
    path: String,
}
enum Initialization {
    Unused,
    Incomplete,
    Complete(EmptyInitialization),
}
fn original_rpc<T>(reply: std::result::Result<T, codex_rpc::RpcError>) -> Result<T> {
    reply.map_err(|error| OrchestrationError::V37StoreFailure(
        format!("original Host idle initialization: {error:?}")))
}

/// This is the existing producer's one empty-thread materialization sequence,
/// not a general resume/history interpreter. Metadata never replaces turns.
fn empty_initialization(frames: &[IdleFrame], thread: &str, cwd: &str, model: &str,
    verify_cwd: impl Fn(&str) -> Result<()>)
    -> Result<Initialization> {
    let mut opening = None;
    for (index, frame) in frames.iter().enumerate() {
        let raw = fields(&frame.raw)?;
        if value(&raw, "result").and_then(object)
            .and_then(|result| value(result, "thread")).is_some() {
            opening = Some(index);
            break;
        }
    }
    let Some(start_index) = opening else { return Ok(Initialization::Unused); };
    let start = &frames[start_index];
    if start.command.is_empty() { return Ok(Initialization::Unused); }
    let start_command = fields(&start.command)?;
    if string(value(&start_command, "method")).as_deref() != Some("thread/start") {
        return Ok(Initialization::Unused);
    }
    let mut controls = Vec::new();
    for (index, frame) in frames.iter().enumerate().skip(start_index + 1) {
        if frame.command.is_empty() { continue; }
        let command = fields(&frame.command)?;
        if let Some(method) = string(value(&command, "method")) {
            if matches!(method.as_str(), "thread/start" | "thread/archive" | "thread/unarchive" | "thread/resume") {
                controls.push((index, method));
            }
        }
    }
    if controls.is_empty() { return Ok(Initialization::Unused); }
    if controls.len() != 3 || controls[0].1 != "thread/archive"
        || controls[1].1 != "thread/unarchive" || controls[2].1 != "thread/resume" {
        return Ok(Initialization::Incomplete);
    }
    for (index, frame) in frames.iter().enumerate() {
        if frame.cursor != index as u64 + 1 { return Ok(Initialization::Incomplete); }
    }
    if original_rpc(codex_rpc::decode_stored_thread_start(&start.command, &start.raw))? != thread {
        return Ok(Initialization::Incomplete);
    }
    original_rpc(codex_rpc::verify_new_empty_paginated_thread(&start.raw, thread))?;
    let path = original_rpc(codex_rpc::persistent_paginated_thread_path(&start.raw, thread))?;
    let Some(params) = value(&start_command, "params").and_then(object) else {
        return Ok(Initialization::Incomplete);
    };
    if string(value(params, "cwd")).as_deref() != Some(cwd)
        || string(value(params, "model")).as_deref() != Some(model) {
        return Ok(Initialization::Incomplete);
    }
    let archive = &frames[controls[0].0];
    let unarchive = &frames[controls[1].0];
    let resume = &frames[controls[2].0];
    if original_rpc(codex_rpc::decode_stored_thread_archive(&archive.command, &archive.raw))? != thread
        || original_rpc(codex_rpc::decode_stored_thread_unarchive(&unarchive.command, &unarchive.raw))? != thread
        || original_rpc(codex_rpc::decode_stored_thread_resume(&resume.command, &resume.raw))? != thread {
        return Ok(Initialization::Incomplete);
    }
    let resumed_command = fields(&resume.command)?;
    let Some(params) = value(&resumed_command, "params").and_then(object) else {
        return Ok(Initialization::Incomplete);
    };
    if !matches!(value(params, "excludeTurns"), Some(Json::Bool(true)))
        || string(value(params, "cwd")).as_deref() != Some(cwd)
        || string(value(params, "model")).as_deref() != Some(model) {
        return Ok(Initialization::Incomplete);
    }
    for frame in [start, unarchive, resume] {
        let raw = fields(&frame.raw)?;
        let Some(observed) = value(&raw, "result").and_then(object)
            .and_then(|result| value(result, "thread")).and_then(object) else {
            return Ok(Initialization::Incomplete);
        };
        let Some(observed_cwd) = string(value(observed, "cwd")) else {
            return Ok(Initialization::Incomplete);
        };
        // H already verifies these original replies against the bound physical
        // directory. Recheck that same authority, not its DOS/verbatim spelling.
        verify_cwd(&observed_cwd)?;
        if original_rpc(codex_rpc::persistent_paginated_thread_path(&frame.raw, thread))? != path {
            return Ok(Initialization::Incomplete);
        }
        // Nonempty metadata conflicts with the original no-input sequence;
        // an omitted/empty list is never used as its proof of empty history.
        if matches!(value(observed, "turns"), Some(Json::Array(turns)) if !turns.is_empty()) {
            return Ok(Initialization::Incomplete);
        }
        if [start.cursor, resume.cursor].contains(&frame.cursor)
            && !positive_idle(value(observed, "status")) {
            return Ok(Initialization::Incomplete);
        }
        if frame.cursor == unarchive.cursor && !matches!(value(observed, "status").and_then(object)
            .and_then(|status| string(value(status, "type"))).as_deref(), Some("notLoaded" | "idle")) {
            return Ok(Initialization::Incomplete);
        }
    }
    for frame in frames.iter().filter(|frame| frame.cursor > start.cursor && frame.cursor <= resume.cursor) {
        let raw = fields(&frame.raw)?;
        let method = string(value(&raw, "method"));
        if method.as_deref().is_some_and(|method| method.starts_with("turn/") || method.starts_with("item/"))
            || (method.is_some() && value(&raw, "id").is_some()) {
            return Ok(Initialization::Incomplete);
        }
        if value(&raw, "id").is_some()
            && ![archive.cursor, unarchive.cursor, resume.cursor].contains(&frame.cursor) {
            return Ok(Initialization::Incomplete);
        }
        if value(&raw,"result").and_then(object).and_then(|result| value(result,"turn")).is_some() {
            return Ok(Initialization::Incomplete);
        }
        if let Some(params) = value(&raw, "params").and_then(object) {
            if ["turn", "turnId", "item"].iter().any(|name| value(params,name).is_some()) {
                return Ok(Initialization::Incomplete);
            }
            if string(value(params, "threadId")).is_some_and(|id| id != thread) {
                return Ok(Initialization::Incomplete);
            }
            if method.as_deref() == Some("thread/status/changed") {
                let status = value(params, "status").and_then(object);
                if !matches!(status.and_then(|status| string(value(status, "type"))).as_deref(),
                    Some("idle" | "notLoaded")) {
                    return Ok(Initialization::Incomplete);
                }
            }
        }
    }
    Ok(Initialization::Complete(EmptyInitialization {start:start.cursor,end:resume.cursor,path}))
}

impl<'root> ProductDatabase<'root> {
    /// Re-read original A at the authority safe point. Every observation is
    /// tied to the retained process ticket/nonce and this exact generation.
    pub(super) fn host_rule_recipient_idle(&mut self, key: &(String, String)) -> Result<bool> {
        self.recipient_idle_with_purpose(key,ledger::SessionPurpose::Work)
    }
    pub(super) fn side_recipient_idle(&mut self,key:&(String,String),
        purpose:ledger::SessionPurpose)->Result<bool> {
        self.recipient_idle_with_purpose(key,purpose)
    }
    fn recipient_idle_with_purpose(&mut self, key:&(String,String),
        purpose:ledger::SessionPurpose)->Result<bool> {
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
                || registration.purpose != purpose) {
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
        let mut frames = Vec::new();
        for (cursor, command_hex) in rows {
            let ordinal = cursor.parse::<u64>().ok().filter(|ordinal| *ordinal > 0
                && ordinal.to_string() == cursor);
            let Some(cursor_number) = ordinal else { return Ok(false); };
            let source = ledger::read_captured_raw_source(&self.connection, &operation,
                &custody.custodian_nonce, &cursor)?.ok_or(OrchestrationError::OperationConflict)?;
            frames.push(IdleFrame {cursor:cursor_number, command:bytes(&command_hex)?,
                pending:source.state == ledger::RawSourceState::Pending, raw:source.raw_bytes});
        }
        let initialization = empty_initialization(&frames, &thread,
            &run.evidence.protocol_cwd().to_string_lossy(), &run.model, |observed|
                run.evidence.verify_observed_cwd(observed).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("original Host idle cwd: {error}"))))?;
        let initialization = match &initialization {
            Initialization::Incomplete => return Ok(false),
            Initialization::Unused => None,
            Initialization::Complete(chain) => {
                if !self.empty_initialization_inputs_after(key, &custody, &operation, chain, &frames)? {
                    return Ok(false);
                }
                run.evidence.verify_codex_rollout_path(&chain.path, true).map_err(|error|
                    OrchestrationError::V37StoreFailure(format!("original Host idle namespace: {error:?}")))?;
                Some(chain)
            },
        };
        fold_idle(&frames, &thread, None)
    }

    /// All phases matter: an unacknowledged write has no A cursor. Never infer
    /// that it happened after initialization from row order or a step name.
    fn empty_initialization_inputs_after(&self, key: &(String, String),
        custody: &crate::process::PreparedCustody, operation: &str, chain: &EmptyInitialization,
        frames: &[IdleFrame]) -> Result<bool> {
        let rpc = Statement::prepare(self.connection.as_ptr(),
            "SELECT step_id,command_hex,phase,COALESCE(source_cursor,''),COALESCE(source_epoch,''),
                    process_operation_id,ticket,custodian_nonce
               FROM main.gogoke_v37_rpc_steps
              WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
        for (index, text) in [key.0.as_str(),key.1.as_str(),custody.binding.generation.as_str()].iter().enumerate() {
            rpc.bind_text((index + 1) as i32, text)?;
        }
        let mut later_inputs = BTreeSet::new();
        while rpc.step_row()? {
            if rpc.column_text(5)? != operation || rpc.column_text(6)? != custody.ticket.opaque()
                || rpc.column_text(7)? != custody.custodian_nonce { return Ok(false); }
            let command_bytes = bytes(&rpc.column_text(1)?)?;
            let command = fields(&command_bytes)?;
            let method = string(value(&command, "method"));
            let input = method.as_deref().is_some_and(|method| method.starts_with("turn/")
                || matches!(method,"thread/inject_items" | "thread/compact/start"));
            let control = matches!(method.as_deref(),
                Some("thread/start" | "thread/archive" | "thread/unarchive" | "thread/resume"));
            if input || control {
                let cursor = rpc.column_text(3)?;
                let Some(cursor) = cursor.parse::<u64>().ok().filter(|ordinal|
                    *ordinal > 0 && ordinal.to_string() == cursor) else { return Ok(false); };
                if rpc.column_text(2)? != "OBSERVED" || rpc.column_text(4)? != custody.custodian_nonce
                    || !frames.iter().any(|frame| frame.cursor == cursor && frame.command == command_bytes) {
                    return Ok(false);
                }
                if input {
                    if cursor <= chain.end { return Ok(false); }
                    later_inputs.insert(rpc.column_text(0)?);
                } else if cursor < chain.start || cursor > chain.end { return Ok(false); }
            } else if !matches!(method.as_deref(), Some("initialize" | "initialized" | "config/read"
                | "experimentalFeature/list" | "model/list" | "thread/turns/list"))
                && !(method.is_none() && value(&command,"id").is_some()
                    && (value(&command,"result").is_some() || value(&command,"error").is_some())) {
                return Ok(false);
            }
        }
        drop(rpc);
        let stdin = Statement::prepare(self.connection.as_ptr(),
            "SELECT operation,request_hex,phase,process_operation_id,ticket,custodian_nonce
               FROM main.gogoke_v37_h_stdin_journal
              WHERE domain_id=?1 AND session_id=?2 AND generation=?3")?;
        for (index, text) in [key.0.as_str(),key.1.as_str(),custody.binding.generation.as_str()].iter().enumerate() {
            stdin.bind_text((index + 1) as i32, text)?;
        }
        while stdin.step_row()? {
            if stdin.column_text(2)? != "RECEIPTED" || stdin.column_text(3)? != operation
                || stdin.column_text(4)? != custody.ticket.opaque()
                || stdin.column_text(5)? != custody.custodian_nonce { return Ok(false); }
            let prefix = match stdin.column_text(0)?.as_str() {
                "send" => "send", "append-without-turn" => "append", _ => return Ok(false),
            };
            let request = bytes(&stdin.column_text(1)?)?;
            let step = format!("{prefix}-{}", &crate::store::digest::sha256_hex(&request)[..40]);
            if !later_inputs.contains(&step) { return Ok(false); }
        }
        Ok(true)
    }
}

fn fold_idle(frames: &[IdleFrame], thread: &str, initialization: Option<&EmptyInitialization>)
    -> Result<bool> {
        let mut idle = false;
        let mut opening_seen = false;
        let mut known = BTreeSet::new();
        let mut unfinished = BTreeSet::new();
        let mut terminal = BTreeSet::new();
        for source in frames {
            let raw = fields(&source.raw)?;
            if source.pending && value(&raw,"id").is_some() {
                return Ok(false);
            }
            if let Some(chain) = initialization {
                if source.cursor > chain.start && source.cursor <= chain.end {
                    if source.cursor == chain.end { idle = true; }
                    continue;
                }
            }
            if let Some(result) = value(&raw, "result").and_then(object) {
                if let Some(observed_thread) = value(result, "thread").and_then(object) {
                    if opening_seen { return Ok(false); }
                    if source.command.is_empty() { return Ok(false); }
                    let command = &source.command;
                    let encoded = fields(command)?;
                    let observed = match string(value(&encoded, "method")).as_deref() {
                        Some("thread/start") => codex_rpc::decode_stored_thread_start(command, &source.raw),
                        Some("thread/resume") => codex_rpc::decode_stored_thread_resume(command, &source.raw),
                        _ => return Ok(false),
                    }.map_err(|error| OrchestrationError::V37StoreFailure(
                        format!("original Host idle thread ACK: {error:?}")))?;
                    if observed != thread { return Ok(false); }
                    let Some((turns, blockers)) = original_thread_turns(observed_thread) else { return Ok(false); };
                    known.extend(turns);
                    unfinished.extend(blockers.into_iter().filter(|id| !terminal.contains(id)));
                    idle = positive_idle(value(observed_thread, "status"));
                    opening_seen = true;
                    continue;
                }
                // A later admitted input cannot inherit the opening idle ACK.
                if let Some(turn) = value(result, "turn") {
                    let Some(turn) = object(turn) else { return Ok(false); };
                    let Some(id) = string(value(turn, "id")) else { return Ok(false); };
                    if id.is_empty() || source.command.is_empty() { return Ok(false); }
                    let command_bytes = &source.command;
                    let (rpc_id, command) = codex_rpc::decode_stored_turn_start(command_bytes)
                        .map_err(|error| OrchestrationError::V37StoreFailure(
                            format!("original Host idle turn command: {error:?}")))?;
                    if !matches!(&command, codex_rpc::Command::TurnStart {thread_id, ..} if thread_id.as_str() == thread) {
                        return Ok(false);
                    }
                    let reply = codex_rpc::decode(&source.raw, Some((&rpc_id, &command)))
                        .map_err(|error| OrchestrationError::V37StoreFailure(
                            format!("original Host idle turn ACK: {error:?}")))?;
                    let codex_rpc::Reply::Turn {turn_id, status, ..} = reply else { return Ok(false); };
                    if turn_id != id { return Ok(false); }
                    known.insert(id.clone());
                    if status != codex_rpc::TurnStatus::InProgress {
                        // The original matched ACK can itself be terminal.
                        // It does not manufacture a positive thread idle.
                        terminal.insert(id.clone());
                        unfinished.remove(&id);
                    } else if !terminal.contains(&id) {
                        // A late in-progress ACK cannot reopen an earlier
                        // original terminal for this exact turn.
                        unfinished.insert(id);
                        idle = false;
                    }
                }
            }
            let Some(params) = value(&raw, "params").and_then(object) else { continue; };
            if string(value(params, "threadId")).as_deref() != Some(thread) { continue; }
            let method = string(value(&raw, "method"));
            if method.as_deref() == Some("thread/status/changed") {
                if !opening_seen { continue; }
                let Some(status) = value(params, "status").and_then(object) else { return Ok(false); };
                if !matches!(string(value(status, "type")).as_deref(), Some("idle" | "active")) {
                    return Ok(false);
                }
                idle = positive_idle(value(params, "status"));
            } else if method.as_deref() == Some("turn/started") {
                let Some(turn) = value(params, "turn").and_then(object) else { return Ok(false); };
                let Some(id) = string(value(turn, "id")) else { return Ok(false); };
                if id.is_empty() || string(value(turn, "status")).as_deref() != Some("inProgress") {
                    return Ok(false);
                }
                known.insert(id.clone());
                if !terminal.contains(&id) {
                    unfinished.insert(id);
                    idle = false;
                }
            } else if method.as_deref() == Some("turn/completed") {
                let Some(turn) = value(params, "turn").and_then(object) else { return Ok(false); };
                let Some(id) = string(value(turn, "id")) else { return Ok(false); };
                if id.is_empty() || !matches!(string(value(turn, "status")).as_deref(),
                    Some("completed" | "failed" | "interrupted")) {
                    return Ok(false);
                }
                terminal.insert(id.clone());
                unfinished.remove(&id);
            } else if !opening_seen {
                // The opening ACK is the first complete thread snapshot.
                continue;
            }
            if source.pending {
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
        Ok(opening_seen && idle && unfinished.is_empty() && terminal.is_subset(&known))
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_rpc::{Command, RpcId};

    // Protocol boundary fixtures only. Nothing is written to a provider; the
    // real pinned-CLI/namespace positive lives in v37_runtime_tests.
    fn reply(id:u64,status:&str)->Vec<u8> {
        format!("{{\"id\":{id},\"result\":{{\"thread\":{{\"id\":\"threadA\",\"cwd\":\"fixture-cwd\",\"path\":\"fixture-history\",\"status\":{{\"type\":\"{status}\"}},\"historyMode\":\"paginated\",\"ephemeral\":false,\"turns\":[]}}}}}}\n").into_bytes()
    }
    fn control(cursor:u64,id:u64,command:Command,raw:Vec<u8>)->IdleFrame {
        IdleFrame {cursor,command:command.encode(Some(&RpcId::client(id).unwrap())).unwrap(),raw,pending:false}
    }
    fn notification(cursor:u64,raw:&str)->IdleFrame {
        IdleFrame {cursor,command:Vec::new(),raw:raw.as_bytes().to_vec(),pending:true}
    }
    fn empty_chain()->Vec<IdleFrame> {
        vec![
            control(1,1,Command::ThreadStart {cwd:"fixture-cwd".into(),model:"modelA".into()},reply(1,"idle")),
            notification(2,r#"{"method":"thread/status/changed","params":{"threadId":"threadA","status":{"type":"notLoaded"}}}"#),
            control(3,2,Command::ThreadArchive {thread_id:"threadA".into()},b"{\"id\":2,\"result\":{}}\n".to_vec()),
            control(4,3,Command::ThreadUnarchive {thread_id:"threadA".into()},reply(3,"notLoaded")),
            control(5,4,Command::ThreadResumeMetadata {thread_id:"threadA".into(),cwd:"fixture-cwd".into(),model:"modelA".into()},reply(4,"idle")),
        ]
    }
    fn fixture_cwd(observed:&str)->Result<()> {
        if observed=="fixture-cwd" {Ok(())}
        else {Err(OrchestrationError::Invalid("protocol fixture cwd"))}
    }
    fn observed(frames:&[IdleFrame])->Initialization {
        empty_initialization(frames,"threadA","fixture-cwd","modelA",fixture_cwd).unwrap()
    }
    fn rejects(frames:&[IdleFrame]) {
        assert!(!matches!(empty_initialization(frames,"threadA","fixture-cwd","modelA",fixture_cwd),
            Ok(Initialization::Complete(_))),"damaged initialization must not qualify");
    }
    #[test]
    fn complete_original_empty_initialization_settles_only_its_not_loaded() {
        let frames=empty_chain();
        assert!(!fold_idle(&frames,"threadA",None).unwrap(),"original consumer reproduces the historical blocker");
        let Initialization::Complete(chain)=observed(&frames) else {panic!("complete chain");};
        assert!(fold_idle(&frames,"threadA",Some(&chain)).unwrap());
        assert_eq!(chain.path,"fixture-history");
        for index in 2..5 {
            let mut missing=frames.clone();missing.remove(index);rejects(&missing);
        }
        let mut reordered=frames.clone();reordered.swap(2,3);rejects(&reordered);
        let mut duplicate=frames.clone();duplicate.push(duplicate[2].clone());rejects(&duplicate);
        let mut missing_cursor=frames.clone();missing_cursor[1].cursor=3;rejects(&missing_cursor);
    }
    #[test]
    fn initialization_rejects_changed_binding_and_intermediate_input() {
        let frames=empty_chain();
        for (index,old,new) in [(3,"fixture-history","other-history"),(4,"fixture-cwd","other-cwd"),
            (3,"threadA","other-thread"),(4,"\"turns\":[]","\"turns\":[{\"id\":\"hidden\",\"status\":\"inProgress\"}]")] {
            let mut damaged=frames.clone();
            damaged[index].raw=String::from_utf8(damaged[index].raw.clone()).unwrap().replace(old,new).into_bytes();
            rejects(&damaged);
        }
        for raw in [
            r#"{"method":"turn/started","params":{"threadId":"threadA","turn":{"id":"t","status":"inProgress"}}}"#,
            r#"{"id":9,"method":"item/tool/requestUserInput","params":{}}"#,
            r#"{"method":"thread/status/changed","params":{"threadId":"threadA","status":{"type":"active"}}}"#,
        ] {
            let mut damaged=frames.clone();damaged[1]=notification(2,raw);rejects(&damaged);
        }
    }
    fn turn_ack(cursor:u64)->IdleFrame {
        control(cursor,8,Command::TurnStart {thread_id:"threadA".into(),cwd:"fixture-cwd".into(),
            model:"modelA".into(),effort:"high".into(),text:"synthetic boundary; never written".into(),network_access:Some(false)},
            b"{\"id\":8,\"result\":{\"turn\":{\"id\":\"turnA\",\"status\":\"inProgress\"}}}\n".to_vec())
    }
    fn idle(cursor:u64)->IdleFrame {
        notification(cursor,r#"{"method":"thread/status/changed","params":{"threadId":"threadA","status":{"type":"idle"}}}"#)
    }
    fn completed(cursor:u64)->IdleFrame {
        notification(cursor,r#"{"method":"turn/completed","params":{"threadId":"threadA","turn":{"id":"turnA","status":"completed"}}}"#)
    }
    #[test]
    fn initialized_busy_turn_requires_terminal_and_current_idle() {
        let mut frames=empty_chain();
        let Initialization::Complete(chain)=observed(&frames) else {panic!("complete chain");};
        frames.push(turn_ack(6));frames.push(idle(7));
        assert!(!fold_idle(&frames,"threadA",Some(&chain)).unwrap(),"idle cannot clear an unfinished turn");
        frames.push(completed(8));
        assert!(fold_idle(&frames,"threadA",Some(&chain)).unwrap(),"original idle then terminal is sufficient");
        frames.push(notification(9,r#"{"method":"thread/status/changed","params":{"threadId":"threadA","status":{"type":"active"}}}"#));
        assert!(!fold_idle(&frames,"threadA",Some(&chain)).unwrap());
        let mut late=empty_chain();late.push(completed(6));late.push(turn_ack(7));late.push(idle(8));
        assert!(fold_idle(&late,"threadA",Some(&chain)).unwrap(),"late ACK cannot reopen the exact terminal");
    }
    #[test]
    fn metadata_and_unmatched_pending_request_never_clear_busy() {
        let mut frames=empty_chain();
        let Initialization::Complete(chain)=observed(&frames) else {panic!("complete chain");};
        frames.push(turn_ack(6));
        frames.push(control(7,9,Command::ThreadResumeMetadata {thread_id:"threadA".into(),cwd:"fixture-cwd".into(),model:"modelA".into()},reply(9,"idle")));
        assert!(!fold_idle(&frames,"threadA",Some(&chain)).unwrap());
        rejects(&frames);
        let mut request=empty_chain();
        request.push(notification(6,r#"{"id":99,"method":"unknown/request","params":{}}"#));
        assert!(!fold_idle(&request,"threadA",Some(&chain)).unwrap());
    }
}
