//! C's durable rows on the verified product connection. Registration and the
//! native issuer are owned by the product entry, outside this module's scope.
use super::atomic::{AtomicError, Statement};
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};

const SCHEMA: [(&str, &str); 5] = [
    ("gogoke_v37_inbox_messages", "CREATE TABLE gogoke_v37_inbox_messages(domain_id TEXT NOT NULL,message_id TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('PENDING','PREPARED','UNKNOWN','DELIVERED','FAILED','CANCELLED')),sender_seat_id TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,body TEXT NOT NULL,queued_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),requeued_as TEXT,PRIMARY KEY(domain_id,message_id)) STRICT"),
    ("gogoke_v37_inbox_operations", "CREATE TABLE gogoke_v37_inbox_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,request_hex TEXT NOT NULL,message_id TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('PREPARED','UNKNOWN','APPLIED','FAILED','DENIED','CONFLICT')),previous_revision TEXT NOT NULL,revision TEXT NOT NULL,result_state TEXT NOT NULL,reason TEXT NOT NULL,native_receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
    ("gogoke_v37_qcards", "CREATE TABLE gogoke_v37_qcards(domain_id TEXT NOT NULL,card_id TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('OPEN','ANSWERED','EXPIRED')),request_ref TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,answer TEXT NOT NULL,PRIMARY KEY(domain_id,card_id)) STRICT"),
    ("gogoke_v37_qcard_options", "CREATE TABLE gogoke_v37_qcard_options(domain_id TEXT NOT NULL,card_id TEXT NOT NULL,option_id TEXT NOT NULL,recommended INTEGER NOT NULL CHECK(recommended IN (0,1)),PRIMARY KEY(domain_id,card_id,option_id),FOREIGN KEY(domain_id,card_id) REFERENCES gogoke_v37_qcards(domain_id,card_id)) STRICT"),
    ("gogoke_v37_qcard_operations", "CREATE TABLE gogoke_v37_qcard_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,request_hex TEXT NOT NULL,card_id TEXT NOT NULL,previous_revision TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL,request_ref TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,answer TEXT NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
];

#[derive(Debug)]
pub(crate) enum InboxError {
    Invalid(&'static str),
    Stale,
    Conflict,
    Denied,
    Unknown,
    Sqlite(AtomicError),
    Open(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
}
impl From<AtomicError> for InboxError {
    fn from(error: AtomicError) -> Self { Self::Sqlite(error) }
}
impl From<SameOpenError> for InboxError {
    fn from(error: SameOpenError) -> Self { Self::Open(error) }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Message {
    pub(crate) domain_id: String,
    pub(crate) message_id: String,
    pub(crate) revision: u64,
    pub(crate) state: String,
    pub(crate) sender_seat_id: String,
    pub(crate) seat_id: String,
    pub(crate) turn_id: String,
    pub(crate) generation: String,
    pub(crate) body: String,
    pub(crate) queued_at: String,
    pub(crate) requeued_as: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Card {
    pub(crate) domain_id: String,
    pub(crate) card_id: String,
    pub(crate) revision: u64,
    pub(crate) state: String,
    pub(crate) request_ref: String,
    pub(crate) seat_id: String,
    pub(crate) turn_id: String,
    pub(crate) generation: String,
    pub(crate) answer: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CardOperation {
    pub(crate) request_hex: String,
    pub(crate) previous_revision: u64,
    pub(crate) card: Card,
}

pub(crate) struct CardEnvelope<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) card_id: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) expected_revision: u64,
}

impl CardEnvelope<'_> {
    fn validate(&self) -> Result<(), InboxError> {
        for (value,name) in [(self.domain_id,"domain"),(self.card_id,"card"),(self.request_id,"request")] {
            if !valid_id(value) { return Err(InboxError::Invalid(name)); }
        }
        if self.request_bytes.is_empty() { return Err(InboxError::Invalid("request bytes")); }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredOperation {
    pub(crate) request_hex: String,
    pub(crate) message_id: String,
    pub(crate) phase: String,
    pub(crate) previous_revision: u64,
    pub(crate) revision: u64,
    pub(crate) result_state: String,
    pub(crate) reason: String,
    pub(crate) native_receipt_id: String,
}

fn parse_revision(value: String) -> Result<u64, InboxError> {
    if value.is_empty() || (value.len() > 1 && value.starts_with('0')) ||
        !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InboxError::Invalid("stored revision"));
    }
    value.parse().map_err(|_| InboxError::Invalid("stored revision"))
}

fn valid_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z')) && value.len() <= 128 &&
        bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn required(value: &str, name: &'static str) -> Result<(), InboxError> {
    if value.is_empty() || value != value.trim() { return Err(InboxError::Invalid(name)); }
    Ok(())
}

fn raw_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 15) as usize] as char);
    }
    output
}

fn observed_schema(connection: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE name LIKE 'gogoke_v37_inbox_%' OR name LIKE 'gogoke_v37_qcard%' ORDER BY name")?;
    let mut rows = Vec::new();
    while statement.step_row()? { rows.push((statement.column_text(0)?, statement.column_text(1)?)); }
    Ok(rows)
}

fn expected_schema() -> Vec<(String, String)> {
    let mut rows: Vec<_> = SCHEMA.iter().map(|(name, sql)| (name.to_string(), sql.to_string())).collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    rows
}

fn reject_side_effect_objects(connection: &VerifiedDatabaseConnection<'_>) -> Result<(), InboxError> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE name LIKE 'gogoke_v37_inbox_%' OR name LIKE 'gogoke_v37_qcard%' OR tbl_name LIKE 'gogoke_v37_inbox_%' OR tbl_name LIKE 'gogoke_v37_qcard%' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND (tbl_name LIKE 'gogoke_v37_inbox_%' OR tbl_name LIKE 'gogoke_v37_qcard%') LIMIT 1",
    ] {
        if Statement::prepare(connection.as_ptr(),sql)?.step_row()? {
            return Err(InboxError::Invalid("schema side effect"));
        }
    }
    Ok(())
}

/// A partial or changed family is corruption; the product opener must fail closed.
pub(crate) fn initialize_schema(connection: &mut VerifiedDatabaseConnection<'_>) -> Result<(), InboxError> {
    reject_side_effect_objects(connection)?;
    let observed = observed_schema(connection)?;
    if !observed.is_empty() {
        return if observed == expected_schema() { Ok(()) } else { Err(InboxError::Invalid("schema drift")) };
    }
    transact(connection, |connection| {
        reject_side_effect_objects(connection)?;
        if !observed_schema(connection)?.is_empty() { return Err(InboxError::Invalid("schema race")); }
        for (_, sql) in SCHEMA { connection.execute(sql)?; }
        if observed_schema(connection)? != expected_schema() { return Err(InboxError::Invalid("schema mismatch")); }
        Ok(())
    })
}

fn transact<T>(connection: &mut VerifiedDatabaseConnection<'_>,
    work: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, InboxError>) -> Result<T, InboxError> {
    connection.execute("BEGIN IMMEDIATE")?;
    match work(connection) {
        Ok(value) => {
            connection.execute("COMMIT").map_err(InboxError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => {
            connection.execute("ROLLBACK").map_err(InboxError::RollbackUnknown)?;
            Err(error)
        }
    }
}

fn read_message(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
    message_id: &str) -> Result<Option<Message>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT revision,state,sender_seat_id,seat_id,turn_id,generation,body,queued_at,COALESCE(requeued_as,'') FROM main.gogoke_v37_inbox_messages WHERE domain_id=? AND message_id=?")?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, message_id)?;
    if !statement.step_row()? { return Ok(None); }
    let requeued = statement.column_text(8)?;
    Ok(Some(Message { domain_id: domain_id.to_owned(), message_id: message_id.to_owned(),
        revision: parse_revision(statement.column_text(0)?)?, state: statement.column_text(1)?,
        sender_seat_id: statement.column_text(2)?, seat_id: statement.column_text(3)?,
        turn_id: statement.column_text(4)?, generation: statement.column_text(5)?,
        body: statement.column_text(6)?, queued_at: statement.column_text(7)?,
        requeued_as: if requeued.is_empty() { None } else { Some(requeued) } }))
}

fn read_operation(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
    request_id: &str) -> Result<Option<StoredOperation>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT request_hex,message_id,phase,previous_revision,revision,result_state,reason,native_receipt_id FROM main.gogoke_v37_inbox_operations WHERE domain_id=? AND request_id=?")?;
    statement.bind_text(1, domain_id)?;
    statement.bind_text(2, request_id)?;
    if !statement.step_row()? { return Ok(None); }
    Ok(Some(StoredOperation { request_hex: statement.column_text(0)?,
        message_id: statement.column_text(1)?, phase: statement.column_text(2)?,
        previous_revision: parse_revision(statement.column_text(3)?)?,
        revision: parse_revision(statement.column_text(4)?)?, result_state: statement.column_text(5)?,
        reason: statement.column_text(6)?, native_receipt_id: statement.column_text(7)? }))
}

/// K-INBOX check-unknown is read only. The same verified connection checks the
/// current native grant and reads the row; no side-effect callback is admitted.
pub(crate) fn check_unknown(connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str, message_id: &str, request_id: Option<&str>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<(Option<Message>, Option<StoredOperation>), InboxError> {
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        let mut message = read_message(connection,domain_id,message_id)?;
        if let Some(ref mut value) = message {
            if value.state == "PREPARED" { value.state = "UNKNOWN".to_owned(); }
        }
        let operation = match request_id {
            Some(id) => {
                let operation = read_operation(connection,domain_id,id)?;
                if operation.as_ref().is_some_and(|value| value.message_id != message_id) {
                    return Err(InboxError::Conflict);
                }
                operation
            },
            None => None,
        };
        Ok((message,operation))
    })
}

/// Stable native queue order for one recipient; no item is dropped to meet a
/// size cap. The next page starts after the returned rowid cursor.
pub(crate) fn list_pending(connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str,
    seat_id: &str, after_rowid: i64, limit: u32,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<Vec<(i64, Message)>, InboxError> {
    if after_rowid < 0 || limit == 0 || limit > 256 { return Err(InboxError::Invalid("queue page")); }
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        let statement = Statement::prepare(connection.as_ptr(), "SELECT CAST(rowid AS TEXT),message_id FROM main.gogoke_v37_inbox_messages WHERE domain_id=? AND seat_id=? AND state='PENDING' AND rowid>? ORDER BY rowid LIMIT ?")?;
        statement.bind_text(1,domain_id)?;
        statement.bind_text(2,seat_id)?;
        statement.bind_i64(3,after_rowid)?;
        statement.bind_i64(4,i64::from(limit))?;
        let mut rows = Vec::new();
        while statement.step_row()? {
            let cursor = statement.column_text(0)?.parse::<i64>().map_err(|_| InboxError::Invalid("queue cursor"))?;
            let message_id = statement.column_text(1)?;
            let message = read_message(connection,domain_id,&message_id)?.ok_or(InboxError::Unknown)?;
            rows.push((cursor,message));
        }
        Ok(rows)
    })
}

fn save_operation(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str, request_id: &str,
    request_hex: &str, message_id: &str, phase: &str, previous: u64, revision: u64,
    result_state: &str, reason: &str, native_receipt_id: &str) -> Result<(), InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_inbox_operations(domain_id,request_id,request_hex,message_id,phase,previous_revision,revision,result_state,reason,native_receipt_id) VALUES(?,?,?,?,?,?,?,?,?,?)")?;
    for (index, value) in [domain_id,request_id,request_hex,message_id,phase,&previous.to_string(),
        &revision.to_string(),result_state,reason,native_receipt_id].iter().enumerate() {
        statement.bind_text((index + 1) as i32, value)?;
    }
    statement.step_done()?;
    Ok(())
}

fn require_one_change(connection: &VerifiedDatabaseConnection<'_>) -> Result<(), InboxError> {
    let row = Statement::prepare(connection.as_ptr(), "SELECT changes()")?;
    if !row.step_row()? || row.column_text(0)? != "1" {
        return Err(InboxError::Conflict);
    }
    Ok(())
}

fn replay(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str, request_id: &str,
    message_id: &str, request_bytes: &[u8]) -> Result<Option<StoredOperation>, InboxError> {
    let previous = read_operation(connection, domain_id, request_id)?;
    if let Some(ref operation) = previous {
        if operation.request_hex != raw_hex(request_bytes) || operation.message_id != message_id {
            return Err(InboxError::Conflict);
        }
    }
    Ok(previous)
}

pub(crate) struct InboxEnvelope<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) message_id: &'a str,
    pub(crate) expected_revision: u64,
}

impl InboxEnvelope<'_> {
    fn validate(&self) -> Result<(), InboxError> {
        for (value, name) in [(self.domain_id,"domain"),(self.request_id,"request"),(self.message_id,"message")] {
            if !valid_id(value) { return Err(InboxError::Invalid(name)); }
        }
        if self.request_bytes.is_empty() { return Err(InboxError::Invalid("request bytes")); }
        Ok(())
    }
}

pub(crate) enum InboxEdit<'a> {
    Enqueue { sender_seat_id: &'a str, seat_id: &'a str, turn_id: &'a str,
        generation: &'a str, body: &'a str },
    Edit { body: &'a str },
    Cancel,
    Requeue { new_message_id: &'a str, seat_id: &'a str, turn_id: &'a str,
        generation: &'a str },
}

/// Durable CAS for queued edits. The native issuer's current grant and any
/// requeue-target eligibility must be resolved by `authorized` in this same transaction.
pub(crate) fn edit_message(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>, edit: InboxEdit<'_>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<StoredOperation, InboxError> {
    envelope.validate()?;
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,envelope.request_bytes)? {
            return Ok(prior);
        }
        let current = read_message(connection,envelope.domain_id,envelope.message_id)?;
        if current.as_ref().map_or(0, |message| message.revision) != envelope.expected_revision {
            return Err(InboxError::Stale);
        }
        let (state, next) = match edit {
            InboxEdit::Enqueue { sender_seat_id, seat_id, turn_id, generation, body } => {
                if current.is_some() { return Err(InboxError::Conflict); }
                for (value,name) in [(sender_seat_id,"sender"),(seat_id,"seat"),(turn_id,"turn"),
                    (generation,"generation"),(body,"body")] {
                    required(value,name)?;
                }
                let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_inbox_messages(domain_id,message_id,revision,state,sender_seat_id,seat_id,turn_id,generation,body,requeued_as) VALUES(?,?,'1','PENDING',?,?,?,?,?,NULL)")?;
                for (index,value) in [envelope.domain_id,envelope.message_id,sender_seat_id,
                    seat_id,turn_id,generation,body].iter().enumerate() {
                    statement.bind_text((index+1) as i32,value)?;
                }
                statement.step_done()?;
                ("PENDING",1)
            }
            InboxEdit::Edit { body } => {
                if current.as_ref().map(|message| message.state.as_str()) != Some("PENDING") { return Err(InboxError::Conflict); }
                required(body,"body")?;
                let next = envelope.expected_revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
                let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_inbox_messages SET body=?,revision=? WHERE domain_id=? AND message_id=? AND revision=? AND state='PENDING'")?;
                for (index,value) in [body,&next.to_string(),envelope.domain_id,envelope.message_id,&envelope.expected_revision.to_string()].iter().enumerate() {
                    statement.bind_text((index+1) as i32,value)?;
                }
                statement.step_done()?;
                ("PENDING",next)
            }
            InboxEdit::Cancel => {
                if current.as_ref().map(|message| message.state.as_str()) != Some("PENDING") { return Err(InboxError::Conflict); }
                let next = envelope.expected_revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
                let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_inbox_messages SET state='CANCELLED',revision=? WHERE domain_id=? AND message_id=? AND revision=? AND state='PENDING'")?;
                for (index,value) in [&next.to_string(),envelope.domain_id,envelope.message_id,&envelope.expected_revision.to_string()].iter().enumerate() {
                    statement.bind_text((index+1) as i32,value)?;
                }
                statement.step_done()?;
                ("CANCELLED",next)
            }
            InboxEdit::Requeue { new_message_id, seat_id, turn_id, generation } => {
                let old = current.as_ref().ok_or(InboxError::Conflict)?;
                if old.state != "FAILED" || old.requeued_as.is_some() { return Err(InboxError::Conflict); }
                if !valid_id(new_message_id) || new_message_id == envelope.message_id ||
                    read_message(connection,envelope.domain_id,new_message_id)?.is_some() {
                    return Err(InboxError::Conflict);
                }
                for (value,name) in [(seat_id,"seat"),(turn_id,"turn"),(generation,"generation")] {
                    required(value,name)?;
                }
                let next = envelope.expected_revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
                let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_inbox_messages(domain_id,message_id,revision,state,sender_seat_id,seat_id,turn_id,generation,body,requeued_as) VALUES(?,?,'1','PENDING',?,?,?,?,?,NULL)")?;
                for (index,value) in [envelope.domain_id,new_message_id,old.sender_seat_id.as_str(),
                    seat_id,turn_id,generation,old.body.as_str()].iter().enumerate() {
                    statement.bind_text((index+1) as i32,value)?;
                }
                statement.step_done()?;
                let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_inbox_messages SET requeued_as=?,revision=? WHERE domain_id=? AND message_id=? AND revision=? AND state='FAILED'")?;
                for (index,value) in [new_message_id,&next.to_string(),envelope.domain_id,envelope.message_id,&envelope.expected_revision.to_string()].iter().enumerate() {
                    statement.bind_text((index+1) as i32,value)?;
                }
                statement.step_done()?;
                ("FAILED",next)
            }
        };
        save_operation(connection,envelope.domain_id,envelope.request_id,&raw_hex(envelope.request_bytes),
            envelope.message_id,"APPLIED",envelope.expected_revision,next,state,"","")?;
        read_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
    })
}

/// Reserves one delivery request. An exact replay returns the stored phase;
/// neither PREPARED nor UNKNOWN may dispatch a second H send.
pub(crate) fn reserve_delivery(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>, generation: &str, turn_id: Option<&str>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<(StoredOperation, Option<Message>), InboxError> {
    envelope.validate()?;
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,envelope.request_bytes)? {
            return Ok((prior,None));
        }
        let message = read_message(connection,envelope.domain_id,envelope.message_id)?.ok_or(InboxError::Conflict)?;
        if message.revision != envelope.expected_revision { return Err(InboxError::Stale); }
        if message.state != "PENDING" || message.generation != generation ||
            turn_id.is_some_and(|turn| turn != message.turn_id) { return Err(InboxError::Conflict); }
        let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_inbox_messages SET state='PREPARED' WHERE domain_id=? AND message_id=? AND state='PENDING' AND revision=?")?;
        for (index,value) in [envelope.domain_id,envelope.message_id,&message.revision.to_string()].iter().enumerate() {
            statement.bind_text((index+1) as i32,value)?;
        }
        statement.step_done()?;
        require_one_change(connection)?;
        save_operation(connection,envelope.domain_id,envelope.request_id,&raw_hex(envelope.request_bytes),
            envelope.message_id,"PREPARED",message.revision,message.revision,"PREPARED","","")?;
        let operation = read_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)?;
        Ok((operation,Some(message)))
    })
}

/// H must provide a typed native abort fact before a prepared delivery can
/// return to PENDING. There is no constructor until H exposes that fact; a
/// Node boolean or model text cannot stand in for it.
pub(crate) struct NativeAbortProof {
    domain_id: String,
    message_id: String,
    request_id: String,
    receipt_id: String,
}

pub(crate) fn abort_delivery(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>, proof: Option<&NativeAbortProof>, reason: &str) -> Result<StoredOperation, InboxError> {
    transact(connection, |connection| {
        let prior = replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase != "PREPARED" { return Ok(prior); }
        if proof.is_some_and(|fact| fact.domain_id != envelope.domain_id ||
            fact.message_id != envelope.message_id || fact.request_id != envelope.request_id ||
            fact.receipt_id.is_empty()) { return Err(InboxError::Denied); }
        let confirmed = proof.is_some();
        let next = if confirmed { prior.revision } else { prior.revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))? };
        let state = if confirmed { "PENDING" } else { "UNKNOWN" };
        let phase = if !confirmed { "UNKNOWN" } else if reason == "DENIED" { "DENIED" } else { "CONFLICT" };
        update_delivery(connection,envelope,phase,state,"PREPARED",prior.revision,next,reason,"")?;
        read_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
    })
}

/// Called before H beginCommitted. A crash after this write leaves a durable
/// UNKNOWN, so recovery queries H by the original request ID and never sends anew.
pub(crate) fn mark_commit_unknown(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<StoredOperation, InboxError> {
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        let prior = replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase != "PREPARED" { return Ok(prior); }
        let next = prior.revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
        update_delivery(connection,envelope,"UNKNOWN","UNKNOWN","PREPARED",prior.revision,next,"","")?;
        read_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
    })
}

/// This type cannot be assembled from a string receipt. A future H adapter
/// must expose a checked constructor from its durable completion/failure fact.
pub(crate) struct NativeDeliveryProof {
    domain_id: String,
    message_id: String,
    request_id: String,
    outcome: NativeDeliveryOutcome,
}

enum NativeDeliveryOutcome { Completed { native_receipt_id: String }, Failed { reason: String } }

/// Only a typed H fact may resolve UNKNOWN. No constructor exists in C until
/// H exposes its durable, exact-request completion or confirmed failure.
pub(crate) fn settle_delivery(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>, proof: &NativeDeliveryProof) -> Result<StoredOperation, InboxError> {
    transact(connection, |connection| {
        let prior = replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase != "UNKNOWN" { return Ok(prior); }
        if proof.domain_id != envelope.domain_id || proof.message_id != envelope.message_id ||
            proof.request_id != envelope.request_id { return Err(InboxError::Denied); }
        let (phase,state,reason,receipt) = match &proof.outcome {
            NativeDeliveryOutcome::Completed { native_receipt_id } => {
                required(native_receipt_id,"native receipt")?;
                let used = Statement::prepare(connection.as_ptr(), "SELECT 1 FROM main.gogoke_v37_inbox_operations WHERE domain_id=? AND native_receipt_id=? AND phase='APPLIED' LIMIT 1")?;
                used.bind_text(1,envelope.domain_id)?;
                used.bind_text(2,native_receipt_id)?;
                if used.step_row()? { return Err(InboxError::Conflict); }
                ("APPLIED","DELIVERED","",native_receipt_id.as_str())
            }
            NativeDeliveryOutcome::Failed { reason } => { required(reason,"failure reason")?; ("FAILED","FAILED",reason.as_str(),"") }
        };
        // Completion resolves the same request's UNKNOWN; it does not consume a
        // second message revision or turn one delivery into two logical writes.
        update_delivery(connection,envelope,phase,state,"UNKNOWN",prior.revision,prior.revision,reason,receipt)?;
        read_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
    })
}

fn update_delivery(connection: &VerifiedDatabaseConnection<'_>, envelope: &InboxEnvelope<'_>,
    phase: &str, state: &str, expected_state: &str, previous: u64, next: u64, reason: &str,
    receipt: &str) -> Result<(), InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_inbox_messages SET state=?,revision=? WHERE domain_id=? AND message_id=? AND revision=? AND state=?")?;
    for (index,value) in [state,&next.to_string(),envelope.domain_id,envelope.message_id,&previous.to_string(),expected_state].iter().enumerate() {
        statement.bind_text((index+1) as i32,value)?;
    }
    statement.step_done()?;
    require_one_change(connection)?;
    let operation_phase = if expected_state == "PREPARED" { "PREPARED" } else { "UNKNOWN" };
    let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_inbox_operations SET phase=?,previous_revision=?,revision=?,result_state=?,reason=?,native_receipt_id=? WHERE domain_id=? AND request_id=? AND phase=?")?;
    for (index,value) in [phase,&envelope.expected_revision.to_string(),&next.to_string(),state,reason,receipt,
        envelope.domain_id,envelope.request_id,operation_phase].iter().enumerate() {
        statement.bind_text((index+1) as i32,value)?;
    }
    statement.step_done()?;
    require_one_change(connection)?;
    Ok(())
}

fn read_card(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
    card_id: &str) -> Result<Option<Card>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT revision,state,request_ref,seat_id,turn_id,generation,answer FROM main.gogoke_v37_qcards WHERE domain_id=? AND card_id=?")?;
    statement.bind_text(1,domain_id)?;
    statement.bind_text(2,card_id)?;
    if !statement.step_row()? { return Ok(None); }
    Ok(Some(Card { domain_id: domain_id.to_owned(), card_id: card_id.to_owned(),
        revision: parse_revision(statement.column_text(0)?)?, state: statement.column_text(1)?,
        request_ref: statement.column_text(2)?, seat_id: statement.column_text(3)?,
        turn_id: statement.column_text(4)?, generation: statement.column_text(5)?,
        answer: statement.column_text(6)? }))
}

pub(crate) fn query_card(connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str,
    card_id: &str,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<Option<Card>, InboxError> {
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        read_card(connection,domain_id,card_id)
    })
}

fn read_card_operation(connection: &VerifiedDatabaseConnection<'_>,
    domain_id: &str, request_id: &str) -> Result<Option<CardOperation>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT request_hex,card_id,previous_revision,revision,state,request_ref,seat_id,turn_id,generation,answer FROM main.gogoke_v37_qcard_operations WHERE domain_id=? AND request_id=?")?;
    statement.bind_text(1,domain_id)?;
    statement.bind_text(2,request_id)?;
    if !statement.step_row()? { return Ok(None); }
    let card_id = statement.column_text(1)?;
    Ok(Some(CardOperation { request_hex: statement.column_text(0)?,
        previous_revision: parse_revision(statement.column_text(2)?)?,
        card: Card { domain_id: domain_id.to_owned(), card_id,
            revision: parse_revision(statement.column_text(3)?)?, state: statement.column_text(4)?,
            request_ref: statement.column_text(5)?, seat_id: statement.column_text(6)?,
            turn_id: statement.column_text(7)?, generation: statement.column_text(8)?,
            answer: statement.column_text(9)? } }))
}

fn card_replay(connection: &VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>) -> Result<Option<CardOperation>, InboxError> {
    let prior = read_card_operation(connection,envelope.domain_id,envelope.request_id)?;
    if let Some(ref operation) = prior {
        if operation.request_hex != raw_hex(envelope.request_bytes) ||
            operation.card.card_id != envelope.card_id { return Err(InboxError::Conflict); }
    }
    Ok(prior)
}

fn save_card_operation(connection: &VerifiedDatabaseConnection<'_>, envelope: &CardEnvelope<'_>,
    card: &Card) -> Result<CardOperation, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_qcard_operations(domain_id,request_id,request_hex,card_id,previous_revision,revision,state,request_ref,seat_id,turn_id,generation,answer) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")?;
    for (index,value) in [envelope.domain_id,envelope.request_id,&raw_hex(envelope.request_bytes),
        envelope.card_id,&envelope.expected_revision.to_string(),&card.revision.to_string(),
        card.state.as_str(),card.request_ref.as_str(),card.seat_id.as_str(),card.turn_id.as_str(),
        card.generation.as_str(),card.answer.as_str()].iter().enumerate() {
        statement.bind_text((index+1) as i32,value)?;
    }
    statement.step_done()?;
    read_card_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
}

pub(crate) struct CardOption<'a> { pub(crate) id: &'a str, pub(crate) recommended: bool }

/// Own-card creation is allowed only after a trusted adapter observation says
/// native cards are absent. Native-card pass-through remains outside C.
pub(crate) fn raise_card(connection: &mut VerifiedDatabaseConnection<'_>, envelope: &CardEnvelope<'_>,
    request_ref: &str, seat_id: &str, turn_id: &str, generation: &str,
    options: &[CardOption<'_>],
    authorized_and_native_card_absent: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<CardOperation, InboxError> {
    envelope.validate()?;
    for (value,name) in [(request_ref,"request"),
        (seat_id,"seat"),(turn_id,"turn")] {
        if !valid_id(value) { return Err(InboxError::Invalid(name)); }
    }
    required(generation,"generation")?;
    if options.is_empty() || options.iter().filter(|option| option.recommended).count() != 1 ||
        options.iter().any(|option| !valid_id(option.id)) { return Err(InboxError::Invalid("options")); }
    let mut ids = std::collections::BTreeSet::new();
    if options.iter().any(|option| !ids.insert(option.id)) { return Err(InboxError::Invalid("duplicate option")); }
    transact(connection, |connection| {
        if !authorized_and_native_card_absent(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = card_replay(connection,envelope)? { return Ok(prior); }
        if envelope.expected_revision != 0 { return Err(InboxError::Stale); }
        if read_card(connection,envelope.domain_id,envelope.card_id)?.is_some() { return Err(InboxError::Conflict); }
        let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_qcards(domain_id,card_id,revision,state,request_ref,seat_id,turn_id,generation,answer) VALUES(?,?,'1','OPEN',?,?,?,?,'')")?;
        for (index,value) in [envelope.domain_id,envelope.card_id,request_ref,seat_id,turn_id,generation].iter().enumerate() {
            statement.bind_text((index+1) as i32,value)?;
        }
        statement.step_done()?;
        for option in options {
            let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_qcard_options(domain_id,card_id,option_id,recommended) VALUES(?,?,?,?)")?;
            statement.bind_text(1,envelope.domain_id)?;
            statement.bind_text(2,envelope.card_id)?;
            statement.bind_text(3,option.id)?;
            statement.bind_i64(4,i64::from(option.recommended))?;
            statement.step_done()?;
        }
        let card = read_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Unknown)?;
        save_card_operation(connection,envelope,&card)
    })
}

pub(crate) enum CardDecision<'a> { AnswerOption(&'a str), AnswerFree(&'a str), Expire, Recover }

/// Every decision, including expire and recover, must name the exact request,
/// seat, turn and generation. The callback additionally checks native grant.
pub(crate) fn decide_card(connection: &mut VerifiedDatabaseConnection<'_>, envelope: &CardEnvelope<'_>,
    request_ref: &str, seat_id: &str, turn_id: &str, generation: &str,
    decision: CardDecision<'_>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<CardOperation, InboxError> {
    envelope.validate()?;
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = card_replay(connection,envelope)? { return Ok(prior); }
        let card = read_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Conflict)?;
        if card.revision != envelope.expected_revision { return Err(InboxError::Stale); }
        if card.state != "OPEN" { return Err(InboxError::Conflict); }
        if card.request_ref != request_ref || card.seat_id != seat_id ||
            card.turn_id != turn_id || card.generation != generation {
            return Err(InboxError::Conflict);
        }
        let (state,answer) = match decision {
            CardDecision::AnswerOption(option) => {
                let statement = Statement::prepare(connection.as_ptr(), "SELECT 1 FROM main.gogoke_v37_qcard_options WHERE domain_id=? AND card_id=? AND option_id=?")?;
                statement.bind_text(1,envelope.domain_id)?;
                statement.bind_text(2,envelope.card_id)?;
                statement.bind_text(3,option)?;
                if !statement.step_row()? { return Err(InboxError::Conflict); }
                ("ANSWERED",option)
            }
            CardDecision::AnswerFree(text) => { required(text,"answer")?; ("ANSWERED",text) }
            CardDecision::Expire => ("EXPIRED",""),
            CardDecision::Recover => ("OPEN",""),
        };
        let next = envelope.expected_revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
        let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_qcards SET state=?,answer=?,revision=? WHERE domain_id=? AND card_id=? AND revision=? AND state='OPEN'")?;
        for (index,value) in [state,answer,&next.to_string(),envelope.domain_id,envelope.card_id,
            &envelope.expected_revision.to_string()].iter().enumerate() {
            statement.bind_text((index+1) as i32,value)?;
        }
        statement.step_done()?;
        let changed = read_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Unknown)?;
        save_card_operation(connection,envelope,&changed)
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-inbox-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut connection = create_new(&root,&database).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        run(&mut connection);
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        if let Err(error) = std::fs::remove_dir(&path) {
            eprintln!("owned inbox fixture retained: {} ({error})",path.display());
        }
    }

    #[test]
    fn durable_inbox_cas_unknown_and_receipt() {
        fixture(|connection| {
            initialize_schema(connection).unwrap();
            initialize_schema(connection).unwrap();
            let enqueue = InboxEnvelope { domain_id: "projectA", request_id: "enqueueA",
                request_bytes: b"enqueue exact wire", message_id: "messageA", expected_revision: 0 };
            let added = edit_message(connection,&enqueue,InboxEdit::Enqueue {
                sender_seat_id: "senderA",seat_id: "seatA",turn_id: "turnA",generation: "1",body: "first" },|_| Ok(true)).unwrap();
            assert_eq!(added.revision,1);
            assert_eq!(edit_message(connection,&enqueue,InboxEdit::Enqueue {
                sender_seat_id: "senderA",seat_id: "seatA",turn_id: "turnA",generation: "1",body: "first" },|_| Ok(true)).unwrap(),added);
            let changed_wire = InboxEnvelope { request_bytes: b"other wire", ..enqueue };
            assert!(matches!(edit_message(connection,&changed_wire,InboxEdit::Cancel,|_| Ok(true)),Err(InboxError::Conflict)));
            let edit = InboxEnvelope { domain_id: "projectA",request_id: "editA",
                request_bytes: b"edit exact wire",message_id: "messageA",expected_revision: 1 };
            assert_eq!(edit_message(connection,&edit,InboxEdit::Edit { body: "second" },|_| Ok(true)).unwrap().revision,2);
            let queued = list_pending(connection,"projectA","seatA",0,10,|_| Ok(true)).unwrap();
            assert_eq!(queued.len(),1);
            assert_eq!(queued[0].1.message_id,"messageA");
            assert!(!queued[0].1.queued_at.is_empty());
            let cancel = InboxEnvelope { domain_id: "projectA",request_id: "cancelA",
                request_bytes: b"cancel exact wire",message_id: "messageA",expected_revision: 1 };
            assert!(matches!(edit_message(connection,&cancel,InboxEdit::Cancel,|_| Ok(true)),Err(InboxError::Stale)));
            let deliver = InboxEnvelope { domain_id: "projectA",request_id: "deliverA",
                request_bytes: b"delivery exact wire",message_id: "messageA",expected_revision: 2 };
            let (prepared,message) = reserve_delivery(connection,&deliver,"1",Some("turnA"),|_| Ok(true)).unwrap();
            assert_eq!(prepared.phase,"PREPARED");
            assert_eq!(message.unwrap().body,"second");
            assert_eq!(check_unknown(connection,"projectA","messageA",Some("deliverA"),|_| Ok(true))
                .unwrap().0.unwrap().state,"UNKNOWN");
            assert_eq!(read_message(connection,"projectA","messageA").unwrap().unwrap().state,"PREPARED");
            let (replayed,none) = reserve_delivery(connection,&deliver,"1",Some("turnA"),|_| Ok(true)).unwrap();
            assert_eq!(replayed.phase,"PREPARED");
            assert!(none.is_none());
            assert_eq!(mark_commit_unknown(connection,&deliver,|_| Ok(true)).unwrap().phase,"UNKNOWN");
            assert_eq!(read_message(connection,"projectA","messageA").unwrap().unwrap().state,"UNKNOWN");
            assert_eq!(read_message(connection,"projectA","messageA").unwrap().unwrap().state,"UNKNOWN");
            // No typed H completion proof exists yet. A string receipt cannot
            // convert this durable UNKNOWN to DELIVERED.
            let wrong_target = InboxEnvelope { message_id: "messageB", ..deliver };
            assert!(matches!(mark_commit_unknown(connection,&wrong_target,|_| Ok(true)),
                Err(InboxError::Conflict)));
            assert!(matches!(check_unknown(connection,"projectA","messageB",Some("deliverA"),|_| Ok(true)),
                Err(InboxError::Conflict)));
        });
    }

    #[test]
    fn unproven_h_abort_cannot_requeue_or_release_the_original_send() {
        fixture(|connection| {
            initialize_schema(connection).unwrap();
            let enqueue = InboxEnvelope { domain_id: "projectA", request_id: "enqueueA",
                request_bytes: b"enqueue A", message_id: "messageA", expected_revision: 0 };
            edit_message(connection,&enqueue,InboxEdit::Enqueue {
                sender_seat_id: "senderA", seat_id: "seatA", turn_id: "turnA",
                generation: "1", body: "body" }, |_| Ok(true)).unwrap();
            let delivery = InboxEnvelope { domain_id: "projectA", request_id: "deliveryA",
                request_bytes: b"delivery A", message_id: "messageA", expected_revision: 1 };
            reserve_delivery(connection,&delivery,"1",Some("turnA"),|_| Ok(true)).unwrap();
            let unresolved = abort_delivery(connection,&delivery,None,"TURN_ENDED").unwrap();
            assert_eq!(unresolved.phase,"UNKNOWN");
            assert_eq!(read_message(connection,"projectA","messageA").unwrap().unwrap().state,"UNKNOWN");
            let requeue = InboxEnvelope { domain_id: "projectA", request_id: "requeueA",
                request_bytes: b"requeue A", message_id: "messageA", expected_revision: 2 };
            assert!(matches!(edit_message(connection,&requeue,InboxEdit::Requeue {
                new_message_id: "messageB", seat_id: "seatA", turn_id: "turnB",
                generation: "2" }, |_| Ok(true)),Err(InboxError::Conflict)));
            assert!(read_message(connection,"projectA","messageB").unwrap().is_none());
        });
    }

    #[test]
    fn own_card_has_one_recommendation_and_answer_wins_once() {
        fixture(|connection| {
            initialize_schema(connection).unwrap();
            let raise = CardEnvelope { domain_id: "projectA",card_id: "cardA",request_id: "raiseA",
                request_bytes: b"raise exact wire",expected_revision: 0 };
            let options = [CardOption { id: "yes",recommended: true },CardOption { id: "no",recommended: false }];
            let card = raise_card(connection,&raise,"questionA","seatA","turnA","1",&options,|_| Ok(true)).unwrap();
            assert_eq!(card.card.state,"OPEN");
            assert_eq!(raise_card(connection,&raise,"questionA","seatA","turnA","1",&options,|_| Ok(true)).unwrap(),card);
            let changed_wire = CardEnvelope { request_bytes: b"different wire", ..raise };
            assert!(matches!(raise_card(connection,&changed_wire,"questionA","seatA","turnA","1",
                &options,|_| Ok(true)),Err(InboxError::Conflict)));
            let recover = CardEnvelope { domain_id: "projectA",card_id: "cardA",request_id: "recoverA",
                request_bytes: b"recover exact wire",expected_revision: 1 };
            assert!(matches!(decide_card(connection,&recover,"","","","",CardDecision::Recover,
                |_| Ok(true)),Err(InboxError::Conflict)));
            assert_eq!(decide_card(connection,&recover,"questionA","seatA","turnA","1",CardDecision::Recover,
                |_| Ok(true)).unwrap().card.revision,2);
            let answer = CardEnvelope { domain_id: "projectA",card_id: "cardA",request_id: "answerA",
                request_bytes: b"answer exact wire",expected_revision: 2 };
            let done = decide_card(connection,&answer,"questionA","seatA","turnA","1",
                CardDecision::AnswerOption("yes"),|_| Ok(true)).unwrap();
            assert_eq!(done.card.state,"ANSWERED");
            assert_eq!(done.card.answer,"yes");
            let expire = CardEnvelope { domain_id: "projectA",card_id: "cardA",request_id: "expireA",
                request_bytes: b"expire exact wire",expected_revision: 2 };
            assert!(matches!(decide_card(connection,&expire,"questionA","seatA","turnA","1",
                CardDecision::Expire,|_| Ok(true)),Err(InboxError::Stale)));
        });
    }
}
