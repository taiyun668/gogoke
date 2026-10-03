//! Durable H stdin request/receipt journal.
//!
//! The request row is written on the same verified SQLite connection before a
//! caller writes a byte to the child.  A write with an uncertain outcome is
//! fenced as UNKNOWN and cannot be prepared again under a new request ID.  A
//! terminal row is accepted only when the receipt came from an
//! `OriginBoundFrame` read from the exact native process custody.

use super::{codex_rpc, decode_receipt, decode_request, encode_receipt, V37Receipt, V37Status};
use super::provider_evidence::{acp, commands, stream_json};
use crate::process::{OriginBoundFrame, PreparedCustody};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use crate::store::authority::OwnerIssuer;
use crate::store::ledger::{self, RawSourceKey, RawSourceState};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

const MAX_FRAME_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(crate) enum JournalError {
    Invalid(&'static str),
    Denied,
    Conflict,
    Unknown,
    Store(AtomicError),
    Sqlite(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown { primary: Box<JournalError>, rollback: SameOpenError },
    Codec(codex_rpc::RpcError),
    Rpc(super::rpc_journal::RpcJournalError),
    RemoteError(Vec<u8>),
}

impl From<AtomicError> for JournalError {
    fn from(error: AtomicError) -> Self {
        Self::Store(error)
    }
}

impl From<SameOpenError> for JournalError {
    fn from(error: SameOpenError) -> Self {
        Self::Sqlite(error)
    }
}

impl From<codex_rpc::RpcError> for JournalError {
    fn from(error: codex_rpc::RpcError) -> Self { Self::Codec(error) }
}
impl From<super::rpc_journal::RpcJournalError> for JournalError {
    fn from(error: super::rpc_journal::RpcJournalError) -> Self { Self::Rpc(error) }
}

pub(crate) struct StdinRequest<'a> {
    /// Trusted H/session identity supplied by the native Controller path.
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    /// The process ticket is checked against the same verified custody row.
    pub(crate) ticket: &'a str,
    pub(crate) generation: &'a str,
    /// Exact original v37 request bytes. The legacy adapter writes these to
    /// stdin; native Codex commands have their own correlated RPC journal.
    pub(crate) request_bytes: &'a [u8],
}

/// The actual process/open binding stays with H. The pending callback needs
/// only this input's original bytes, deterministic identity, and A source key.
pub(crate) struct AcpSendInput<'a> {
    pub(crate) user: StdinRequest<'a>,
    pub(crate) custody: &'a PreparedCustody,
    pub(crate) open_request_id: &'a str,
    pub(crate) open_request_bytes: &'a [u8],
}

pub(crate) struct ClaudeSendInput<'a> {
    pub(crate) user: StdinRequest<'a>,
    pub(crate) custody: &'a PreparedCustody,
    pub(crate) open_request_id: &'a str,
    pub(crate) open_request_bytes: &'a [u8],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeSendIdentity {
    pub(crate) step_id: String,
    pub(crate) uuid: String,
}

pub(crate) struct ClaudeSendPrepared {
    pub(crate) user: JournalDecision,
    pub(crate) identity: ClaudeSendIdentity,
    pub(crate) bytes: Vec<u8>,
    pub(crate) write_permitted: bool,
}

pub(crate) struct ClaudeSendCompleted {
    pub(crate) user: JournalDecision,
    pub(crate) terminal: stream_json::ClaudeData,
    pub(crate) vendor_session_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AcpSendIdentity {
    pub(crate) step_id: String,
    pub(crate) rpc_id: acp::RpcId,
}

#[derive(Debug)]
pub(crate) struct AcpSendPrepared {
    pub(crate) user: JournalDecision,
    pub(crate) identity: AcpSendIdentity,
    pub(crate) bytes: Vec<u8>,
    pub(crate) write_permitted: bool,
}

pub(crate) struct AcpSendCompleted {
    pub(crate) user: JournalDecision,
    pub(crate) observation: acp::Observation,
}

fn acp_terminal_status(observation: &acp::Observation)
    -> Result<(V37Status, &'static str), JournalError> {
    match observation {
        acp::Observation::Prompt { stop_reason: acp::StopReason::EndTurn, .. } =>
            Ok((V37Status::Applied, "end_turn")),
        acp::Observation::Prompt { stop_reason: acp::StopReason::MaxTokens, .. } =>
            Ok((V37Status::Failed, "max_tokens")),
        acp::Observation::Prompt { stop_reason: acp::StopReason::MaxTurnRequests, .. } =>
            Ok((V37Status::Failed, "max_turn_requests")),
        acp::Observation::Prompt { stop_reason: acp::StopReason::Refusal, .. } =>
            Ok((V37Status::Failed, "refusal")),
        acp::Observation::Prompt { stop_reason: acp::StopReason::Cancelled, .. } =>
            Ok((V37Status::Failed, "cancelled")),
        acp::Observation::Prompt { stop_reason: acp::StopReason::Error, .. } =>
            Ok((V37Status::Failed, "error")),
        acp::Observation::RemoteError { .. } => Ok((V37Status::Failed, "remote_error")),
        _ => Err(JournalError::Invalid("not an ACP prompt response")),
    }
}

pub(crate) struct StdinJournalKey<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) ticket: &'a str,
    pub(crate) generation: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum JournalState {
    Prepared,
    Unknown,
    Receipted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StdinJournalRecord {
    pub(crate) domain_id: String,
    pub(crate) request_id: String,
    pub(crate) operation: String,
    pub(crate) ticket: String,
    pub(crate) process_operation_id: String,
    pub(crate) custodian_nonce: String,
    pub(crate) session_id: String,
    pub(crate) generation: String,
    pub(crate) request_bytes: Vec<u8>,
    pub(crate) state: JournalState,
    pub(crate) receipt_bytes: Option<Vec<u8>>,
    pub(crate) receipt_status: Option<V37Status>,
    pub(crate) expected_revision: u64,
    pub(crate) receipt_previous_revision: Option<u64>,
    pub(crate) receipt_revision: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PrepareDisposition {
    Prepared,
    Replayed,
    Unknown,
    Completed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct JournalDecision {
    pub(crate) disposition: PrepareDisposition,
    pub(crate) record: StdinJournalRecord,
}

#[derive(Clone, Copy)]
enum BindingUse {
    Prepare,
    MarkUnknown,
    Complete,
    Read,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct HBinding {
    process_operation_id: String,
    ticket: String,
    custodian_nonce: String,
    domain_id: String,
    generation: String,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn valid_generation(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok()
}

fn require_id(value: &str, name: &'static str) -> Result<(), JournalError> {
    if valid_id(value) {
        Ok(())
    } else {
        Err(JournalError::Invalid(name))
    }
}

fn require_generation(value: &str) -> Result<(), JournalError> {
    if valid_generation(value) {
        Ok(())
    } else {
        Err(JournalError::Invalid("generation"))
    }
}

fn frame_bytes(value: &[u8], name: &'static str) -> Result<(), JournalError> {
    if value.is_empty()
        || value.len() > MAX_FRAME_BYTES
        || value.last() != Some(&b'\n')
        || value[..value.len() - 1].contains(&b'\n')
    {
        return Err(JournalError::Invalid(name));
    }
    Ok(())
}

fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(value.len() * 2);
    for byte in value {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}

fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn unhex(value: &str) -> Result<Vec<u8>, JournalError> {
    if value.is_empty() || value.len() % 2 != 0 {
        return Err(JournalError::Unknown);
    }
    let mut output = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let high = hex_nibble(pair[0]).ok_or(JournalError::Unknown)?;
        let low = hex_nibble(pair[1]).ok_or(JournalError::Unknown)?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn generation_from_payload(request: &super::V37Request) -> Result<String, JournalError> {
    let Some(Json::String(value)) = request.payload.get(&JsonString::from_str("generation")) else {
        return Err(JournalError::Invalid("payload.generation"));
    };
    let generation = value
        .to_well_formed_string()
        .ok_or(JournalError::Invalid("payload.generation"))?;
    require_generation(&generation)?;
    Ok(generation)
}

fn parse_request(input: &StdinRequest<'_>) -> Result<super::V37Request, JournalError> {
    parse_operation(input, true)
}

fn parse_operation(input: &StdinRequest<'_>, child_frame: bool) -> Result<super::V37Request, JournalError> {
    for (value, name) in [
        (input.domain_id, "domain_id"),
        (input.session_id, "session_id"),
        (input.ticket, "ticket"),
    ] {
        require_id(value, name)?;
    }
    require_generation(input.generation)?;
    if child_frame {
        frame_bytes(input.request_bytes, "request frame")?;
    } else if input.request_bytes.is_empty() || input.request_bytes.len() > MAX_FRAME_BYTES {
        return Err(JournalError::Invalid("original operation size"));
    }
    let request =
        decode_request(input.request_bytes).map_err(|_| JournalError::Invalid("request frame"))?;
    if request.family != "K-SESSION"
        || request.domain_id != input.domain_id
        || request.target_id != input.session_id
    {
        return Err(JournalError::Denied);
    }
    if generation_from_payload(&request)? != input.generation {
        return Err(JournalError::Conflict);
    }
    Ok(request)
}

fn status_from_wire(value: &str) -> Option<V37Status> {
    match value {
        "APPLIED" => Some(V37Status::Applied),
        "REPLAYED" => Some(V37Status::Replayed),
        "DENIED" => Some(V37Status::Denied),
        "STALE" => Some(V37Status::Stale),
        "CONFLICT" => Some(V37Status::Conflict),
        "UNSUPPORTED" => Some(V37Status::Unsupported),
        "UNKNOWN" => Some(V37Status::Unknown),
        "FAILED" => Some(V37Status::Failed),
        _ => None,
    }
}

fn parse_revision(value: &str) -> Result<u64, JournalError> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(JournalError::Unknown);
    }
    value.parse::<u64>().map_err(|_| JournalError::Unknown)
}

fn in_transaction<T>(
    connection: &mut VerifiedDatabaseConnection<'_>,
    action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    connection.execute("BEGIN IMMEDIATE")?;
    match action(connection) {
        Ok(value) => {
            connection
                .execute("COMMIT")
                .map_err(JournalError::CommitUnknown)?;
            Ok(value)
        }
        Err(primary) => match connection.execute("ROLLBACK") {
            Ok(()) => Err(primary),
            Err(rollback) => Err(JournalError::RollbackUnknown { primary: Box::new(primary), rollback }),
        },
    }
}

fn changes(connection: &VerifiedDatabaseConnection<'_>) -> Result<i64, JournalError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT changes()")?;
    if !statement.step_row()? {
        return Err(JournalError::Unknown);
    }
    statement
        .column_text(0)?
        .parse::<i64>()
        .map_err(|_| JournalError::Unknown)
}

fn h_binding(
    connection: &VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    ticket: &str,
    generation: &str,
    use_case: BindingUse,
) -> Result<HBinding, JournalError> {
    if matches!(use_case,BindingUse::Read) {
        let history=Statement::prepare(connection.as_ptr(),
            "SELECT e.process_operation_id,c.ticket,c.custodian_nonce
               FROM main.gogoke_v37_h_process_episode e
               JOIN main.gogoke_v37_h_generation g
                 ON g.domain_id=e.domain_id AND g.session_id=e.session_id
                 AND g.generation=e.generation
                 AND g.process_operation_id=e.process_operation_id
               JOIN main.gogoke_coordination_process_custody c
                 ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
                 AND c.generation=e.generation
              WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3
                AND c.ticket=?4
                AND ((e.phase='STOPPED' AND c.state='STOPPED'
                      AND e.stop_fact_id=c.stop_proof_hash AND e.stop_fact_id IS NOT NULL)
                   OR (e.phase='ACTIVE' AND c.state IN ('ACTIVE','UNKNOWN'))
                   OR (e.phase='UNKNOWN' AND c.state IN ('UNKNOWN','STOPPED')))")?;
        for (index,value) in [domain_id,session_id,generation,ticket].iter().enumerate() {
            history.bind_text((index+1) as i32,value)?;
        }
        if !history.step_row()? {return Err(JournalError::Denied);}
        let binding=HBinding {process_operation_id:history.column_text(0)?,
            ticket:history.column_text(1)?,custodian_nonce:history.column_text(2)?,
            domain_id:domain_id.to_owned(),generation:generation.to_owned()};
        if history.step_row()? {return Err(JournalError::Conflict);}
        return Ok(binding);
    }
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT c.operation_id,c.ticket,c.custodian_nonce,a.domain_id,a.generation,
                a.state,c.state,COALESCE(a.stop_fact_id,''),COALESCE(c.stop_proof_hash,'')
           FROM main.gogoke_v37_h_claim AS a
           JOIN main.gogoke_coordination_process_custody AS c
             ON c.operation_id=a.process_operation_id
            AND c.domain_id=a.domain_id
            AND c.generation=a.generation
           JOIN main.gogoke_v37_h_owner_binding AS b
             ON b.binding_id=a.binding_id
          WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3 AND c.ticket=?4",
    )?;
    for (index, value) in [domain_id, session_id, generation, ticket]
        .iter()
        .enumerate()
    {
        statement.bind_text((index + 1) as i32, value)?;
    }
    if !statement.step_row()? {
        return Err(JournalError::Denied);
    }
    let binding = HBinding {
        process_operation_id: statement.column_text(0)?,
        ticket: statement.column_text(1)?,
        custodian_nonce: statement.column_text(2)?,
        domain_id: statement.column_text(3)?,
        generation: statement.column_text(4)?,
    };
    let claim_state = statement.column_text(5)?;
    let custody_state = statement.column_text(6)?;
    let stop_fact = statement.column_text(7)?;
    let stop_proof = statement.column_text(8)?;
    let released_recovery = claim_state == "RELEASED"
        && custody_state == "STOPPED"
        && !stop_fact.is_empty()
        && stop_fact == stop_proof;
    let state_ok = match use_case {
        BindingUse::Prepare => {
            claim_state == "COMMITTED"
                && custody_state == "ACTIVE"
                && owner_binding_active(connection, domain_id, session_id, generation)?
        }
        BindingUse::MarkUnknown => {
            matches!(claim_state.as_str(), "COMMITTED" | "UNKNOWN") && custody_state == "UNKNOWN"
        }
        BindingUse::Complete => {
            matches!(claim_state.as_str(), "COMMITTED" | "UNKNOWN")
                && matches!(custody_state.as_str(), "ACTIVE" | "UNKNOWN")
        }
        BindingUse::Read => {
            matches!(
                (claim_state.as_str(), custody_state.as_str()),
                ("COMMITTED", "ACTIVE" | "UNKNOWN")
                    | ("UNKNOWN", "UNKNOWN" | "STOPPED")
                    | ("STOPPED", "STOPPED")
            ) || released_recovery
        }
    };
    if !state_ok || statement.step_row()? {
        return Err(JournalError::Denied);
    }
    Ok(binding)
}

fn owner_binding_active(
    connection: &VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    generation: &str,
) -> Result<bool, JournalError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT COUNT(*) FROM main.gogoke_v37_h_claim AS a
           JOIN main.gogoke_v37_h_owner_binding AS b ON b.binding_id=a.binding_id
          WHERE a.domain_id=?1 AND a.session_id=?2 AND a.generation=?3
            AND b.domain_id=a.domain_id AND b.owner_id=a.session_id
            AND b.generation=a.generation AND b.instance_id=a.instance_id
            AND b.kind='SESSION' AND b.state='ACTIVE'",
    )?;
    for (index, value) in [domain_id, session_id, generation].iter().enumerate() {
        statement.bind_text((index + 1) as i32, value)?;
    }
    if !statement.step_row()? {
        return Err(JournalError::Unknown);
    }
    Ok(statement.column_text(0)? == "1" && !statement.step_row()?)
}

fn frame_matches(frame: &OriginBoundFrame, binding: &HBinding) -> Result<(), JournalError> {
    let custody = frame.custody();
    if custody.ticket.opaque() != binding.ticket
        || custody.custodian_nonce != binding.custodian_nonce
        || custody.binding.domain_id != binding.domain_id
        || custody.binding.generation != binding.generation
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

fn state_from_wire(value: &str) -> Result<JournalState, JournalError> {
    match value {
        "PREPARED" => Ok(JournalState::Prepared),
        "UNKNOWN" => Ok(JournalState::Unknown),
        "RECEIPTED" => Ok(JournalState::Receipted),
        _ => Err(JournalError::Unknown),
    }
}

fn read_row(
    connection: &VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    request_id: &str,
) -> Result<Option<StdinJournalRecord>, JournalError> {
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT operation,ticket,process_operation_id,custodian_nonce,session_id,generation,
                request_hex,phase,COALESCE(receipt_hex,''),COALESCE(receipt_status,''),
                expected_revision,COALESCE(receipt_previous_revision,''),COALESCE(receipt_revision,'')
           FROM main.gogoke_v37_h_stdin_journal
          WHERE domain_id=?1 AND request_id=?2",
    )?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, request_id)?;
    if !statement.step_row()? {
        return Ok(None);
    }
    let operation = statement.column_text(0)?;
    let ticket = statement.column_text(1)?;
    let process_operation_id = statement.column_text(2)?;
    let custodian_nonce = statement.column_text(3)?;
    let session_id = statement.column_text(4)?;
    let generation = statement.column_text(5)?;
    let request_bytes = unhex(&statement.column_text(6)?)?;
    if request_bytes.is_empty() || request_bytes.len() > MAX_FRAME_BYTES {
        return Err(JournalError::Invalid("stored original operation size"));
    }
    let state = state_from_wire(&statement.column_text(7)?)?;
    let receipt_hex = statement.column_text(8)?;
    let receipt_status_text = statement.column_text(9)?;
    let receipt_bytes = if receipt_hex.is_empty() {
        None
    } else {
        let value = unhex(&receipt_hex)?;
        frame_bytes(&value, "stored receipt frame")?;
        Some(value)
    };
    let expected_revision = parse_revision(&statement.column_text(10)?)?;
    let receipt_previous_revision = match statement.column_text(11)?.as_str() {
        "" => None,
        value => Some(parse_revision(value)?),
    };
    let receipt_revision = match statement.column_text(12)?.as_str() {
        "" => None,
        value => Some(parse_revision(value)?),
    };
    let receipt_status = match receipt_status_text.as_str() {
        "" => None,
        value => Some(status_from_wire(value).ok_or(JournalError::Unknown)?),
    };
    let record = StdinJournalRecord {
        domain_id: domain_id.to_owned(),
        request_id: request_id.to_owned(),
        operation,
        ticket,
        process_operation_id,
        custodian_nonce,
        session_id,
        generation,
        request_bytes,
        state,
        receipt_bytes,
        receipt_status,
        expected_revision,
        receipt_previous_revision,
        receipt_revision,
    };
    validate_record(&record)?;
    Ok(Some(record))
}

fn validate_record(record: &StdinJournalRecord) -> Result<(), JournalError> {
    let request = decode_request(&record.request_bytes).map_err(|_| JournalError::Unknown)?;
    if request.family != "K-SESSION"
        || request.request_id != record.request_id
        || request.target_id != record.session_id
        || request.domain_id != record.domain_id
        || request.operation != record.operation
        || request.expected_revision != record.expected_revision
        || generation_from_payload(&request)? != record.generation
    {
        return Err(JournalError::Unknown);
    }
    require_id(&record.ticket, "stored ticket")?;
    require_id(&record.process_operation_id, "stored process operation")?;
    require_id(&record.custodian_nonce, "stored custodian nonce")?;
    require_id(&record.session_id, "stored session")?;
    require_generation(&record.generation)?;
    match (
        &record.state,
        &record.receipt_bytes,
        &record.receipt_status,
        &record.receipt_previous_revision,
        &record.receipt_revision,
    ) {
        (JournalState::Prepared, None, None, None, None) => {}
        (JournalState::Unknown, None, None, None, None) => {}
        (
            JournalState::Unknown,
            Some(bytes),
            Some(V37Status::Unknown),
            Some(previous),
            Some(revision),
        ) => {
            let receipt = decode_receipt(bytes).map_err(|_| JournalError::Unknown)?;
            validate_receipt(record, &receipt, *previous, *revision)?;
        }
        (JournalState::Receipted, Some(bytes), Some(status), Some(previous), Some(revision))
            if *status != V37Status::Unknown =>
        {
            let receipt = decode_receipt(bytes).map_err(|_| JournalError::Unknown)?;
            validate_receipt(record, &receipt, *previous, *revision)?;
        }
        _ => return Err(JournalError::Unknown),
    }
    Ok(())
}

fn validate_receipt(
    record: &StdinJournalRecord,
    receipt: &V37Receipt,
    previous_revision: u64,
    revision: u64,
) -> Result<(), JournalError> {
    if receipt.family != "K-SESSION"
        || receipt.operation != record.operation
        || receipt.request_id != record.request_id
        || receipt.target_id != record.session_id
        || receipt.status.wire() != record.receipt_status.map(V37Status::wire).unwrap_or("")
        || receipt.previous_revision != previous_revision
        || receipt.revision != revision
        || receipt.revision < receipt.previous_revision
    {
        return Err(JournalError::Unknown);
    }
    Ok(())
}

fn binding_matches(record: &StdinJournalRecord, binding: &HBinding) -> Result<(), JournalError> {
    if record.process_operation_id != binding.process_operation_id
        || record.ticket != binding.ticket
        || record.custodian_nonce != binding.custodian_nonce
        || record.domain_id != binding.domain_id
        || record.generation != binding.generation
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

fn input_matches(
    input: &StdinRequest<'_>,
    request: &super::V37Request,
    record: &StdinJournalRecord,
) -> Result<(), JournalError> {
    if request.request_id != record.request_id
        || input.request_bytes != record.request_bytes
        || input.domain_id != record.domain_id
        || input.session_id != record.session_id
        || input.ticket != record.ticket
        || input.generation != record.generation
    {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

fn existing_decision(record: StdinJournalRecord) -> JournalDecision {
    let disposition = match record.state {
        JournalState::Prepared => PrepareDisposition::Replayed,
        JournalState::Unknown => PrepareDisposition::Unknown,
        JournalState::Receipted => PrepareDisposition::Completed,
    };
    JournalDecision {
        disposition,
        record,
    }
}

/// Persist the exact bounded request before the caller writes to child stdin.
/// A replayed PREPARED row is returned as `Replayed`; it is never a write
/// permission.  UNKNOWN and RECEIPTED rows fence the original request ID.
pub(crate) fn prepare_stdin_request(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
) -> Result<JournalDecision, JournalError> {
    let request = parse_request(input)?;
    prepare_decoded(connection, input, &request)
}

/// A native User operation is retained byte for byte. It is not the provider
/// command written to stdin and does not acquire a synthetic LF for identity.
pub(crate) fn prepare_codex_request(
    connection: &mut VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
) -> Result<JournalDecision, JournalError> {
    let request = parse_operation(input, false)?;
    if !matches!(request.operation.as_str(),"send"|"append-without-turn") { return Err(JournalError::Invalid("Codex send operation")); }
    prepare_decoded(connection, input, &request)
}

fn acp_send_request(input: &StdinRequest<'_>)
    -> Result<(super::V37Request, String, AcpSendIdentity), JournalError> {
    let request = parse_operation(input, false)?;
    if request.operation != "send" || request.payload.len() != 2 {
        return Err(JournalError::Invalid("ACP prompt requires original send"));
    }
    let text = payload_string(&request, "body")?;
    let digest = crate::store::digest::sha256_hex(input.request_bytes);
    let identity = AcpSendIdentity {
        step_id: format!("acp-send-{}", &digest[..40]),
        rpc_id: acp::RpcId::String(format!("gogoke-acp-send-{}", &digest[..40])),
    };
    Ok((request, text, identity))
}

fn claude_send_request(input: &StdinRequest<'_>)
    -> Result<(super::V37Request, String, ClaudeSendIdentity), JournalError> {
    let request = parse_operation(input, false)?;
    if request.operation != "send" || request.payload.len() != 2 {
        return Err(JournalError::Invalid("Claude User requires original send"));
    }
    let text = payload_string(&request, "body")?;
    let digest = crate::store::digest::sha256_hex(input.request_bytes);
    let identity = ClaudeSendIdentity {
        step_id: format!("claude-send-{}", &digest[..40]),
        // Stable UUIDv5-shaped identifier from this exact original User frame.
        // The CLI may echo it; an altered or absent echo cannot prove delivery.
        uuid: format!("{}-{}-5{}-8{}-{}", &digest[..8], &digest[8..12],
            &digest[13..16], &digest[17..20], &digest[20..32]),
    };
    Ok((request, text, identity))
}

/// The published SDK's initialize control ACK is a process handshake, not a
/// native session ID. A User input is admitted only after this exact H/A ACK.
fn claude_initialize_observed_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &ClaudeSendInput<'_>,
) -> Result<(), JournalError> {
    let (step_id, request_id) = super::rpc_journal::claude_initialize_identity(
        input.open_request_bytes);
    let command = commands::ClaudeCommand::Initialize { request_id: &request_id };
    let step = super::rpc_journal::ClaudeStep {
        domain_id: input.user.domain_id, session_id: input.user.session_id,
        open_request_id: input.open_request_id,
        open_request_bytes: input.open_request_bytes, step_id: &step_id,
        custody: input.custody, command: &command,
    };
    match super::rpc_journal::read_observed_claude_ack_in_transaction(
        connection, owner, &step)? {
        Some((stream_json::ClaudeData::ControlResponse { success: true, .. }, _)) => Ok(()),
        _ => Err(JournalError::Unknown),
    }
}

/// Original User row and exact Claude stdin echo step are inserted atomically.
/// PREPARED/UNKNOWN/RECEIPTED readback never grants a second physical write.
pub(crate) fn prepare_claude_send_request(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &ClaudeSendInput<'_>,
) -> Result<ClaudeSendPrepared, JournalError> {
    let (request, text, identity) = claude_send_request(&input.user)?;
    in_transaction(connection, |connection| {
        claude_initialize_observed_in_transaction(connection, owner, input)?;
        if read_row(connection, input.user.domain_id, &request.request_id)?.is_none() {
            let current = h_binding(connection, input.user.domain_id,
                input.user.session_id, input.user.ticket, input.user.generation,
                BindingUse::Prepare)?;
            let claim = Statement::prepare(connection.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_h_claim WHERE domain_id=?1
                  AND session_id=?2 AND generation=?3 AND process_operation_id=?4
                  AND state='COMMITTED' AND revision=?5")?;
            for (index, value) in [input.user.domain_id, input.user.session_id,
                input.user.generation, current.process_operation_id.as_str()].iter().enumerate() {
                claim.bind_text((index + 1) as i32, value)?;
            }
            claim.bind_i64(5, i64::try_from(request.expected_revision)
                .map_err(|_| JournalError::Invalid("request revision"))?)?;
            if !claim.step_row()? || claim.step_row()? { return Err(JournalError::Conflict); }
            drop(claim);
        }
        let user = prepare_decoded_in_transaction(connection, &input.user, &request)?;
        if user.disposition != PrepareDisposition::Prepared {
            return Ok(ClaudeSendPrepared { user, identity: identity.clone(),
                bytes: Vec::new(), write_permitted: false });
        }
        let command = commands::ClaudeCommand::User {
            uuid: &identity.uuid, text: &text,
        };
        let step = super::rpc_journal::ClaudeStep {
            domain_id: input.user.domain_id, session_id: input.user.session_id,
            open_request_id: input.open_request_id,
            open_request_bytes: input.open_request_bytes,
            step_id: &identity.step_id, custody: input.custody,
            command: &command,
        };
        let rpc = super::rpc_journal::prepare_claude_in_transaction(
            connection, owner, &step)?;
        if rpc.disposition != super::rpc_journal::Disposition::NewWrite {
            return Err(JournalError::Conflict);
        }
        Ok(ClaudeSendPrepared { user, identity: identity.clone(),
            bytes: rpc.bytes, write_permitted: true })
    })
}

/// One atomic H User intent plus ACP prompt intent. The caller writes the
/// returned bytes only when write_permitted; replay never changes the RPC ID.
pub(crate) fn prepare_acp_send_request(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &AcpSendInput<'_>,
) -> Result<AcpSendPrepared, JournalError> {
    let (request, text, identity) = acp_send_request(&input.user)?;
    in_transaction(connection, |connection| {
      if read_row(connection, input.user.domain_id, &request.request_id)?.is_none() {
        let current = h_binding(connection, input.user.domain_id,
            input.user.session_id, input.user.ticket, input.user.generation,
            BindingUse::Prepare)?;
        let claim = Statement::prepare(connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_claim WHERE domain_id=?1
              AND session_id=?2 AND generation=?3 AND process_operation_id=?4
              AND state='COMMITTED' AND revision=?5")?;
        for (index, value) in [input.user.domain_id, input.user.session_id,
            input.user.generation, current.process_operation_id.as_str()].iter().enumerate() {
            claim.bind_text((index + 1) as i32, value)?;
        }
        claim.bind_i64(5, i64::try_from(request.expected_revision)
            .map_err(|_| JournalError::Invalid("request revision"))?)?;
        if !claim.step_row()? || claim.step_row()? { return Err(JournalError::Conflict); }
        drop(claim);
      }
        let user = prepare_decoded_in_transaction(connection, &input.user, &request)?;
        if user.disposition != PrepareDisposition::Prepared {
            return Ok(AcpSendPrepared { user, identity: identity.clone(),
                bytes: Vec::new(), write_permitted: false });
        }
        let native_session = super::rpc_journal::observed_acp_session_id_in_transaction(
            connection, input.user.domain_id, input.user.session_id,
            input.open_request_id, input.open_request_bytes, input.custody, false)?;
        let command = commands::AcpCommand::Prompt {
            session_id: &native_session, text: &text,
        };
        let step = super::rpc_journal::AcpStep {
            domain_id: input.user.domain_id, session_id: input.user.session_id,
            open_request_id: input.open_request_id,
            open_request_bytes: input.open_request_bytes,
            step_id: &identity.step_id, custody: input.custody,
            rpc_id: Some(&identity.rpc_id), command: &command,
        };
        let rpc = super::rpc_journal::prepare_acp_in_transaction(connection, owner, &step)?;
        if rpc.disposition != super::rpc_journal::Disposition::NewWrite {
            return Err(JournalError::Conflict);
        }
        Ok(AcpSendPrepared { user, identity: identity.clone(),
            bytes: rpc.bytes, write_permitted: true })
    })
}

/// A has already durably captured this source from the real child. Resolve
/// its original prompt ACK, A source, and H User receipt in one transaction.
/// No caller-supplied frame, vendor result, or second stdin write is needed.
pub(crate) fn complete_acp_send_from_source(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &AcpSendInput<'_>, key: &RawSourceKey,
) -> Result<AcpSendCompleted, JournalError> {
    let (request, text, identity) = acp_send_request(&input.user)?;
    in_transaction(connection, |connection| {
        let prior = read_row(connection, input.user.domain_id, &request.request_id)?
            .ok_or(JournalError::Unknown)?;
        input_matches(&input.user, &request, &prior)?;
        let binding = h_binding(connection, input.user.domain_id, input.user.session_id,
            input.user.ticket, input.user.generation,
            if prior.state == JournalState::Receipted {
                BindingUse::Read
            } else { BindingUse::Complete })?;
        binding_matches(&prior, &binding)?;
        if input.custody.ticket.opaque() != binding.ticket
            || input.custody.custodian_nonce != binding.custodian_nonce
            || input.custody.binding.domain_id != binding.domain_id
            || input.custody.binding.generation != binding.generation {
            return Err(JournalError::Conflict);
        }
        let native_session = super::rpc_journal::observed_acp_session_id_in_transaction(
            connection, input.user.domain_id, input.user.session_id,
            input.open_request_id, input.open_request_bytes, input.custody, false)?;
        let command = commands::AcpCommand::Prompt {
            session_id: &native_session, text: &text,
        };
        let step = super::rpc_journal::AcpStep {
            domain_id: input.user.domain_id, session_id: input.user.session_id,
            open_request_id: input.open_request_id,
            open_request_bytes: input.open_request_bytes,
            step_id: &identity.step_id, custody: input.custody,
            rpc_id: Some(&identity.rpc_id), command: &command,
        };
        let (observation, raw_response) =
            super::rpc_journal::observe_acp_captured_response_in_transaction(
                connection, owner, &step, key)?;
        let (status, stop_reason) = acp_terminal_status(&observation)?;
        let revision = request.expected_revision.checked_add(1)
            .ok_or(JournalError::Invalid("revision overflow"))?;
        let receipt_identity = format!("{}\n{}\n{}\n{}",
            crate::store::digest::sha256_hex(input.user.request_bytes),
            crate::store::digest::sha256_hex(&raw_response),
            binding.process_operation_id, binding.custodian_nonce);
        let string = |value: &str| Json::String(JsonString::from_str(value));
        let mut result = std::collections::BTreeMap::from([
            (JsonString::from_str("generation"), string(input.user.generation)),
            (JsonString::from_str("receiptId"), string(&format!("rpc-{}",
                &crate::store::digest::sha256_hex(receipt_identity.as_bytes())[..40]))),
            (JsonString::from_str("deliveryBasis"), string("ACP_PROMPT_RESPONSE")),
            (JsonString::from_str("stopReason"), string(stop_reason)),
            (JsonString::from_str("sourceEpoch"), string(&key.source_epoch)),
            (JsonString::from_str("sourceCursor"), string(&key.source_cursor)),
            (JsonString::from_str("rawResponseSha256"),
                string(&crate::store::digest::sha256_hex(&raw_response))),
        ]);
        if status == V37Status::Applied {
            // The original prompt response proves this User send created and
            // ended a provider turn. It does not prove task success.
            result.insert(JsonString::from_str("createdTurn"), Json::Bool(true));
        }
        let mut receipt_bytes = encode_receipt(&request, status,
            request.expected_revision, revision, result);
        receipt_bytes.push(b'\n');
        frame_bytes(&receipt_bytes, "native ACP receipt")?;
        let receipt = decode_receipt(&receipt_bytes)
            .map_err(|_| JournalError::Invalid("native ACP receipt"))?;
        if prior.state != JournalState::Receipted {
            current_native_seat(connection, &input.user)?;
            let update = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_claim SET revision=?1
                  WHERE domain_id=?2 AND session_id=?3 AND generation=?4
                    AND state='COMMITTED' AND revision=?5")?;
            update.bind_i64(1, i64::try_from(revision)
                .map_err(|_| JournalError::Invalid("receipt revision"))?)?;
            update.bind_text(2, input.user.domain_id)?;
            update.bind_text(3, input.user.session_id)?;
            update.bind_text(4, input.user.generation)?;
            update.bind_i64(5, i64::try_from(request.expected_revision)
                .map_err(|_| JournalError::Invalid("request revision"))?)?;
            update.step_done()?;
            if changes(connection)? != 1 { return Err(JournalError::Conflict); }
        }
        let user = complete_decoded(connection, &input.user, &request,
            None, &receipt_bytes, &receipt)?;
        Ok(AcpSendCompleted { user, observation })
    })
}

fn stored_acp_prompt_matches(encoded: &[u8], id: &acp::RpcId,
    expected_text: &str) -> bool {
    let Ok(text) = std::str::from_utf8(encoded) else { return false };
    let Ok(Json::Object(fields)) = Parser::parse(text.trim_end_matches('\n')) else {
        return false;
    };
    let key = |name| JsonString::from_str(name);
    if !matches!(fields.get(&key("jsonrpc")), Some(Json::String(value))
        if value.to_well_formed_string().as_deref() == Some("2.0"))
        || !matches!(fields.get(&key("method")), Some(Json::String(value))
            if value.to_well_formed_string().as_deref() == Some("session/prompt")) {
        return false;
    }
    let id_matches = match (fields.get(&key("id")), id) {
        (Some(Json::String(value)), acp::RpcId::String(expected)) =>
            value.to_well_formed_string().as_deref() == Some(expected),
        (Some(Json::Number(value)), acp::RpcId::Number(expected)) =>
            value.parse::<i64>().ok() == Some(*expected),
        _ => false,
    };
    if !id_matches { return false; }
    let Some(Json::Object(params)) = fields.get(&key("params")) else { return false };
    if !matches!(params.get(&key("sessionId")), Some(Json::String(value))
        if value.to_well_formed_string().is_some_and(|value| !value.is_empty())) {
        return false;
    }
    let Some(Json::Array(prompt)) = params.get(&key("prompt")) else { return false };
    if prompt.len() != 1 { return false; }
    let Json::Object(content) = &prompt[0] else { return false };
    matches!(content.get(&key("type")), Some(Json::String(value))
        if value.to_well_formed_string().as_deref() == Some("text"))
        && matches!(content.get(&key("text")), Some(Json::String(value))
            if value.to_well_formed_string().as_deref() == Some(expected_text))
}

/// Restart/readback uses only the original H User row, RPC row, and A source.
/// It never reconstructs an OriginBoundFrame or contacts the provider.
pub(crate) fn read_acp_send_completed(
    connection: &VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
    key: &RawSourceKey,
) -> Result<Option<AcpSendCompleted>, JournalError> {
    let (request, text, identity) = acp_send_request(input)?;
    let record = read_stdin_journal(connection, &StdinJournalKey {
        domain_id: input.domain_id, request_id: &request.request_id,
        session_id: input.session_id, ticket: input.ticket,
        generation: input.generation,
    })?.ok_or(JournalError::Unknown)?;
    input_matches(input, &request, &record)?;
    if record.state != JournalState::Receipted { return Ok(None); }
    let source = ledger::read_captured_raw_source(connection, &key.operation_id,
        &key.source_epoch, &key.source_cursor)?
        .ok_or(JournalError::Denied)?;
    if source.state != RawSourceState::NoEvent
        || source.no_event_reason.as_deref() != Some("ACP_RPC_RESPONSE")
        || key.operation_id != record.process_operation_id
        || source.process_ticket != record.ticket
        || source.custodian_nonce != record.custodian_nonce
        || source.domain_id != record.domain_id
        || source.session_id != record.session_id
        || source.generation != record.generation {
        return Err(JournalError::Denied);
    }
    let rpc = Statement::prepare(connection.as_ptr(),
        "SELECT s.command_hex FROM main.gogoke_v37_rpc_steps s
           JOIN main.gogoke_v37_h_process_episode e
             ON e.domain_id=s.domain_id AND e.session_id=s.session_id
            AND e.generation=s.generation AND e.process_operation_id=s.process_operation_id
            AND e.request_id=s.open_request_id
          WHERE s.domain_id=?1 AND s.session_id=?2
            AND s.step_id=?3 AND s.process_operation_id=?4 AND s.ticket=?5
            AND s.custodian_nonce=?6 AND s.generation=?7 AND s.phase='OBSERVED'
            AND s.source_epoch=?8 AND s.source_cursor=?9")?;
    for (index, value) in [record.domain_id.as_str(), record.session_id.as_str(),
        identity.step_id.as_str(), record.process_operation_id.as_str(),
        record.ticket.as_str(), record.custodian_nonce.as_str(),
        record.generation.as_str(), key.source_epoch.as_str(),
        key.source_cursor.as_str()].iter().enumerate() {
        rpc.bind_text((index + 1) as i32, value)?;
    }
    if !rpc.step_row()? { return Err(JournalError::Denied); }
    let command = unhex(&rpc.column_text(0)?)?;
    if rpc.step_row()? || !stored_acp_prompt_matches(&command, &identity.rpc_id, &text) {
        return Err(JournalError::Denied);
    }
    let receipt_bytes = record.receipt_bytes.as_ref().ok_or(JournalError::Unknown)?;
    let receipt = decode_receipt(receipt_bytes)
        .map_err(|_| JournalError::Invalid("stored ACP receipt"))?;
    let status = receipt.status;
    if receipt.previous_revision != request.expected_revision
        || receipt.revision != request.expected_revision.checked_add(1)
            .ok_or(JournalError::Invalid("revision overflow"))? {
        return Err(JournalError::Conflict);
    }
    let result = receipt.into_result();
    if status == V37Status::Applied
        && !matches!(result.get(&JsonString::from_str("createdTurn")),
            Some(Json::Bool(true))) {
        return Err(JournalError::Conflict);
    }
    let field = |name| -> Option<String> {
        match result.get(&JsonString::from_str(name)) {
            Some(Json::String(value)) => value.to_well_formed_string(),
            _ => None,
        }
    };
    let pending = acp::Pending { id: &identity.rpc_id,
        method: acp::PendingMethod::SessionPrompt, requested_session_id: None };
    let observation = acp::decode(&source.raw_bytes, Some(&pending)).map_err(|error|
        JournalError::Rpc(super::rpc_journal::RpcJournalError::AcpDecode {
            reason: error.reason, raw_frame: error.raw_frame }))?;
    let (expected_status, stop_reason) = acp_terminal_status(&observation)?;
    let receipt_identity = format!("{}\n{}\n{}\n{}",
        crate::store::digest::sha256_hex(input.request_bytes),
        crate::store::digest::sha256_hex(&source.raw_bytes),
        record.process_operation_id, record.custodian_nonce);
    let expected_receipt_id = format!("rpc-{}",
        &crate::store::digest::sha256_hex(receipt_identity.as_bytes())[..40]);
    if status != expected_status
        || field("receiptId").as_deref() != Some(expected_receipt_id.as_str())
        || field("generation").as_deref() != Some(input.generation)
        || field("deliveryBasis").as_deref() != Some("ACP_PROMPT_RESPONSE")
        || field("stopReason").as_deref() != Some(stop_reason)
        || field("sourceEpoch").as_deref() != Some(key.source_epoch.as_str())
        || field("sourceCursor").as_deref() != Some(key.source_cursor.as_str())
        || field("rawResponseSha256").as_deref()
            != Some(crate::store::digest::sha256_hex(&source.raw_bytes).as_str()) {
        return Err(JournalError::Conflict);
    }
    Ok(Some(AcpSendCompleted {
        user: existing_decision(record), observation,
    }))
}

fn with_acp_send_step<T>(connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AcpSendInput<'_>, allow_unknown_claim: bool,
    action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>,
        &super::rpc_journal::AcpStep<'_>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    let (request, text, identity) = acp_send_request(&input.user)?;
    let record = read_row(connection, input.user.domain_id, &request.request_id)?
        .ok_or(JournalError::Unknown)?;
    input_matches(&input.user, &request, &record)?;
    let native_session = super::rpc_journal::observed_acp_session_id_in_transaction(
        connection, input.user.domain_id, input.user.session_id,
        input.open_request_id, input.open_request_bytes, input.custody,
        allow_unknown_claim)?;
    let command = commands::AcpCommand::Prompt {
        session_id: &native_session, text: &text,
    };
    let step = super::rpc_journal::AcpStep {
        domain_id: input.user.domain_id, session_id: input.user.session_id,
        open_request_id: input.open_request_id,
        open_request_bytes: input.open_request_bytes,
        step_id: &identity.step_id, custody: input.custody,
        rpc_id: Some(&identity.rpc_id), command: &command,
    };
    action(connection, &step)
}

/// Exact persistent writer success advances only the ACP RPC row to WRITTEN.
/// The User row remains PREPARED until its original prompt response appears.
pub(crate) fn mark_acp_send_written(connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, input: &AcpSendInput<'_>) -> Result<(), JournalError> {
    in_transaction(connection, |connection| with_acp_send_step(connection, input, false,
        |connection, step| super::rpc_journal::mark_acp_written_in_transaction(
            connection, owner, step).map_err(JournalError::from)))
}

/// An uncertain physical write is terminal for both intents. H must first
/// record its original process UNKNOWN custody; waiting for a long prompt is
/// never a reason to call this API.
pub(crate) fn mark_acp_send_write_unknown(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &AcpSendInput<'_>, original_error: &str,
) -> Result<JournalDecision, JournalError> {
    let (request, _, _) = acp_send_request(&input.user)?;
    in_transaction(connection, |connection| with_acp_send_step(connection, input, true,
        |connection, step| {
            super::rpc_journal::mark_acp_unknown_in_transaction(connection, owner,
                step, original_error)?;
            mark_decoded_unknown_in_transaction(connection, &input.user, &request)
        }))
}

fn with_claude_send_step<T>(connection: &mut VerifiedDatabaseConnection<'_>,
    input: &ClaudeSendInput<'_>,
    action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>,
        &super::rpc_journal::ClaudeStep<'_>) -> Result<T, JournalError>,
) -> Result<T, JournalError> {
    let (request, text, identity) = claude_send_request(&input.user)?;
    let record = read_row(connection, input.user.domain_id, &request.request_id)?
        .ok_or(JournalError::Unknown)?;
    input_matches(&input.user, &request, &record)?;
    let command = commands::ClaudeCommand::User { uuid: &identity.uuid, text: &text };
    let step = super::rpc_journal::ClaudeStep {
        domain_id: input.user.domain_id, session_id: input.user.session_id,
        open_request_id: input.open_request_id,
        open_request_bytes: input.open_request_bytes,
        step_id: &identity.step_id, custody: input.custody,
        command: &command,
    };
    action(connection, &step)
}

/// Exact OS writer success marks only the Claude stdin step WRITTEN. A User
/// remains PREPARED until replay ACK and terminal result are both captured.
pub(crate) fn mark_claude_send_written(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &ClaudeSendInput<'_>,
) -> Result<(), JournalError> {
    in_transaction(connection, |connection| with_claude_send_step(
        connection, input, |connection, step|
            super::rpc_journal::mark_claude_written_in_transaction(
                connection, owner, step).map_err(JournalError::from)))
}

pub(crate) fn mark_claude_send_write_unknown(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &ClaudeSendInput<'_>, original_error: &str,
) -> Result<JournalDecision, JournalError> {
    let (request, _, _) = claude_send_request(&input.user)?;
    in_transaction(connection, |connection| with_claude_send_step(
        connection, input, |connection, step| {
            super::rpc_journal::mark_claude_unknown_in_transaction(
                connection, owner, step, original_error)?;
            mark_decoded_unknown_in_transaction(connection, &input.user, &request)
        }))
}

/// A has already captured the exact User echo. It advances the H stdin step
/// only; terminal delivery still needs the original result in a later source.
pub(crate) fn observe_claude_send_echo_from_source(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &ClaudeSendInput<'_>, key: &RawSourceKey,
) -> Result<stream_json::ClaudeData, JournalError> {
    in_transaction(connection, |connection| with_claude_send_step(
        connection, input, |connection, step| {
            let (observation, _) =
                super::rpc_journal::observe_claude_captured_ack_in_transaction(
                    connection, owner, step, key)?;
            Ok(observation)
        }))
}

const CLAUDE_RESULT_NO_EVENT: &str = "CLAUDE_RESULT_RESPONSE";

/// The first real system/init in this exact A process stream supplies native
/// session identity. A caller label or control initialize ACK cannot do so.
fn claude_init_before_result(connection: &VerifiedDatabaseConnection<'_>,
    record: &StdinJournalRecord, epoch: &str, result_cursor: u64,
    wanted_session: &str) -> Result<(), JournalError> {
    let q = Statement::prepare(connection.as_ptr(),
        "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source
          WHERE operation_id=?1 AND source_epoch=?2
            AND process_ticket=?3 AND custodian_nonce=?4
            AND domain_id=?5 AND session_id=?6 AND generation=?7
            AND CAST(source_cursor AS INTEGER) < ?8
          ORDER BY CAST(source_cursor AS INTEGER)")?;
    for (index, value) in [record.process_operation_id.as_str(), epoch,
        record.ticket.as_str(), record.custodian_nonce.as_str(),
        record.domain_id.as_str(), record.session_id.as_str(),
        record.generation.as_str()].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    q.bind_i64(8, i64::try_from(result_cursor).map_err(|_| JournalError::Conflict)?)?;
    let mut found = false;
    while q.step_row()? {
        let raw = unhex(&q.column_text(0)?)?;
        match stream_json::decode_claude_line(&raw) {
            Ok(stream_json::ClaudeData::Init { session_id, .. }) => {
                if session_id != wanted_session || found { return Err(JournalError::Conflict); }
                found = true;
            }
            Ok(_) => {},
            Err(_) => return Err(JournalError::Conflict),
        }
    }
    if found { Ok(()) } else { Err(JournalError::Unknown) }
}

fn claude_result_is_first_after_echo(connection: &VerifiedDatabaseConnection<'_>,
    record: &StdinJournalRecord, epoch: &str, echo_cursor: u64,
    result_cursor: u64) -> Result<(), JournalError> {
    let q = Statement::prepare(connection.as_ptr(),
        "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source
          WHERE operation_id=?1 AND source_epoch=?2
            AND process_ticket=?3 AND custodian_nonce=?4
            AND domain_id=?5 AND session_id=?6 AND generation=?7
            AND CAST(source_cursor AS INTEGER) > ?8
            AND CAST(source_cursor AS INTEGER) < ?9
          ORDER BY CAST(source_cursor AS INTEGER)")?;
    for (index, value) in [record.process_operation_id.as_str(), epoch,
        record.ticket.as_str(), record.custodian_nonce.as_str(),
        record.domain_id.as_str(), record.session_id.as_str(),
        record.generation.as_str()].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    q.bind_i64(8, i64::try_from(echo_cursor).map_err(|_| JournalError::Conflict)?)?;
    q.bind_i64(9, i64::try_from(result_cursor).map_err(|_| JournalError::Conflict)?)?;
    while q.step_row()? {
        let raw = unhex(&q.column_text(0)?)?;
        match stream_json::decode_claude_line(&raw) {
            Ok(stream_json::ClaudeData::Result { .. }
                | stream_json::ClaudeData::UserReplay { .. }) =>
                    return Err(JournalError::Conflict),
            Ok(stream_json::ClaudeData::Unhandled {frame_type:Some(kind)})
                if kind=="user" => return Err(JournalError::Conflict),
            Ok(_) => {},
            Err(_) => return Err(JournalError::Conflict),
        }
    }
    Ok(())
}

/// Complete the original User request only after its exact UUID/text replay
/// and a later real result from the same A process stream and vendor session.
/// The receipt, claim CAS, and A terminalization share this transaction.
pub(crate) fn complete_claude_send_from_source(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    input: &ClaudeSendInput<'_>, key: &RawSourceKey,
) -> Result<ClaudeSendCompleted, JournalError> {
    let (request, text, identity) = claude_send_request(&input.user)?;
    in_transaction(connection, |connection| {
        let prior = read_row(connection, input.user.domain_id, &request.request_id)?
            .ok_or(JournalError::Unknown)?;
        input_matches(&input.user, &request, &prior)?;
        let binding = h_binding(connection, input.user.domain_id,
            input.user.session_id, input.user.ticket, input.user.generation,
            if prior.state == JournalState::Receipted {
                BindingUse::Read
            } else { BindingUse::Complete })?;
        binding_matches(&prior, &binding)?;
        if input.custody.ticket.opaque() != binding.ticket
            || input.custody.custodian_nonce != binding.custodian_nonce
            || input.custody.binding.domain_id != binding.domain_id
            || input.custody.binding.generation != binding.generation {
            return Err(JournalError::Conflict);
        }
        let command = commands::ClaudeCommand::User {
            uuid: &identity.uuid, text: &text,
        };
        let step = super::rpc_journal::ClaudeStep {
            domain_id: input.user.domain_id, session_id: input.user.session_id,
            open_request_id: input.open_request_id,
            open_request_bytes: input.open_request_bytes,
            step_id: &identity.step_id, custody: input.custody,
            command: &command,
        };
        let Some((stream_json::ClaudeData::UserReplay {
            session_id: vendor_session_id, .. }, raw_echo)) =
            super::rpc_journal::read_observed_claude_ack_in_transaction(
                connection, owner, &step)? else {
            return Err(JournalError::Unknown);
        };
        let echo = Statement::prepare(connection.as_ptr(),
            "SELECT source_epoch,source_cursor FROM main.gogoke_v37_rpc_steps
              WHERE domain_id=?1 AND session_id=?2 AND step_id=?3
                AND process_operation_id=?4 AND ticket=?5 AND custodian_nonce=?6
                AND generation=?7 AND phase='OBSERVED' AND requires_response=1")?;
        for (index, value) in [input.user.domain_id, input.user.session_id,
            identity.step_id.as_str(), binding.process_operation_id.as_str(),
            binding.ticket.as_str(), binding.custodian_nonce.as_str(),
            input.user.generation].iter().enumerate() {
            echo.bind_text((index + 1) as i32, value)?;
        }
        if !echo.step_row()? { return Err(JournalError::Unknown); }
        let echo_epoch = echo.column_text(0)?;
        let echo_cursor_raw = echo.column_text(1)?;
        let echo_cursor = echo_cursor_raw.parse::<u64>()
            .map_err(|_| JournalError::Conflict)?;
        if echo_cursor == 0 || echo_cursor > i64::MAX as u64
            || echo_cursor.to_string() != echo_cursor_raw {
            return Err(JournalError::Conflict);
        }
        if echo.step_row()? { return Err(JournalError::Conflict); }
        drop(echo);
        let result_cursor = key.source_cursor.parse::<u64>().ok()
            .filter(|cursor| *cursor > echo_cursor && *cursor <= i64::MAX as u64
                && cursor.to_string() == key.source_cursor)
            .ok_or(JournalError::Denied)?;
        if key.operation_id != binding.process_operation_id
            || key.source_epoch != echo_epoch
        {
            return Err(JournalError::Denied);
        }
        claude_init_before_result(connection, &prior, &echo_epoch,
            result_cursor, &vendor_session_id)?;
        claude_result_is_first_after_echo(connection, &prior, &echo_epoch,
            echo_cursor, result_cursor)?;
        let source = ledger::read_captured_raw_source(connection, &key.operation_id,
            &key.source_epoch, &key.source_cursor)?.ok_or(JournalError::Denied)?;
        if source.process_ticket != binding.ticket
            || source.custodian_nonce != binding.custodian_nonce
            || source.domain_id != input.user.domain_id
            || source.session_id != input.user.session_id
            || source.generation != input.user.generation {
            return Err(JournalError::Denied);
        }
        let terminal = stream_json::decode_claude_line(&source.raw_bytes)
            .map_err(|_| JournalError::Denied)?;
        let (status, subtype) = match &terminal {
            stream_json::ClaudeData::Result { session_id, subtype, is_error }
                if session_id == &vendor_session_id =>
                (if !is_error && subtype == "success" { V37Status::Applied }
                    else { V37Status::Failed }, subtype.as_str()),
            _ => return Err(JournalError::Denied),
        };
        match source.state {
            RawSourceState::Pending => {
                ledger::resolve_raw_source_no_event(connection,
                    &key.operation_id, &key.source_epoch, &key.source_cursor,
                    CLAUDE_RESULT_NO_EVENT)?;
            },
            RawSourceState::NoEvent if prior.state == JournalState::Receipted
                && source.no_event_reason.as_deref() == Some(CLAUDE_RESULT_NO_EVENT) => {},
            _ => return Err(JournalError::Conflict),
        }
        let revision = request.expected_revision.checked_add(1)
            .ok_or(JournalError::Invalid("revision overflow"))?;
        let receipt_identity = format!("{}\n{}\n{}\n{}\n{}",
            crate::store::digest::sha256_hex(input.user.request_bytes),
            crate::store::digest::sha256_hex(&raw_echo),
            crate::store::digest::sha256_hex(&source.raw_bytes),
            binding.process_operation_id, binding.custodian_nonce);
        let string = |value: &str| Json::String(JsonString::from_str(value));
        let mut result = std::collections::BTreeMap::from([
            (JsonString::from_str("generation"), string(input.user.generation)),
            (JsonString::from_str("receiptId"), string(&format!("claude-{}",
                &crate::store::digest::sha256_hex(receipt_identity.as_bytes())[..40]))),
            (JsonString::from_str("deliveryBasis"), string("CLAUDE_USER_REPLAY_AND_RESULT")),
            (JsonString::from_str("vendorSessionId"), string(&vendor_session_id)),
            (JsonString::from_str("userUuid"), string(&identity.uuid)),
            (JsonString::from_str("resultSubtype"), string(subtype)),
            (JsonString::from_str("sourceEpoch"), string(&key.source_epoch)),
            (JsonString::from_str("sourceCursor"), string(&key.source_cursor)),
            (JsonString::from_str("rawResultSha256"),
                string(&crate::store::digest::sha256_hex(&source.raw_bytes))),
        ]);
        if status == V37Status::Applied {
            result.insert(JsonString::from_str("createdTurn"), Json::Bool(true));
        }
        let mut receipt_bytes = encode_receipt(&request, status,
            request.expected_revision, revision, result);
        receipt_bytes.push(b'\n');
        frame_bytes(&receipt_bytes, "native Claude receipt")?;
        let receipt = decode_receipt(&receipt_bytes)
            .map_err(|_| JournalError::Invalid("native Claude receipt"))?;
        if prior.state != JournalState::Receipted {
            current_native_seat(connection, &input.user)?;
            let update = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_claim SET revision=?1
                  WHERE domain_id=?2 AND session_id=?3 AND generation=?4
                    AND state='COMMITTED' AND revision=?5")?;
            update.bind_i64(1, i64::try_from(revision)
                .map_err(|_| JournalError::Invalid("receipt revision"))?)?;
            update.bind_text(2, input.user.domain_id)?;
            update.bind_text(3, input.user.session_id)?;
            update.bind_text(4, input.user.generation)?;
            update.bind_i64(5, i64::try_from(request.expected_revision)
                .map_err(|_| JournalError::Invalid("request revision"))?)?;
            update.step_done()?;
            if changes(connection)? != 1 { return Err(JournalError::Conflict); }
        }
        let user = complete_decoded(connection, &input.user, &request,
            None, &receipt_bytes, &receipt)?;
        Ok(ClaudeSendCompleted { user, terminal, vendor_session_id })
    })
}

/// Strong readback of an already completed original User receipt from its
/// H/A rows. ACTIVE/UNKNOWN are historical read states only and do not grant
/// a new live action; STOPPED additionally requires the physical StopFact.
pub(crate) fn read_original_claude_send_completed(
    connection: &VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
) -> Result<Option<ClaudeSendCompleted>, JournalError> {
    let (request, text, identity) = claude_send_request(input)?;
    let Some(record) = read_row(connection, input.domain_id, &request.request_id)? else {
        return Ok(None);
    };
    input_matches(input, &request, &record)?;
    if record.state != JournalState::Receipted { return Ok(None); }
    let episode = Statement::prepare(connection.as_ptr(),
        "SELECT e.request_id FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_operation o
             ON o.domain_id=e.domain_id AND o.request_id=e.request_id
            AND o.session_id=e.session_id AND o.raw_hex=e.raw_hex
            AND o.operation='open' AND o.status IN ('APPLIED','UNKNOWN')
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
            AND c.generation=e.generation
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3
            AND e.process_operation_id=?4 AND e.old_generation IS NULL
            AND c.ticket=?5 AND c.custodian_nonce=?6
            AND i.driver_id='claude' AND i.version='2.1.196'
            AND ((e.phase='STOPPED' AND c.state='STOPPED'
                  AND e.stop_fact_id IS NOT NULL
                  AND e.stop_fact_id=c.stop_proof_hash)
              OR (e.phase IN ('ACTIVE','UNKNOWN')
                  AND c.state IN ('ACTIVE','UNKNOWN')))")?;
    for (index, value) in [record.domain_id.as_str(), record.session_id.as_str(),
        record.generation.as_str(), record.process_operation_id.as_str(),
        record.ticket.as_str(), record.custodian_nonce.as_str()].iter().enumerate() {
        episode.bind_text((index + 1) as i32, value)?;
    }
    if !episode.step_row()? { return Err(JournalError::Denied); }
    let open_request_id = episode.column_text(0)?;
    if episode.step_row()? { return Err(JournalError::Denied); }
    drop(episode);
    let echo = Statement::prepare(connection.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes),s.source_epoch,s.source_cursor
           FROM main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id
             AND r.generation=s.generation
          WHERE s.domain_id=?1 AND s.session_id=?2 AND s.generation=?3
            AND s.process_operation_id=?4 AND s.ticket=?5
            AND s.custodian_nonce=?6 AND s.step_id=?7
            AND s.open_request_id=?8
            AND s.phase='OBSERVED' AND s.requires_response=1
            AND r.state='NO_EVENT' AND r.no_event_reason='CLAUDE_STDIN_ACK'")?;
    for (index, value) in [record.domain_id.as_str(), record.session_id.as_str(),
        record.generation.as_str(), record.process_operation_id.as_str(),
        record.ticket.as_str(), record.custodian_nonce.as_str(),
        identity.step_id.as_str(), open_request_id.as_str()].iter().enumerate() {
        echo.bind_text((index + 1) as i32, value)?;
    }
    if !echo.step_row()? { return Err(JournalError::Denied); }
    let command = unhex(&echo.column_text(0)?)?;
    let raw_echo = unhex(&echo.column_text(1)?)?;
    let echo_epoch = echo.column_text(2)?;
    let echo_cursor_raw = echo.column_text(3)?;
    if echo.step_row()? { return Err(JournalError::Conflict); }
    drop(echo);
    let expected_command = commands::encode_claude(commands::ClaudeCommand::User {
        uuid: &identity.uuid, text: &text,
    }).map_err(|_| JournalError::Denied)?;
    if command != expected_command { return Err(JournalError::Denied); }
    let vendor_session_id = match stream_json::decode_claude_line(&raw_echo) {
        Ok(stream_json::ClaudeData::UserReplay { session_id, uuid, text: echoed })
            if uuid == identity.uuid && echoed == text => session_id,
        _ => return Err(JournalError::Denied),
    };
    let echo_cursor = echo_cursor_raw.parse::<u64>().ok()
        .filter(|cursor| *cursor > 0 && *cursor <= i64::MAX as u64
            && cursor.to_string() == echo_cursor_raw)
        .ok_or(JournalError::Denied)?;
    let receipt_bytes = record.receipt_bytes.as_ref().ok_or(JournalError::Unknown)?;
    let receipt = decode_receipt(receipt_bytes)
        .map_err(|_| JournalError::Invalid("stored Claude receipt"))?;
    if receipt.previous_revision != request.expected_revision
        || receipt.revision != request.expected_revision.checked_add(1)
            .ok_or(JournalError::Invalid("revision overflow"))? {
        return Err(JournalError::Conflict);
    }
    let status = receipt.status;
    let result = receipt.into_result();
    let field = |name| -> Option<String> {
        match result.get(&JsonString::from_str(name)) {
            Some(Json::String(value)) => value.to_well_formed_string(),
            _ => None,
        }
    };
    let epoch = field("sourceEpoch").ok_or(JournalError::Denied)?;
    let cursor = field("sourceCursor").ok_or(JournalError::Denied)?;
    let result_cursor = cursor.parse::<u64>().ok()
        .filter(|value| *value > echo_cursor && *value <= i64::MAX as u64
            && value.to_string() == cursor)
        .ok_or(JournalError::Denied)?;
    if epoch != echo_epoch { return Err(JournalError::Denied); }
    let source = ledger::read_captured_raw_source(connection,
        &record.process_operation_id, &epoch, &cursor)?
        .ok_or(JournalError::Denied)?;
    if source.state != RawSourceState::NoEvent
        || source.no_event_reason.as_deref() != Some(CLAUDE_RESULT_NO_EVENT)
        || source.process_ticket != record.ticket
        || source.custodian_nonce != record.custodian_nonce
        || source.domain_id != record.domain_id
        || source.session_id != record.session_id
        || source.generation != record.generation {
        return Err(JournalError::Denied);
    }
    claude_init_before_result(connection, &record, &epoch,
        result_cursor, &vendor_session_id)?;
    claude_result_is_first_after_echo(connection, &record, &epoch,
        echo_cursor, result_cursor)?;
    let terminal = stream_json::decode_claude_line(&source.raw_bytes)
        .map_err(|_| JournalError::Denied)?;
    let (expected_status, subtype) = match &terminal {
        stream_json::ClaudeData::Result { session_id, subtype, is_error }
            if session_id == &vendor_session_id =>
            (if !is_error && subtype == "success" { V37Status::Applied }
                else { V37Status::Failed }, subtype.as_str()),
        _ => return Err(JournalError::Denied),
    };
    let receipt_identity = format!("{}\n{}\n{}\n{}\n{}",
        crate::store::digest::sha256_hex(input.request_bytes),
        crate::store::digest::sha256_hex(&raw_echo),
        crate::store::digest::sha256_hex(&source.raw_bytes),
        record.process_operation_id, record.custodian_nonce);
    let expected_receipt_id = format!("claude-{}",
        &crate::store::digest::sha256_hex(receipt_identity.as_bytes())[..40]);
    if status != expected_status
        || field("receiptId").as_deref() != Some(expected_receipt_id.as_str())
        || field("deliveryBasis").as_deref() != Some("CLAUDE_USER_REPLAY_AND_RESULT")
        || field("generation").as_deref() != Some(input.generation)
        || field("vendorSessionId").as_deref() != Some(vendor_session_id.as_str())
        || field("userUuid").as_deref() != Some(identity.uuid.as_str())
        || field("resultSubtype").as_deref() != Some(subtype)
        || field("rawResultSha256").as_deref()
            != Some(crate::store::digest::sha256_hex(&source.raw_bytes).as_str())
        || matches!(result.get(&JsonString::from_str("createdTurn")), Some(Json::Bool(true)))
            != (status == V37Status::Applied) {
        return Err(JournalError::Conflict);
    }
    Ok(Some(ClaudeSendCompleted {
        user: existing_decision(record), terminal, vendor_session_id,
    }))
}

fn prepare_decoded(connection: &mut VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
    request: &super::V37Request) -> Result<JournalDecision, JournalError> {
    in_transaction(connection, |connection| prepare_decoded_in_transaction(connection, input, request))
}

fn prepare_decoded_in_transaction(connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>, request: &super::V37Request) -> Result<JournalDecision, JournalError> {
        if let Some(record) = read_row(connection, input.domain_id, &request.request_id)? {
            input_matches(input, &request, &record)?;
            let binding = h_binding(
                connection,
                input.domain_id,
                input.session_id,
                input.ticket,
                input.generation,
                BindingUse::Read,
            )?;
            binding_matches(&record, &binding)?;
            return Ok(existing_decision(record));
        }
        let binding = h_binding(
            connection,
            input.domain_id,
            input.session_id,
            input.ticket,
            input.generation,
            BindingUse::Prepare,
        )?;
        let unresolved = Statement::prepare(
            connection.as_ptr(),
            "SELECT COUNT(*) FROM main.gogoke_v37_h_stdin_journal
              WHERE domain_id=?1 AND session_id=?2 AND generation=?3
                AND phase IN ('PREPARED','UNKNOWN')",
        )?;
        for (index, value) in [input.domain_id, input.session_id, input.generation]
            .iter()
            .enumerate()
        {
            unresolved.bind_text((index + 1) as i32, value)?;
        }
        if !unresolved.step_row()? {
            return Err(JournalError::Unknown);
        }
        if unresolved.column_text(0)? != "0" {
            return Err(JournalError::Unknown);
        }
        let request_hex = hex(input.request_bytes);
        let expected_revision = request.expected_revision.to_string();
        let statement = Statement::prepare(
            connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_stdin_journal
             (domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,
              session_id,generation,request_hex,phase,expected_revision)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'PREPARED',?10)",
        )?;
        for (index, value) in [
            input.domain_id,
            &request.request_id,
            &request.operation,
            input.ticket,
            &binding.process_operation_id,
            &binding.custodian_nonce,
            input.session_id,
            input.generation,
            &request_hex,
            &expected_revision,
        ]
        .iter()
        .enumerate()
        {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        let record = read_row(connection, input.domain_id, &request.request_id)?
            .ok_or(JournalError::Unknown)?;
        binding_matches(&record, &binding)?;
        Ok(JournalDecision {
            disposition: PrepareDisposition::Prepared,
            record,
        })
}

/// Fence a request after the stdin write outcome is uncertain.  The custody
/// row must already be UNKNOWN, so a live process cannot be marked uncertain
/// by a caller that skipped the native process transition.
pub(crate) fn mark_stdin_write_unknown(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
) -> Result<JournalDecision, JournalError> {
    let request = parse_request(input)?;
    mark_decoded_unknown(connection, input, &request)
}

pub(crate) fn mark_codex_write_unknown(
    connection: &mut VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
) -> Result<JournalDecision, JournalError> {
    let request = parse_operation(input, false)?;
    if !matches!(request.operation.as_str(),"send"|"append-without-turn") { return Err(JournalError::Invalid("Codex send operation")); }
    mark_decoded_unknown(connection, input, &request)
}

fn mark_decoded_unknown(connection: &mut VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
    request: &super::V37Request) -> Result<JournalDecision, JournalError> {
    in_transaction(connection, |connection| mark_decoded_unknown_in_transaction(
        connection, input, request))
}

fn mark_decoded_unknown_in_transaction(connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>, request: &super::V37Request)
    -> Result<JournalDecision, JournalError> {
        let record = read_row(connection, input.domain_id, &request.request_id)?
            .ok_or(JournalError::Unknown)?;
        input_matches(input, &request, &record)?;
        let binding = h_binding(
            connection,
            input.domain_id,
            input.session_id,
            input.ticket,
            input.generation,
            if record.state == JournalState::Prepared {
                BindingUse::MarkUnknown
            } else {
                BindingUse::Read
            },
        )?;
        binding_matches(&record, &binding)?;
        if record.state == JournalState::Prepared {
            let statement = Statement::prepare(
                connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_stdin_journal SET phase='UNKNOWN'
                  WHERE domain_id=?1 AND request_id=?2 AND phase='PREPARED'",
            )?;
            statement.bind_text(1, input.domain_id)?;
            statement.bind_text(2, &request.request_id)?;
            statement.step_done()?;
            if changes(connection)? != 1 {
                return Err(JournalError::Conflict);
            }
            let updated = read_row(connection, input.domain_id, &request.request_id)?
                .ok_or(JournalError::Unknown)?;
            return Ok(JournalDecision {
                disposition: PrepareDisposition::Unknown,
                record: updated,
            });
        }
        Ok(existing_decision(record))
}

fn receipt_for_frame(
    input: &StdinRequest<'_>,
    request: &super::V37Request,
    frame: &OriginBoundFrame,
) -> Result<V37Receipt, JournalError> {
    frame_bytes(frame.bytes(), "receipt frame")?;
    let receipt =
        decode_receipt(frame.bytes()).map_err(|_| JournalError::Invalid("receipt frame"))?;
    if receipt.family != "K-SESSION"
        || receipt.operation != request.operation
        || receipt.request_id != request.request_id
        || receipt.target_id != input.session_id
        || receipt.revision < receipt.previous_revision
    {
        return Err(JournalError::Conflict);
    }
    Ok(receipt)
}

/// Complete the prepared row only from an output frame read by the exact
/// native process custody.  UNKNOWN adapter receipts remain UNKNOWN; a later
/// exact non-UNKNOWN receipt may settle that same request ID without writing
/// another stdin frame.
pub(crate) fn complete_stdin_request(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
    frame: &OriginBoundFrame,
) -> Result<JournalDecision, JournalError> {
    let request = parse_request(input)?;
    let receipt = receipt_for_frame(input, &request, frame)?;
    in_transaction(connection, |connection|
        complete_decoded(connection, input, &request, Some(frame), frame.bytes(), &receipt))
}

fn complete_decoded(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
    request: &super::V37Request,
    frame: Option<&OriginBoundFrame>,
    receipt_bytes: &[u8],
    receipt: &V37Receipt,
) -> Result<JournalDecision, JournalError> {
        let record = read_row(connection, input.domain_id, &request.request_id)?
            .ok_or(JournalError::Unknown)?;
        input_matches(input, &request, &record)?;
        let use_case = if record.state == JournalState::Receipted {
            BindingUse::Read
        } else {
            BindingUse::Complete
        };
        let binding = h_binding(
            connection,
            input.domain_id,
            input.session_id,
            input.ticket,
            input.generation,
            use_case,
        )?;
        binding_matches(&record, &binding)?;
        if let Some(frame) = frame { frame_matches(frame, &binding)?; }
        if let Some(existing) = record.receipt_bytes.as_deref() {
            if existing == receipt_bytes {
                return Ok(existing_decision(record));
            }
            let resolving_unknown = record.state == JournalState::Unknown
                && record.receipt_status == Some(V37Status::Unknown)
                && receipt.status != V37Status::Unknown
                && receipt.revision >= record.receipt_revision.unwrap_or(0);
            if !resolving_unknown {
                return Err(JournalError::Conflict);
            }
        }
        if record.state == JournalState::Receipted {
            return Err(JournalError::Unknown);
        }
        let next_state = if receipt.status == V37Status::Unknown {
            "UNKNOWN"
        } else {
            "RECEIPTED"
        };
        let receipt_hex = hex(receipt_bytes);
        let status = receipt.status.wire();
        let previous = receipt.previous_revision.to_string();
        let revision = receipt.revision.to_string();
        let statement = Statement::prepare(
            connection.as_ptr(),
            "UPDATE main.gogoke_v37_h_stdin_journal
                SET phase=?1,receipt_hex=?2,receipt_status=?3,
                    receipt_previous_revision=?4,receipt_revision=?5
              WHERE domain_id=?6 AND request_id=?7 AND phase IN ('PREPARED','UNKNOWN')",
        )?;
        for (index, value) in [
            next_state,
            &receipt_hex,
            status,
            &previous,
            &revision,
            input.domain_id,
            &request.request_id,
        ]
        .iter()
        .enumerate()
        {
            statement.bind_text((index + 1) as i32, value)?;
        }
        statement.step_done()?;
        if changes(connection)? != 1 {
            return Err(JournalError::Conflict);
        }
        let updated = read_row(connection, input.domain_id, &request.request_id)?
            .ok_or(JournalError::Unknown)?;
        let disposition = if updated.state == JournalState::Unknown {
            PrepareDisposition::Unknown
        } else {
            PrepareDisposition::Completed
        };
        Ok(JournalDecision {
            disposition,
            record: updated,
        })
}

fn payload_string(request: &super::V37Request, name: &'static str) -> Result<String, JournalError> {
    let Some(Json::String(value)) = request.payload.get(&JsonString::from_str(name)) else {
        return Err(JournalError::Invalid(name));
    };
    value.to_well_formed_string().filter(|text| !text.is_empty() && !text.contains('\0'))
        .ok_or(JournalError::Invalid(name))
}

fn native_thread_id(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
    binding: &HBinding,
) -> Result<String, JournalError> {
    let origin=Statement::prepare(connection.as_ptr(),
        "SELECT request_id FROM main.gogoke_v37_h_process_episode
          WHERE domain_id=?1 AND session_id=?2 AND generation=?3
            AND process_operation_id=?4")?;
    for (index,value) in [input.domain_id,input.session_id,input.generation,
        binding.process_operation_id.as_str()].iter().enumerate() {
        origin.bind_text((index+1) as i32,value)?;
    }
    if !origin.step_row()? {return Err(JournalError::Denied);}
    let request_id=origin.column_text(0)?;
    if origin.step_row()? {return Err(JournalError::Conflict);}
    super::rpc_journal::observed_thread_id(connection,input.domain_id,
        input.session_id,&binding.process_operation_id,input.generation,&request_id,
        &binding.ticket,&binding.custodian_nonce).map_err(JournalError::from)
}

fn current_native_seat(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
) -> Result<String, JournalError> {
    let query=Statement::prepare(connection.as_ptr(),
        "SELECT sb.seat_id FROM main.gogoke_v37_h_seat_binding sb
         JOIN main.gogoke_v37_h_claim h ON h.domain_id=sb.domain_id AND h.session_id=sb.session_id
           AND h.generation=sb.generation
         JOIN main.gogoke_v37_seats e ON e.domain_id=sb.domain_id AND e.seat_id=sb.seat_id
           AND e.incarnation=sb.seat_incarnation AND CAST(e.generation AS TEXT)=sb.generation
           AND e.instance_id=h.instance_id AND e.state='BUSY'
         WHERE sb.domain_id=?1 AND sb.session_id=?2 AND sb.generation=?3")?;
    for (index,value) in [input.domain_id,input.session_id,input.generation].iter().enumerate() {
        query.bind_text((index+1) as i32,value)?;
    }
    if !query.step_row()? { return Err(JournalError::Denied); }
    let seat_id=query.column_text(0)?;
    require_id(&seat_id,"native seat")?;
    if query.step_row()? { return Err(JournalError::Denied); }
    Ok(seat_id)
}

fn codex_response_observed(
    connection: &VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
    binding: &HBinding,
    encoded_command: &[u8],
    frame_bytes: &[u8],
) -> Result<(), JournalError> {
    let command_hex=hex(encoded_command);
    let query=Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_rpc_steps s
         JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
           AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
           AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
           AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
         WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
           AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.generation=?6
           AND s.phase='OBSERVED' AND s.command_hex=?7 AND r.raw_bytes=?8")?;
    for (index,value) in [input.domain_id,input.session_id,
        binding.process_operation_id.as_str(),binding.ticket.as_str(),
        binding.custodian_nonce.as_str(),input.generation,command_hex.as_str()].iter().enumerate() {
        query.bind_text((index+1) as i32,value)?;
    }
    query.bind_blob(8,frame_bytes)?;
    if !query.step_row()? || query.step_row()? { return Err(JournalError::Denied); }
    Ok(())
}

/// K-SESSION send completes only after the exact native Codex turn response
/// has been captured by A and correlated by the RPC journal. The returned
/// receipt is produced here from the original user request, never supplied by
/// Codex, Node, or the caller.
pub(crate) fn complete_codex_turn_request(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &StdinRequest<'_>,
    frame: &OriginBoundFrame,
    rpc_id: &codex_rpc::RpcId,
    command: &codex_rpc::Command,
    expected_thread_id: &str,
) -> Result<JournalDecision, JournalError> {
    in_transaction(connection, |connection| {
        let binding=h_binding(connection,input.domain_id,input.session_id,input.ticket,
            input.generation,BindingUse::Complete)?;
        frame_matches(frame,&binding)?;
        complete_codex_turn_in_transaction(connection,input,frame.bytes(),rpc_id,command,expected_thread_id)
    })
}

fn complete_codex_turn_in_transaction(
    connection: &mut VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
    response: &[u8], rpc_id: &codex_rpc::RpcId, command: &codex_rpc::Command,
    expected_thread_id: &str,
) -> Result<JournalDecision, JournalError> {
    let request=parse_operation(input, false)?;
    validate_codex_send(&request,command,expected_thread_id)?;
    let encoded_command=command.encode(Some(rpc_id))?;
    let reply=codex_rpc::decode(response,Some((rpc_id,command)))?;
    let turn_id=match reply {
        codex_rpc::Reply::Turn { turn_id, status: codex_rpc::TurnStatus::InProgress | codex_rpc::TurnStatus::Completed, .. } if request.operation=="send" => Some(turn_id),
        codex_rpc::Reply::Ack { .. } if request.operation=="append-without-turn"=>None,
        codex_rpc::Reply::RemoteError { raw_frame,.. } => return Err(JournalError::RemoteError(raw_frame)),
        _ => return Err(JournalError::Invalid("Codex turn response")),
    };
        let binding=h_binding(connection,input.domain_id,input.session_id,input.ticket,
            input.generation,BindingUse::Complete)?;
        current_native_seat(connection,input)?;
        if native_thread_id(connection,input,&binding)? != expected_thread_id {
            return Err(JournalError::Conflict);
        }
        codex_response_observed(connection,input,&binding,&encoded_command,response)?;
        let revision=request.expected_revision.checked_add(1)
            .ok_or(JournalError::Invalid("revision overflow"))?;
        let receipt_identity=format!("{}\n{}\n{}\n{}",
            crate::store::digest::sha256_hex(input.request_bytes),
            crate::store::digest::sha256_hex(&encoded_command),
            binding.process_operation_id,binding.custodian_nonce);
        let mut result=std::collections::BTreeMap::from([
            (JsonString::from_str("generation"),Json::String(JsonString::from_str(input.generation))),
            (JsonString::from_str("receiptId"),Json::String(JsonString::from_str(&format!("rpc-{}",&crate::store::digest::sha256_hex(receipt_identity.as_bytes())[..40])))),
            (JsonString::from_str("createdTurn"),Json::Bool(turn_id.is_some())) ]);
        if let Some(turn_id)=turn_id {result.insert(JsonString::from_str("turnId"),Json::String(JsonString::from_str(&turn_id)));}
        else {result.insert(JsonString::from_str("deliveryBasis"),Json::String(JsonString::from_str("NATIVE_INJECT_ITEMS_ACK")));}
        let mut receipt_bytes=encode_receipt(&request,V37Status::Applied,
            request.expected_revision,revision,result);
        receipt_bytes.push(b'\n');
        frame_bytes(&receipt_bytes,"native receipt")?;
        let receipt=decode_receipt(&receipt_bytes).map_err(|_| JournalError::Invalid("native receipt"))?;
        let prior = read_row(connection, input.domain_id, &request.request_id)?.ok_or(JournalError::Unknown)?;
        if prior.state != JournalState::Receipted {
            let update = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_claim SET revision=?1 WHERE domain_id=?2 AND session_id=?3 AND generation=?4 AND state='COMMITTED' AND revision=?5")?;
            update.bind_i64(1, i64::try_from(receipt.revision).map_err(|_| JournalError::Invalid("receipt revision"))?)?;
            update.bind_text(2, input.domain_id)?;
            update.bind_text(3, input.session_id)?;
            update.bind_text(4, input.generation)?;
            update.bind_i64(5, i64::try_from(request.expected_revision).map_err(|_| JournalError::Invalid("request revision"))?)?;
            update.step_done()?;
            if changes(connection)? != 1 { return Err(JournalError::Conflict); }
        }
        complete_decoded(connection,input,&request,None,&receipt_bytes,&receipt)
}

/// Recover only an already OBSERVED exact RPC/A outcome. No OS handle is
/// reconstructed and no provider command is sent. Absent response stays UNKNOWN.
pub(crate) fn recover_codex_turn_request(
    connection: &mut VerifiedDatabaseConnection<'_>, input: &StdinRequest<'_>,
) -> Result<Option<JournalDecision>, JournalError> {
    let request=parse_operation(input,false)?;
    in_transaction(connection, |connection| {
        let prior=read_row(connection,input.domain_id,&request.request_id)?.ok_or(JournalError::Unknown)?;
        input_matches(input,&request,&prior)?;
        if prior.state == JournalState::Receipted {
            let binding=h_binding(connection,input.domain_id,input.session_id,input.ticket,
                input.generation,BindingUse::Read)?;
            binding_matches(&prior,&binding)?;
            return Ok(Some(existing_decision(prior)));
        }
        let binding=h_binding(connection,input.domain_id,input.session_id,input.ticket,
            input.generation,BindingUse::Complete)?;
        binding_matches(&prior,&binding)?;
        let step_id=format!("{}-{}",if request.operation=="append-without-turn" {"append"} else {"send"},&crate::store::digest::sha256_hex(input.request_bytes)[..40]);
        let query=Statement::prepare(connection.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes) FROM main.gogoke_v37_rpc_steps s
             JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
               AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
               AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
               AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
             WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
               AND s.ticket=?4 AND s.custodian_nonce=?5 AND s.generation=?6
               AND s.step_id=?7 AND s.phase='OBSERVED'")?;
        for (index,value) in [input.domain_id,input.session_id,binding.process_operation_id.as_str(),
            input.ticket,binding.custodian_nonce.as_str(),input.generation,step_id.as_str()].iter().enumerate() {
            query.bind_text((index+1) as i32,value)?;
        }
        if !query.step_row()? { return Ok(None); }
        let command_bytes=unhex(&query.column_text(0)?)?;
        let response=unhex(&query.column_text(1)?)?;
        if query.step_row()? { return Err(JournalError::Conflict); }
        drop(query);
        let (id,command)=if request.operation=="append-without-turn" {codex_rpc::decode_stored_append(&command_bytes)?} else {codex_rpc::decode_stored_turn_start(&command_bytes)?};
        let thread_id=native_thread_id(connection,input,&binding)?;
        complete_codex_turn_in_transaction(connection,input,&response,&id,&command,&thread_id).map(Some)
    })
}

/// Stop first settles already observed send outcomes while the original
/// generation is still committed. Its own revision check then uses that fact.
/// Unobserved writes stay UNKNOWN; this function never sends to the child.
pub(crate) fn reconcile_observed_codex_sends(
    connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str,
    session_id: &str, generation: &str,
) -> Result<(), JournalError> {
    let query=Statement::prepare(connection.as_ptr(),
        "SELECT request_id FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND operation IN ('send','append-without-turn') AND phase IN ('PREPARED','UNKNOWN')")?;
    query.bind_text(1,domain_id)?; query.bind_text(2,session_id)?; query.bind_text(3,generation)?;
    let mut ids=Vec::new();
    while query.step_row()? { ids.push(query.column_text(0)?); }
    drop(query);
    for id in ids {
        let record=read_row(connection,domain_id,&id)?.ok_or(JournalError::Unknown)?;
        let input=StdinRequest {domain_id,session_id,ticket:&record.ticket,
            generation,request_bytes:&record.request_bytes};
        match recover_codex_turn_request(connection,&input) {
            Ok(_)=>{},
            // The original OBSERVED provider error remains in A/RPC and the
            // input remains UNKNOWN. It denies another input, not Owner's
            // ability to stop the actual process. Store/correlation failures
            // still propagate unchanged.
            Err(JournalError::RemoteError(_))=>{},
            Err(error)=>return Err(error),
        }
    }
    Ok(())
}

fn validate_codex_send(
    request: &super::V37Request,
    command: &codex_rpc::Command,
    expected_thread_id: &str,
) -> Result<(), JournalError> {
    if !matches!(request.operation.as_str(),"send"|"append-without-turn") || request.payload.len()!=2 {
        return Err(JournalError::Invalid("Codex send operation"));
    }
    let text=payload_string(&request,"body")?;
    let (thread_id,command_text)=match (request.operation.as_str(),command) {
        ("send",codex_rpc::Command::TurnStart {thread_id,text,..}) | ("append-without-turn",codex_rpc::Command::AppendWithoutTurn {thread_id,text})=>(thread_id,text),
        _=>return Err(JournalError::Invalid("Codex turn command")),
    };
    if command_text != &text || thread_id != expected_thread_id || expected_thread_id.is_empty() {
        return Err(JournalError::Conflict);
    }
    Ok(())
}

/// Read one journal row after restart without mutating the database or
/// re-preparing the request.  H custody and the stored ticket/session/
/// generation are checked before the bytes are disclosed to the Controller.
pub(crate) fn read_stdin_journal(
    connection: &VerifiedDatabaseConnection<'_>,
    key: &StdinJournalKey<'_>,
) -> Result<Option<StdinJournalRecord>, JournalError> {
    for (value, name) in [
        (key.domain_id, "domain_id"),
        (key.request_id, "request_id"),
        (key.session_id, "session_id"),
        (key.ticket, "ticket"),
    ] {
        require_id(value, name)?;
    }
    require_generation(key.generation)?;
    let Some(record) = read_row(connection, key.domain_id, key.request_id)? else {
        return Ok(None);
    };
    if record.session_id != key.session_id
        || record.ticket != key.ticket
        || record.generation != key.generation
    {
        return Err(JournalError::Conflict);
    }
    let binding = h_binding(
        connection,
        key.domain_id,
        key.session_id,
        key.ticket,
        key.generation,
        BindingUse::Read,
    )?;
    binding_matches(&record, &binding)?;
    Ok(Some(record))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::process::{NativeBinding, PrepareRequest, ProcessCustodian, ProcessLaunch};
    use crate::root::RootLock;
    use crate::store::authority;
    use crate::store::digest::content_hash;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    const REQUEST: &[u8] = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","domainId":"projectA","expectedRevision":"3","payload":{"body":"hello","generation":"1"}}
"#;

    #[test]
    fn codex_send_uses_exact_original_payload_and_thread() {
        let raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","domainId":"projectA","expectedRevision":"3","payload":{"generation":"1","body":"hello"}}
"#;
        let request=parse_request(&input(raw)).unwrap();
        let command=codex_rpc::Command::TurnStart {thread_id:"threadA".into(),
            cwd:"sealed-test-directory".into(),model:"m".into(),effort:"high".into(),text:"hello".into(),network_access:Some(true)};
        validate_codex_send(&request,&command,"threadA").unwrap();
        assert!(matches!(validate_codex_send(&request,&command,"threadB"),Err(JournalError::Conflict)));
        let changed=codex_rpc::Command::TurnStart {thread_id:"threadA".into(),
            cwd:"sealed-test-directory".into(),model:"m".into(),effort:"high".into(),text:"changed".into(),network_access:Some(true)};
        assert!(matches!(validate_codex_send(&request,&changed,"threadA"),Err(JournalError::Conflict)));
        let extra=String::from_utf8(raw.to_vec()).unwrap().replace("\"body\":\"hello\"",
            "\"body\":\"hello\",\"callerGrant\":\"fake\"");
        let extra_request=parse_request(&input(extra.as_bytes())).unwrap();
        assert!(matches!(validate_codex_send(&extra_request,&command,"threadA"),
            Err(JournalError::Invalid("Codex send operation"))));
        let inbox=String::from_utf8(raw.to_vec()).unwrap().replace("\"K-SESSION\",\"operation\":\"send\"",
            "\"K-INBOX\",\"operation\":\"steer\"");
        assert!(matches!(parse_request(&input(inbox.as_bytes())),Err(JournalError::Denied)));
    }

    #[test]
    fn acp_prompt_id_and_stored_command_bind_original_user_bytes() {
        let (_, text, identity) = acp_send_request(&input(REQUEST)).unwrap();
        assert_eq!(text, "hello");
        let encoded = commands::encode_acp(commands::Vendor::OpenCode,
            Some(&identity.rpc_id), commands::AcpCommand::Prompt {
                session_id: "observed-native-session", text: &text,
            }).unwrap();
        assert!(stored_acp_prompt_matches(&encoded, &identity.rpc_id, "hello"));
        assert!(!stored_acp_prompt_matches(&encoded, &identity.rpc_id, "other"));
        let changed = String::from_utf8(REQUEST.to_vec()).unwrap()
            .replace("\"body\":\"hello\"", "\"body\":\"other\"");
        let (_, _, other) = acp_send_request(&input(changed.as_bytes())).unwrap();
        assert_ne!(identity, other);
        assert!(!stored_acp_prompt_matches(&encoded, &other.rpc_id, "hello"));
    }

    fn input<'a>(bytes: &'a [u8]) -> StdinRequest<'a> {
        StdinRequest {
            domain_id: "projectA",
            session_id: "sessionA",
            ticket: "pct1_ticketA",
            generation: "1",
            request_bytes: bytes,
        }
    }

    #[test]
    fn observed_codex_response_recovers_original_receipt_after_write_failure() {
        // SQL fault control only. Stored RPC bytes are an explicit fixture,
        // not evidence of CLI authentication, model delivery, or M1 behavior.
        let _guard = route_b_test_guard();
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-codex-receipt-recovery-{stamp}"));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root,&path).unwrap();
        setup_schema(&mut db);
        insert_fake_custody(&mut db);
        crate::store::seat::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO gogoke_v37_instances VALUES('instanceA')").unwrap();
        db.execute("INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','seatIncarnationA','USER','LONG','instanceA','BUSY',1,1)").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA','sessionA','seatA','seatIncarnationA','1')").unwrap();
        super::super::rpc_journal::initialize_schema(&mut db).unwrap();
        db.execute("CREATE TABLE orchestration_events(event_id TEXT PRIMARY KEY,sequence INTEGER UNIQUE) STRICT").unwrap();
        crate::store::ledger::initialize_schema(&mut db).unwrap();
        let raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"send","requestId":"sendA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"generation":"1","body":"hello"}}"#;
        let stdin=input(raw);
        prepare_codex_request(&mut db,&stdin).unwrap();
        let step_id=format!("send-{}",&crate::store::digest::sha256_hex(raw)[..40]);
        let commands=[
            ("thread-start".to_owned(),codex_rpc::Command::ThreadStart {cwd:"fixture-directory".into(),model:"m".into()},
             codex_rpc::RpcId::Number(3), b"{\"id\":3,\"result\":{\"thread\":{\"id\":\"threadA\",\"cwd\":\"fixture-directory\"}}}\n".as_slice()),
            (step_id.clone(),codex_rpc::Command::TurnStart {thread_id:"threadA".into(),cwd:"fixture-directory".into(),
             model:"m".into(),effort:"high".into(),text:"hello".into(),network_access:Some(true)},codex_rpc::RpcId::Number(4),
             b"{\"id\":4,\"result\":{\"turn\":{\"id\":\"turnA\",\"status\":\"inProgress\"}}}\n".as_slice()),
        ];
        assert!(recover_codex_turn_request(&mut db,&stdin).unwrap().is_none());
        for (index,(step,command,id,response)) in commands.iter().enumerate() {
            let cursor=(index+1).to_string();
            let source=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES('processA','pct1_ticketA','nonceA','projectA','sessionA','1','epochA',?1,?2,'PENDING')").unwrap();
            source.bind_text(1,&cursor).unwrap(); source.bind_blob(2,response).unwrap(); source.step_done().unwrap();
            let rpc=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('projectA','sessionA','openA',?1,'processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','1',?2,1,'OBSERVED','epochA',?3)").unwrap();
            rpc.bind_text(1,step).unwrap(); rpc.bind_text(2,&hex(&command.encode(Some(id)).unwrap())).unwrap();
            rpc.bind_text(3,&cursor).unwrap(); rpc.step_done().unwrap();
        }
        db.execute("UPDATE main.v37_ledger_raw_source SET state='NO_EVENT',no_event_reason='CODEX_RPC_RESPONSE' WHERE operation_id='processA' AND source_cursor='1'").unwrap();
        db.execute("CREATE TRIGGER fail_original_receipt BEFORE UPDATE ON gogoke_v37_h_stdin_journal WHEN NEW.phase='RECEIPTED' BEGIN SELECT RAISE(ABORT,'receipt write fault'); END").unwrap();
        assert!(recover_codex_turn_request(&mut db,&stdin).is_err());
        let revision=Statement::prepare(db.as_ptr(),"SELECT revision FROM gogoke_v37_h_claim WHERE session_id='sessionA'").unwrap();
        assert!(revision.step_row().unwrap()); assert_eq!(revision.column_text(0).unwrap(),"1"); drop(revision);
        assert_eq!(read_row(&db,"projectA","sendA").unwrap().unwrap().state,JournalState::Prepared);
        db.execute("DROP TRIGGER fail_original_receipt").unwrap();
        // Restart loses all volatile state; persisted raw response is sufficient.
        db.close_checked().unwrap();
        let mut db=open_existing(&root,&path).unwrap();
        reconcile_observed_codex_sends(&mut db,"projectA","sessionA","1").unwrap();
        let decision=recover_codex_turn_request(&mut db,&stdin).unwrap().unwrap();
        assert_eq!(decision.record.state,JournalState::Receipted);
        assert_eq!(decision.record.request_bytes,raw);
        assert_eq!(decision.record.receipt_revision,Some(2));
        // Synthetic original provider refusal: an OBSERVED error cannot be
        // promoted to a successful injection, and cannot block native stop.
        let refused_raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"append-without-turn","requestId":"appendRefused","targetId":"sessionA","domainId":"projectA","expectedRevision":"2","payload":{"generation":"1","body":"refused fixture"}}"#;
        let refused=input(refused_raw);
        // This is a persisted historical UNKNOWN, not a newly authorized
        // write after restart made process custody uncertain.
        let historical=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,custodian_nonce,session_id,generation,request_hex,phase,expected_revision) VALUES('projectA','appendRefused','append-without-turn','pct1_ticketA','processA','nonceA','sessionA','1',?1,'UNKNOWN','2')").unwrap();
        historical.bind_text(1,&hex(refused_raw)).unwrap();historical.step_done().unwrap();drop(historical);
        let refusal=b"{\"id\":5,\"error\":{\"code\":-32603,\"message\":\"original injection refusal fixture\"}}\n";
        let refused_command=codex_rpc::Command::AppendWithoutTurn {thread_id:"threadA".into(),text:"refused fixture".into()};
        let refused_step=format!("append-{}",&crate::store::digest::sha256_hex(refused_raw)[..40]);
        let raw_insert=Statement::prepare(db.as_ptr(),"INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,no_event_reason) VALUES('processA','pct1_ticketA','nonceA','projectA','sessionA','1','epochA','3',?1,'NO_EVENT','CODEX_RPC_RESPONSE')").unwrap();
        raw_insert.bind_blob(1,refusal).unwrap();raw_insert.step_done().unwrap();drop(raw_insert);
        let rpc_insert=Statement::prepare(db.as_ptr(),"INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('projectA','sessionA','openA',?1,'processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','1',?2,1,'OBSERVED','epochA','3')").unwrap();
        rpc_insert.bind_text(1,&refused_step).unwrap();rpc_insert.bind_text(2,&hex(&refused_command.encode(Some(&codex_rpc::RpcId::Number(5))).unwrap())).unwrap();rpc_insert.step_done().unwrap();drop(rpc_insert);
        assert!(matches!(recover_codex_turn_request(&mut db,&refused),Err(JournalError::RemoteError(bytes)) if bytes.as_slice()==refusal.as_slice()));
        reconcile_observed_codex_sends(&mut db,"projectA","sessionA","1").expect("original provider refusal must not block actual stop intent");
        assert_eq!(read_row(&db,"projectA","appendRefused").unwrap().unwrap().state,JournalState::Unknown);
        db.execute("UPDATE gogoke_v37_h_claim SET state='STOPPED',revision=3,stop_fact_id='proofA' WHERE session_id='sessionA'").unwrap();
        db.execute("UPDATE gogoke_v37_h_process_episode SET phase='STOPPED',stop_fact_id='proofA' WHERE process_operation_id='processA'").unwrap();
        db.execute("UPDATE gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='proofA' WHERE operation_id='processA'").unwrap();
        let history=read_stdin_journal(&db,&StdinJournalKey {domain_id:"projectA",request_id:"sendA",
            session_id:"sessionA",ticket:"pct1_ticketA",generation:"1"}).unwrap().unwrap();
        assert_eq!(history,decision.record,"stop preserves the original send revision and receipt");
        assert_eq!(recover_codex_turn_request(&mut db,&stdin).unwrap().unwrap().record,decision.record);
        let changed=String::from_utf8(raw.to_vec()).unwrap().replace("hello","changed");
        assert!(matches!(recover_codex_turn_request(&mut db,&input(changed.as_bytes())),Err(JournalError::Conflict)));
        db.close_checked().unwrap(); drop(root); std::fs::remove_dir_all(folder).unwrap();
    }

    fn setup_schema(connection: &mut VerifiedDatabaseConnection<'_>) {
        connection
            .execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY) STRICT")
            .unwrap();
        authority::initialize_process_custody_schema(connection).unwrap();
        super::super::admission::initialize_admission_schema(connection).unwrap();
        connection.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
        connection.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','COMMITTED',1,'processA')").unwrap();
        connection.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','openA','sessionA','1',NULL,'6f70656e',0,1,'processA','instanceA','homeA','bindingA','seatA','seatIncarnationA','ACTIVE')").unwrap();
        connection.execute("INSERT INTO gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','processA')").unwrap();
    }

    fn insert_fake_custody(connection: &mut VerifiedDatabaseConnection<'_>) {
        connection.execute("INSERT INTO gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processA','pct1_ticketA','nonceA','11','1','fixture','sha256:fixture','profileA','projectA','1','ACTIVE')").unwrap();
    }

    #[test]
    fn prepare_unknown_reopen_and_conflict_never_reprepare() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-h-journal-{nonce}"));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        setup_schema(&mut db);
        insert_fake_custody(&mut db);
        db.execute(
            "UPDATE gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='bindingA'",
        )
        .unwrap();
        assert!(matches!(
            prepare_stdin_request(&mut db, &input(REQUEST)),
            Err(JournalError::Denied)
        ));
        db.execute(
            "UPDATE gogoke_v37_h_owner_binding SET state='ACTIVE' WHERE binding_id='bindingA'",
        )
        .unwrap();
        let first = prepare_stdin_request(&mut db, &input(REQUEST)).unwrap();
        assert_eq!(first.disposition, PrepareDisposition::Prepared);
        let next_id = String::from_utf8_lossy(REQUEST).replace("sendA", "sendB");
        assert!(matches!(
            prepare_stdin_request(&mut db, &input(next_id.as_bytes())),
            Err(JournalError::Unknown)
        ));
        let replay = prepare_stdin_request(&mut db, &input(REQUEST)).unwrap();
        assert_eq!(replay.disposition, PrepareDisposition::Replayed);
        let changed = REQUEST
            .windows(5)
            .position(|chunk| chunk == b"hello")
            .map(|index| {
                let mut bytes = REQUEST.to_vec();
                bytes[index..index + 5].copy_from_slice(b"world");
                bytes
            })
            .unwrap();
        assert!(matches!(
            prepare_stdin_request(&mut db, &input(&changed)),
            Err(JournalError::Conflict)
        ));
        db.execute("UPDATE gogoke_coordination_process_custody SET state='UNKNOWN' WHERE operation_id='processA'").unwrap();
        let unknown = mark_stdin_write_unknown(&mut db, &input(REQUEST)).unwrap();
        assert_eq!(unknown.disposition, PrepareDisposition::Unknown);
        assert!(matches!(
            prepare_stdin_request(&mut db, &input(next_id.as_bytes())),
            Err(JournalError::Denied)
        ));
        let request = decode_request(REQUEST).unwrap();
        let mut resolved = unknown.record.clone();
        resolved.state = JournalState::Receipted;
        resolved.receipt_status = Some(V37Status::Replayed);
        resolved.receipt_previous_revision = Some(4);
        resolved.receipt_revision = Some(5);
        let mut resolved_bytes = super::super::wire::encode_receipt(
            &request,
            V37Status::Replayed,
            4,
            5,
            std::collections::BTreeMap::new(),
        );
        resolved_bytes.push(b'\n');
        resolved.receipt_bytes = Some(resolved_bytes);
        validate_record(&resolved).unwrap();
        db.close_checked().unwrap();
        drop(root);
        let root = RootLock::acquire(&folder).unwrap();
        let mut reopened = open_existing(&root, &path).unwrap();
        super::super::admission::initialize_admission_schema(&mut reopened).unwrap();
        let key = StdinJournalKey {
            domain_id: "projectA",
            request_id: "sendA",
            session_id: "sessionA",
            ticket: "pct1_ticketA",
            generation: "1",
        };
        let recovered = read_stdin_journal(&reopened, &key).unwrap().unwrap();
        assert_eq!(recovered.request_bytes, REQUEST);
        assert_eq!(recovered.state, JournalState::Unknown);
        assert!(recovered.receipt_bytes.is_none());
        reopened.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn completion_requires_bound_adapter_receipt_and_fences_reprepare() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-h-journal-receipt-{nonce}"));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        setup_schema(&mut db);

        let request = decode_request(REQUEST).unwrap();
        let mut uncertain_receipt = super::super::wire::encode_receipt(
            &request,
            V37Status::Unknown,
            4,
            5,
            std::collections::BTreeMap::new(),
        );
        uncertain_receipt.push(b'\n');
        let mut receipt = super::super::wire::encode_receipt(
            &request,
            V37Status::Stale,
            5,
            5,
            std::collections::BTreeMap::new(),
        );
        receipt.push(b'\n');
        let receipt_path = folder.join("adapter-receipt.jsonl");
        let mut emitted = uncertain_receipt.clone();
        emitted.extend_from_slice(&receipt);
        std::fs::write(&receipt_path, &emitted).unwrap();
        let command = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        let mut launch = ProcessLaunch::new(command.clone());
        launch.arguments = vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-Command".into(),
            format!(
                "$bytes=[IO.File]::ReadAllBytes('{}');[Console]::OpenStandardOutput().Write($bytes,0,$bytes.Length)",
                receipt_path.display().to_string().replace('\'', "''")
            ),
        ];
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        let prepare = PrepareRequest {
            binding: NativeBinding {
                binary_digest_sha256: content_hash(&std::fs::read(&command).unwrap()),
                profile_id: "profileA".into(),
                domain_id: "projectA".into(),
                generation: "1".into(),
            },
            launch,
        };
        let mut custodian = ProcessCustodian::new().unwrap();
        let prepared = custodian.prepare(&prepare).unwrap();
        authority::record_prepared_process(&mut db, "processA", &prepared).unwrap();
        custodian.activate(&prepared).unwrap();
        authority::mark_process_active(&mut db, "processA", &prepared).unwrap();
        let frame = custodian
            .read_persistent_child_frame(&prepared.ticket, std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(frame.bytes(), uncertain_receipt.as_slice());
        let stdin = StdinRequest {
            domain_id: "projectA",
            session_id: "sessionA",
            ticket: prepared.ticket.opaque(),
            generation: "1",
            request_bytes: REQUEST,
        };
        let first = prepare_stdin_request(&mut db, &stdin).unwrap();
        assert_eq!(first.disposition, PrepareDisposition::Prepared);
        let uncertain = complete_stdin_request(&mut db, &stdin, &frame).unwrap();
        assert_eq!(uncertain.disposition, PrepareDisposition::Unknown);
        let next_id = String::from_utf8_lossy(REQUEST).replace("sendA", "sendB");
        assert!(matches!(
            prepare_stdin_request(
                &mut db,
                &StdinRequest {
                    domain_id: stdin.domain_id,
                    session_id: stdin.session_id,
                    ticket: stdin.ticket,
                    generation: stdin.generation,
                    request_bytes: next_id.as_bytes(),
                }
            ),
            Err(JournalError::Unknown)
        ));
        let frame = custodian
            .read_persistent_child_frame(&prepared.ticket, std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(frame.bytes(), receipt.as_slice());
        let completed = complete_stdin_request(&mut db, &stdin, &frame).unwrap();
        assert_eq!(completed.disposition, PrepareDisposition::Completed);
        assert_eq!(completed.record.state, JournalState::Receipted);
        assert_eq!(completed.record.receipt_previous_revision, Some(5));
        assert_eq!(
            completed.record.receipt_bytes.as_deref(),
            Some(frame.bytes())
        );
        let replay = prepare_stdin_request(&mut db, &stdin).unwrap();
        assert_eq!(replay.disposition, PrepareDisposition::Completed);
        drop(custodian);
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(receipt_path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn temp_shadow_is_rejected_before_journal_creation() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-h-journal-shadow-{nonce}"));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        db.execute("CREATE TEMP TABLE gogoke_v37_h_stdin_journal(request_id TEXT)")
            .unwrap();
        assert!(matches!(
            super::super::admission::initialize_admission_schema(&mut db),
            Err(super::super::admission::AdmissionError::Denied)
        ));
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
