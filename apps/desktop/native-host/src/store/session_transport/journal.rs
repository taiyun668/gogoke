//! Durable H stdin request/receipt journal.
//!
//! The request row is written on the same verified SQLite connection before a
//! caller writes a byte to the child.  A write with an uncertain outcome is
//! fenced as UNKNOWN and cannot be prepared again under a new request ID.  A
//! terminal row is accepted only when the receipt came from an
//! `OriginBoundFrame` read from the exact native process custody.

use super::{decode_receipt, decode_request, V37Receipt, V37Status};
use crate::process::OriginBoundFrame;
use crate::store::atomic::{AtomicError, Json, JsonString, Statement};
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
    RollbackUnknown(SameOpenError),
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

pub(crate) struct StdinRequest<'a> {
    /// Trusted H/session identity supplied by the native Controller path.
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    /// The process ticket is checked against the same verified custody row.
    pub(crate) ticket: &'a str,
    pub(crate) generation: &'a str,
    /// Exact bytes that will be passed to the persistent stdin writer.
    pub(crate) request_bytes: &'a [u8],
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
    for (value, name) in [
        (input.domain_id, "domain_id"),
        (input.session_id, "session_id"),
        (input.ticket, "ticket"),
    ] {
        require_id(value, name)?;
    }
    require_generation(input.generation)?;
    frame_bytes(input.request_bytes, "request frame")?;
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
        Err(error) => {
            connection
                .execute("ROLLBACK")
                .map_err(JournalError::RollbackUnknown)?;
            Err(error)
        }
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
    let statement = Statement::prepare(
        connection.as_ptr(),
        "SELECT c.operation_id,c.ticket,c.custodian_nonce,a.domain_id,a.generation,
                a.state,c.state,COALESCE(a.stop_fact_id,''),COALESCE(c.stop_proof_hash,'')
           FROM main.gogoke_v37_h_claim AS a
           JOIN main.gogoke_coordination_process_custody AS c
             ON c.operation_id=a.process_operation_id
            AND c.domain_id=a.domain_id
            AND c.generation=a.generation
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
        BindingUse::Prepare => claim_state == "COMMITTED" && custody_state == "ACTIVE",
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
    frame_bytes(&request_bytes, "stored request frame")?;
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
        || receipt.previous_revision != record.expected_revision
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
    in_transaction(connection, |connection| {
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
    in_transaction(connection, |connection| {
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
    })
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
        || receipt.previous_revision != request.expected_revision
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
    in_transaction(connection, |connection| {
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
        frame_matches(frame, &binding)?;
        if let Some(existing) = record.receipt_bytes.as_deref() {
            if existing == frame.bytes() {
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
        let receipt_hex = hex(frame.bytes());
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
    })
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

    fn input<'a>(bytes: &'a [u8]) -> StdinRequest<'a> {
        StdinRequest {
            domain_id: "projectA",
            session_id: "sessionA",
            ticket: "pct1_ticketA",
            generation: "1",
            request_bytes: bytes,
        }
    }

    fn setup_schema(connection: &mut VerifiedDatabaseConnection<'_>) {
        connection
            .execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY) STRICT")
            .unwrap();
        authority::initialize_process_custody_schema(connection).unwrap();
        super::super::admission::initialize_admission_schema(connection).unwrap();
        connection.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
        connection.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','COMMITTED',1,'processA')").unwrap();
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
        let first = prepare_stdin_request(&mut db, &input(REQUEST)).unwrap();
        assert_eq!(first.disposition, PrepareDisposition::Prepared);
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
        let mut receipt = super::super::wire::encode_receipt(
            &request,
            V37Status::Applied,
            3,
            4,
            std::collections::BTreeMap::new(),
        );
        receipt.push(b'\n');
        let command = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32")
            .join("cmd.exe");
        let mut launch = ProcessLaunch::new(command.clone());
        launch.arguments = vec![
            "/D".into(),
            "/C".into(),
            format!(
                "echo {}",
                String::from_utf8_lossy(&receipt[..receipt.len() - 1])
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
        let stdin = StdinRequest {
            domain_id: "projectA",
            session_id: "sessionA",
            ticket: prepared.ticket.opaque(),
            generation: "1",
            request_bytes: REQUEST,
        };
        let first = prepare_stdin_request(&mut db, &stdin).unwrap();
        assert_eq!(first.disposition, PrepareDisposition::Prepared);
        let completed = complete_stdin_request(&mut db, &stdin, &frame).unwrap();
        assert_eq!(completed.disposition, PrepareDisposition::Completed);
        assert_eq!(completed.record.state, JournalState::Receipted);
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
