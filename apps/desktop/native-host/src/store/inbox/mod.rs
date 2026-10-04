//! C's durable rows on the verified product connection. Registration and the
//! native issuer are owned by the product entry, outside this module's scope.
use super::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use super::ledger;
use super::same_open::{SameOpenError, VerifiedDatabaseConnection};

pub(crate) mod host_rule;

const SCHEMA: [(&str, &str); 8] = [
    ("gogoke_v37_inbox_messages", "CREATE TABLE gogoke_v37_inbox_messages(domain_id TEXT NOT NULL,message_id TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('PENDING','PREPARED','UNKNOWN','DELIVERED','FAILED','CANCELLED')),sender_seat_id TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,body TEXT NOT NULL,queued_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),requeued_as TEXT,PRIMARY KEY(domain_id,message_id)) STRICT"),
    ("gogoke_v37_inbox_operations", "CREATE TABLE gogoke_v37_inbox_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,request_hex TEXT NOT NULL,message_id TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('PREPARED','UNKNOWN','APPLIED','FAILED','DENIED','CONFLICT')),previous_revision TEXT NOT NULL,revision TEXT NOT NULL,result_state TEXT NOT NULL,reason TEXT NOT NULL,native_receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
    ("gogoke_v37_qcards", "CREATE TABLE gogoke_v37_qcards(domain_id TEXT NOT NULL,card_id TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('OPEN','ANSWERED','EXPIRED')),request_ref TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,answer TEXT NOT NULL,PRIMARY KEY(domain_id,card_id)) STRICT"),
    ("gogoke_v37_qcard_options", "CREATE TABLE gogoke_v37_qcard_options(domain_id TEXT NOT NULL,card_id TEXT NOT NULL,option_id TEXT NOT NULL,recommended INTEGER NOT NULL CHECK(recommended IN (0,1)),PRIMARY KEY(domain_id,card_id,option_id),FOREIGN KEY(domain_id,card_id) REFERENCES gogoke_v37_qcards(domain_id,card_id)) STRICT"),
    ("gogoke_v37_qcard_operations", "CREATE TABLE gogoke_v37_qcard_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,request_hex TEXT NOT NULL,card_id TEXT NOT NULL,previous_revision TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL,request_ref TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,answer TEXT NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
    ("gogoke_v37_qcard_native", "CREATE TABLE gogoke_v37_qcard_native(domain_id TEXT NOT NULL,card_id TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('OPEN','ANSWER_UNKNOWN','ANSWERED','EXPIRED')),vendor_request_id TEXT NOT NULL,vendor_thread_id TEXT NOT NULL,vendor_item_id TEXT NOT NULL,auto_resolution_ms TEXT NOT NULL,question_payload TEXT NOT NULL,question_id TEXT NOT NULL,question_header TEXT NOT NULL,question_text TEXT NOT NULL,answer_shape TEXT NOT NULL CHECK(answer_shape IN ('OPTIONS','FREE_TEXT','OPTIONS_OR_FREE')),seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,answer_kind TEXT NOT NULL CHECK(answer_kind IN ('NONE','OPTION','FREE','WIRE')),answer TEXT NOT NULL,PRIMARY KEY(domain_id,card_id),UNIQUE(domain_id,seat_id,generation,vendor_request_id)) STRICT"),
    ("gogoke_v37_qcard_native_options", "CREATE TABLE gogoke_v37_qcard_native_options(domain_id TEXT NOT NULL,card_id TEXT NOT NULL,option_id TEXT NOT NULL,ordinal TEXT NOT NULL,label TEXT NOT NULL,description TEXT NOT NULL,PRIMARY KEY(domain_id,card_id,option_id),FOREIGN KEY(domain_id,card_id) REFERENCES gogoke_v37_qcard_native(domain_id,card_id)) STRICT"),
    ("gogoke_v37_qcard_native_operations", "CREATE TABLE gogoke_v37_qcard_native_operations(domain_id TEXT NOT NULL,request_id TEXT NOT NULL,request_hex TEXT NOT NULL,card_id TEXT NOT NULL,previous_revision TEXT NOT NULL,revision TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('RAISED','UNKNOWN','ANSWERED','FAILED','EXPIRED')),vendor_request_id TEXT NOT NULL,seat_id TEXT NOT NULL,turn_id TEXT NOT NULL,generation TEXT NOT NULL,answer_kind TEXT NOT NULL CHECK(answer_kind IN ('NONE','OPTION','FREE','WIRE')),answer TEXT NOT NULL,reason TEXT NOT NULL,native_receipt_id TEXT NOT NULL,PRIMARY KEY(domain_id,request_id)) STRICT"),
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
    Authority(crate::store::orchestration::OrchestrationError),
    Codec(crate::store::session_transport::codex_rpc::RpcError),
    InvalidEvidence(String),
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

fn vendor_nonempty(value: &str, name: &'static str) -> Result<(), InboxError> {
    // Codex 0.149 accepts any nonempty string for native question fields.
    // Preserve it exactly; product-owned identifiers use `valid_id` instead.
    if value.is_empty() { return Err(InboxError::Invalid(name)); }
    Ok(())
}

fn vendor_request_identity(value: &str) -> Result<(), InboxError> {
    // B passes the canonical JSON scalar, so numeric 44 and string "44"
    // remain different JSON-RPC request identities across the native journal.
    let parsed = Parser::parse(value).map_err(|_| InboxError::Invalid("vendor request"))?;
    if !matches!(&parsed, Json::Number(_) | Json::String(_)) || parsed.canonical() != value {
        return Err(InboxError::Invalid("vendor request"));
    }
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

fn legacy_schema() -> Vec<(String, String)> {
    let mut rows: Vec<_> = SCHEMA.iter().take(5)
        .map(|(name, sql)| (name.to_string(), sql.to_string())).collect();
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
    let expected = expected_schema();
    let legacy = legacy_schema();
    if observed == expected { return Ok(()); }
    if !observed.is_empty() && observed != legacy {
        return Err(InboxError::Invalid("schema drift"));
    }
    transact(connection, |connection| {
        reject_side_effect_objects(connection)?;
        let current = observed_schema(connection)?;
        if current == expected { return Ok(()); }
        if !current.is_empty() && current != legacy {
            return Err(InboxError::Invalid("schema race"));
        }
        let schemas = if current.is_empty() { SCHEMA.iter().collect::<Vec<_>>() }
            else { SCHEMA.iter().skip(5).collect::<Vec<_>>() };
        for (_, sql) in schemas { connection.execute(sql)?; }
        if observed_schema(connection)? != expected { return Err(InboxError::Invalid("schema mismatch")); }
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

pub(crate) fn read_message(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
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

pub(crate) fn current_revision(connection:&mut VerifiedDatabaseConnection<'_>,domain_id:&str,
    message_id:&str,authorized:impl FnOnce(&VerifiedDatabaseConnection<'_>)->Result<bool,InboxError>)
    ->Result<u64,InboxError> {
    transact(connection,|connection| {
        if !authorized(connection)? {return Err(InboxError::Denied);}
        Ok(read_message(connection,domain_id,message_id)?.map_or(0,|message|message.revision))
    })
}

pub(crate) fn read_operation(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
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
        // Preserve the original Host actor/body and causal request. Owner
        // cancellation stays available and is recorded as an actual User act.
        if current.as_ref().is_some_and(|message|message.sender_seat_id==host_rule::HOST_RULE_ACTOR)
            && !matches!(&edit,InboxEdit::Cancel) {return Err(InboxError::Denied);}
        let (state, next) = match edit {
            InboxEdit::Enqueue { sender_seat_id, seat_id, turn_id, generation, body } => {
                if current.is_some() { return Err(InboxError::Conflict); }
                if sender_seat_id==host_rule::HOST_RULE_ACTOR {return Err(InboxError::Denied);}
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
        if message.sender_seat_id==host_rule::HOST_RULE_ACTOR {return Err(InboxError::Denied);}
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
    transact(connection, |connection| abort_delivery_in_transaction(connection,envelope,proof,reason))
}

fn abort_delivery_in_transaction(connection: &VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>, proof: Option<&NativeAbortProof>, reason: &str)
    -> Result<StoredOperation, InboxError> {
        let prior = replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase != "PREPARED" { return Ok(prior); }
        if proof.is_some_and(|fact| fact.domain_id != envelope.domain_id ||
            fact.message_id != envelope.message_id || fact.request_id != envelope.request_id ||
            fact.receipt_id.is_empty()) { return Err(InboxError::Denied); }
        let confirmed = proof.is_some();
        let next = if confirmed { prior.revision } else { prior.revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))? };
        let state = if confirmed { "PENDING" } else { "UNKNOWN" };
        let phase = if !confirmed { "UNKNOWN" } else if reason == "DENIED" { "DENIED" } else { "CONFLICT" };
        let receipt=proof.map_or("",|proof|proof.receipt_id.as_str());
        update_delivery(connection,envelope,phase,state,"PREPARED",prior.revision,next,reason,receipt)?;
        read_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
}

/// Only H's same-transaction no-step and A terminal-turn fact can release a
/// prepared steer back to PENDING. A miss leaves the original C state intact.
pub(crate) fn abort_native_steer_if_ended(connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &crate::store::authority::OwnerIssuer,envelope:&InboxEnvelope<'_>,
    session_id:&str,step_id:&str,generation:&str,process_operation_id:&str,
    ticket:&str,nonce:&str,thread_id:&str,turn_id:&str) -> Result<Option<StoredOperation>,InboxError> {
    envelope.validate()?;
    transact(connection,|connection| {
        crate::store::authority::check_owner_in_current_transaction(connection,owner)
            .map_err(InboxError::Authority)?;
        let prior=replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,
            envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if !matches!(prior.phase.as_str(),"PREPARED"|"UNKNOWN") {return Ok(Some(prior));}
        let message=read_message(connection,envelope.domain_id,envelope.message_id)?.ok_or(InboxError::Conflict)?;
        if message.state!=prior.phase || message.revision!=prior.revision
            || message.generation!=generation || message.turn_id!=turn_id {
            return Err(InboxError::Conflict);
        }
        let fact=crate::store::session_transport::rpc_journal::
            confirm_turn_ended_without_step_in_transaction(connection,envelope.domain_id,
                session_id,&message.seat_id,generation,process_operation_id,ticket,nonce,
                step_id,thread_id,turn_id)
            .map_err(|error|InboxError::InvalidEvidence(format!("H confirmed abort: {error:?}")))?;
        let Some(fact)=fact else {return Ok(None);};
        let proof={
            let basis=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}",envelope.domain_id,session_id,
                step_id,process_operation_id,nonce,fact.source_epoch,fact.source_cursor);
            NativeAbortProof {domain_id:envelope.domain_id.into(),message_id:envelope.message_id.into(),
                request_id:envelope.request_id.into(),receipt_id:format!("h-inbox-abort-{}",
                    crate::store::digest::sha256_hex(basis.as_bytes()))}
        };
        if prior.phase=="PREPARED" {
            return abort_delivery_in_transaction(connection,envelope,Some(&proof),"TURN_ENDED")
                .map(Some);
        }
        // C fenced the physical call as UNKNOWN before H observed the ended
        // turn. No H step exists, so restore the queue at its current revision
        // without decreasing a published CAS value or dispatching again.
        update_delivery(connection,envelope,"CONFLICT","PENDING","UNKNOWN",prior.revision,
            prior.revision,"TURN_ENDED",&proof.receipt_id)?;
        read_operation(connection,envelope.domain_id,envelope.request_id)?
            .ok_or(InboxError::Unknown).map(Some)
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

/// Preserve the original H/pipe/timeout failure on the same C UNKNOWN intent.
/// A later proof may still resolve it, but an uncertain error is never FAILED.
pub(crate) fn record_delivery_unknown_error(connection:&mut VerifiedDatabaseConnection<'_>,
    owner:&crate::store::authority::OwnerIssuer,envelope:&InboxEnvelope<'_>,error:&str)
    ->Result<StoredOperation,InboxError> {
    envelope.validate()?;
    if error.is_empty() {return Err(InboxError::Invalid("original delivery error"));}
    transact(connection,|connection| {
        crate::store::authority::check_owner_in_current_transaction(connection,owner)
            .map_err(InboxError::Authority)?;
        let prior=replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,
            envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase!="UNKNOWN" || !prior.reason.is_empty() {return Ok(prior);}
        let message=read_message(connection,envelope.domain_id,envelope.message_id)?.ok_or(InboxError::Conflict)?;
        if message.state!="UNKNOWN" || message.revision!=prior.revision {return Err(InboxError::Conflict);}
        let update=Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_inbox_operations SET reason=?1 WHERE domain_id=?2
             AND request_id=?3 AND phase='UNKNOWN' AND reason=''")?;
        update.bind_text(1,error)?;update.bind_text(2,envelope.domain_id)?;
        update.bind_text(3,envelope.request_id)?;update.step_done()?;
        require_one_change(connection)?;
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
    transact(connection, |connection| settle_delivery_in_transaction(connection,envelope,proof))
}

fn settle_delivery_in_transaction(connection: &VerifiedDatabaseConnection<'_>,
    envelope: &InboxEnvelope<'_>, proof: &NativeDeliveryProof) -> Result<StoredOperation, InboxError> {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeDeliveryKind { Deliver, Steer }

fn confirmed_native_failure(frame: &[u8], kind: NativeDeliveryKind, thread_id:&str, turn_id: &str)
    -> Result<Option<String>, InboxError> {
    use crate::store::atomic::JsonString;
    let body=frame.strip_suffix(b"\n").ok_or(InboxError::Denied)?;
    let Json::Object(top)=Parser::parse(std::str::from_utf8(body).map_err(|error|
        InboxError::InvalidEvidence(format!("provider error UTF-8: {error}")))?)? else {
        return Err(InboxError::Denied);
    };
    let Some(Json::Object(error))=top.get(&JsonString::from_str("error")) else {
        return Err(InboxError::Denied);
    };
    let Some(Json::Number(code))=error.get(&JsonString::from_str("code")) else {
        return Err(InboxError::Denied);
    };
    let Some(Json::String(message))=error.get(&JsonString::from_str("message")) else {
        return Err(InboxError::Denied);
    };
    let message=message.to_well_formed_string().ok_or(InboxError::Denied)?;
    // Codex 0.149 app-server turn_steer_inner maps these fixed
    // SteerSubmission::NotSubmitted reasons to invalid_request (-32600).
    // Its failed-to-steer internal error may follow a partial submission and
    // must remain UNKNOWN. The inject-items writer also has uncertain errors.
    let mismatch_prefix=format!("expected active turn id `{turn_id}` but found `");
    let mismatch=message.strip_prefix(&mismatch_prefix).is_some_and(|actual|
        actual.ends_with('`') && actual.len()>1);
    let no_thread=message==format!("thread not found: {thread_id}");
    let not_submitted=code=="-32600" && (no_thread ||
        (kind==NativeDeliveryKind::Steer &&
            (message=="no active turn to steer" || mismatch ||
                message=="cannot steer a review turn" || message=="cannot steer a compact turn")));
    Ok(not_submitted.then_some(message))
}

fn unhex_native(value: &str) -> Result<Vec<u8>, InboxError> {
    if value.is_empty() || value.len() % 2 != 0 { return Err(InboxError::Denied); }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let high = (pair[0] as char).to_digit(16).ok_or(InboxError::Denied)?;
        let low = (pair[1] as char).to_digit(16).ok_or(InboxError::Denied)?;
        Ok(((high << 4) | low) as u8)
    }).collect()
}

/// The only completion factory for native inbox delivery. C reopens H's exact
/// OBSERVED RPC and A's original response, including the process identity,
/// then decodes that response against C's stored full body and turn binding.
/// This does not infer delivery from a pipe write or caller-supplied receipt.
pub(crate) fn settle_native_delivery_observed(connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &crate::store::authority::OwnerIssuer, envelope: &InboxEnvelope<'_>,
    session_id: &str, step_id: &str, kind: NativeDeliveryKind) -> Result<StoredOperation, InboxError> {
    use crate::store::atomic::JsonString;
    use crate::store::session_transport::{codex_rpc::{self, Command, Reply, RpcId}, decode_request};
    envelope.validate()?;
    if !valid_id(session_id) || !valid_id(step_id) { return Err(InboxError::Invalid("native inbox step")); }
    transact(connection, |connection| {
        crate::store::authority::check_owner_in_current_transaction(connection,owner)
            .map_err(InboxError::Authority)?;
        let prior=replay(connection,envelope.domain_id,envelope.request_id,envelope.message_id,
            envelope.request_bytes)?.ok_or(InboxError::Conflict)?;
        if prior.phase!="UNKNOWN" {
            return if matches!(prior.phase.as_str(),"APPLIED"|"FAILED") {Ok(prior)}
                else {Err(InboxError::Conflict)};
        }
        let request=decode_request(envelope.request_bytes)
            .map_err(|error|InboxError::InvalidEvidence(format!("original inbox wire: {error:?}")))?;
        let expected_kind=match kind {NativeDeliveryKind::Deliver=>"deliver",NativeDeliveryKind::Steer=>"steer"};
        if request.family!="K-INBOX" || request.operation!=expected_kind
            || request.domain_id!=envelope.domain_id || request.target_id!=envelope.message_id
            || request.request_id!=envelope.request_id || request.expected_revision!=envelope.expected_revision {
            return Err(InboxError::Denied);
        }
        let message=read_message(connection,envelope.domain_id,envelope.message_id)?.ok_or(InboxError::Conflict)?;
        if message.state!="UNKNOWN" || message.revision!=prior.revision {return Err(InboxError::Conflict);}
        let payload_text=|field:&str| -> Result<String,InboxError> {
            match request.payload.get(&JsonString::from_str(field)) {
                Some(Json::String(value))=>value.to_well_formed_string().ok_or(InboxError::Denied),
                _=>Err(InboxError::Denied),
            }
        };
        if payload_text("generation")?!=message.generation
            || (kind==NativeDeliveryKind::Steer && payload_text("turnId")?!=message.turn_id) {
            return Err(InboxError::Denied);
        }
        let source=Statement::prepare(connection.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes),s.process_operation_id,s.ticket,
                    s.custodian_nonce,s.open_request_id
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.gogoke_v37_h_process_episode ep ON ep.domain_id=s.domain_id
                 AND ep.session_id=s.session_id AND ep.generation=s.generation
                 AND ep.process_operation_id=s.process_operation_id
               LEFT JOIN main.gogoke_v37_h_claim h ON h.domain_id=ep.domain_id
                 AND h.session_id=ep.session_id AND h.generation=ep.generation
                 AND h.process_operation_id=ep.process_operation_id
               JOIN main.gogoke_coordination_process_custody c
                 ON c.operation_id=s.process_operation_id AND c.domain_id=s.domain_id
                 AND c.generation=s.generation AND c.ticket=s.ticket
                 AND c.custodian_nonce=s.custodian_nonce AND c.pid=s.pid
                 AND c.creation_time_100ns=s.creation_time AND c.image_path=s.image_path
                 AND c.binary_digest_sha256=s.binary_digest AND c.profile_id=s.profile_id
               JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=ep.binding_id
                 AND b.domain_id=ep.domain_id AND b.instance_id=ep.instance_id
                 AND b.kind='SESSION' AND b.owner_id=ep.session_id AND b.generation=ep.generation
               LEFT JOIN main.gogoke_v37_seats e ON e.domain_id=ep.domain_id
                 AND e.seat_id=ep.seat_id AND e.incarnation=ep.seat_incarnation
                 AND CAST(e.generation AS TEXT)=ep.generation AND e.instance_id=ep.instance_id
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id
                 AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.step_id=?3
                AND s.generation=?4 AND ep.seat_id=?5 AND s.phase='OBSERVED'
                AND s.requires_response=1 AND r.state='NO_EVENT'
                AND r.no_event_reason='CODEX_RPC_RESPONSE'
                AND ((ep.phase='ACTIVE' AND h.state='COMMITTED'
                    AND c.state IN ('ACTIVE','UNKNOWN') AND b.state='ACTIVE' AND e.state='BUSY')
                  OR (ep.phase='UNKNOWN' AND h.state='UNKNOWN' AND c.state='UNKNOWN')
                  OR (ep.phase='STOPPED' AND c.state='STOPPED'
                    AND ep.stop_fact_id=c.stop_proof_hash AND length(c.stop_proof_hash)>0))")?;
        for (index,value) in [envelope.domain_id,session_id,step_id,
            message.generation.as_str(),message.seat_id.as_str()].iter().enumerate() {
            source.bind_text((index+1) as i32,value)?;
        }
        if !source.step_row()? {return Err(InboxError::Denied);}
        let command_bytes=unhex_native(&source.column_text(0)?)?;
        let response_bytes=unhex_native(&source.column_text(1)?)?;
        let operation=source.column_text(2)?;let ticket=source.column_text(3)?;
        let nonce=source.column_text(4)?;let open_id=source.column_text(5)?;
        if source.step_row()? {return Err(InboxError::Conflict);}drop(source);
        let frame_text=std::str::from_utf8(command_bytes.strip_suffix(b"\n").ok_or(InboxError::Denied)?)
            .map_err(|error|InboxError::InvalidEvidence(format!("H command UTF-8: {error}")))?;
        let Json::Object(frame)=Parser::parse(frame_text)? else {return Err(InboxError::Denied);};
        let id=match frame.get(&JsonString::from_str("id")) {
            Some(Json::Number(value))=>value.parse::<u64>().map_err(|_|InboxError::Denied)?,
            _=>return Err(InboxError::Denied),
        };
        let rpc_id=RpcId::client(id).map_err(InboxError::Codec)?;
        let thread=crate::store::session_transport::rpc_journal::observed_thread_id(connection,
            envelope.domain_id,session_id,&operation,&message.generation,&open_id,&ticket,&nonce)
            .map_err(|error|InboxError::InvalidEvidence(format!("H observed thread: {error:?}")))?;
        let command=match kind {
            NativeDeliveryKind::Deliver=>Command::AppendWithoutTurn {thread_id:thread,text:message.body.clone()},
            NativeDeliveryKind::Steer=>Command::TurnSteer {thread_id:thread,
                expected_turn_id:message.turn_id.clone(),text:message.body.clone()},
        };
        if command.encode(Some(&rpc_id)).map_err(InboxError::Codec)?!=command_bytes {
            return Err(InboxError::Denied);
        }
        let response=codex_rpc::decode(&response_bytes,Some((&rpc_id,&command)))
            .map_err(InboxError::Codec)?;
        let receipt_basis=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}",envelope.domain_id,session_id,
            step_id,operation,nonce,raw_hex(&command_bytes),prior.request_hex);
        if let Reply::RemoteError {raw_frame,..}=&response {
            let Some(reason)=confirmed_native_failure(raw_frame,kind,match &command {
                Command::AppendWithoutTurn {thread_id,..}|Command::TurnSteer {thread_id,..}=>thread_id,
                _=>return Err(InboxError::Denied),
            },&message.turn_id)? else {
                return Err(InboxError::Denied);
            };
            let proof=NativeDeliveryProof {domain_id:envelope.domain_id.into(),
                message_id:envelope.message_id.into(),request_id:envelope.request_id.into(),
                outcome:NativeDeliveryOutcome::Failed {reason}};
            return settle_delivery_in_transaction(connection,envelope,&proof);
        }
        if !matches!(response,Reply::Ack {..}) {return Err(InboxError::Denied);}
        let proof=NativeDeliveryProof {domain_id:envelope.domain_id.into(),
            message_id:envelope.message_id.into(),request_id:envelope.request_id.into(),
            outcome:NativeDeliveryOutcome::Completed {native_receipt_id:format!("h-inbox-{}",
                crate::store::digest::sha256_hex(receipt_basis.as_bytes()))}};
        settle_delivery_in_transaction(connection,envelope,&proof)
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

/// The native adapter's question shape is persisted separately from the
/// existing own-card rows. This keeps Codex's observed labels and answer
/// encoding intact without changing the own-card contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeAnswerShape { Options, FreeText, OptionsOrFree }

impl NativeAnswerShape {
    fn as_str(self) -> &'static str {
        match self {
            Self::Options => "OPTIONS",
            Self::FreeText => "FREE_TEXT",
            Self::OptionsOrFree => "OPTIONS_OR_FREE",
        }
    }
}

pub(crate) struct NativeQuestionOption<'a> {
    pub(crate) id: &'a str,
    pub(crate) label: &'a str,
    pub(crate) description: &'a str,
}

#[derive(Clone, Copy)]
pub(crate) struct NativeQuestion<'a> {
    pub(crate) vendor_request_id: &'a str,
    pub(crate) vendor_thread_id: &'a str,
    pub(crate) vendor_item_id: &'a str,
    /// Empty means the vendor omitted its optional auto-resolution timeout.
    pub(crate) auto_resolution_ms: &'a str,
    /// Canonical complete vendor card payload. This remains authoritative for
    /// cards containing more than one question or vendor-specific fields.
    pub(crate) question_payload: &'a str,
    pub(crate) question_id: &'a str,
    pub(crate) header: &'a str,
    pub(crate) question: &'a str,
    pub(crate) answer_shape: NativeAnswerShape,
    pub(crate) options: &'a [NativeQuestionOption<'a>],
    pub(crate) seat_id: &'a str,
    pub(crate) turn_id: &'a str,
    pub(crate) generation: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeCardOption {
    pub(crate) id: String,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) ordinal: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeQuestionCard {
    pub(crate) domain_id: String,
    pub(crate) card_id: String,
    pub(crate) revision: u64,
    pub(crate) state: String,
    pub(crate) vendor_request_id: String,
    pub(crate) vendor_thread_id: String,
    pub(crate) vendor_item_id: String,
    pub(crate) auto_resolution_ms: String,
    pub(crate) question_payload: String,
    pub(crate) question_id: String,
    pub(crate) header: String,
    pub(crate) question: String,
    pub(crate) answer_shape: String,
    pub(crate) seat_id: String,
    pub(crate) turn_id: String,
    pub(crate) generation: String,
    pub(crate) answer_kind: String,
    pub(crate) answer: String,
    pub(crate) options: Vec<NativeCardOption>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeCardOperation {
    pub(crate) request_hex: String,
    pub(crate) card_id: String,
    pub(crate) phase: String,
    pub(crate) previous_revision: u64,
    pub(crate) revision: u64,
    pub(crate) vendor_request_id: String,
    pub(crate) seat_id: String,
    pub(crate) turn_id: String,
    pub(crate) generation: String,
    pub(crate) answer_kind: String,
    pub(crate) answer: String,
    pub(crate) reason: String,
    pub(crate) native_receipt_id: String,
}

pub(crate) enum NativeAnswer<'a> {
    Option(&'a str),
    Free(&'a str),
    /// Complete answer wire for a multi-question native card. H still owns
    /// protocol validation and receipt correlation.
    Wire(&'a str),
}

fn validate_native_question(question: &NativeQuestion<'_>) -> Result<(), InboxError> {
    vendor_request_identity(question.vendor_request_id)?;
    vendor_nonempty(question.vendor_thread_id,"vendor thread")?;
    vendor_nonempty(question.vendor_item_id,"vendor item")?;
    if !question.auto_resolution_ms.is_empty() &&
        !question.auto_resolution_ms.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(InboxError::Invalid("auto resolution"));
    }
    vendor_nonempty(question.question_payload,"question payload")?;
    let claude_payload=matches!(Parser::parse(question.question_payload),Ok(Json::Object(ref fields))
        if matches!(fields.get(&JsonString::from_str("provider")),
            Some(Json::String(provider)) if provider.to_well_formed_string().as_deref()==Some("claude")));
    for (value,name) in [(question.question_id,"question id"),
        (question.header,"question header"),(question.question,"question"),
        (question.turn_id,"turn")] { vendor_nonempty(value,name)?; }
    required(question.seat_id,"seat")?;
    required(question.generation,"generation")?;
    if question.answer_shape == NativeAnswerShape::Options && question.options.is_empty() {
        return Err(InboxError::Invalid("options"));
    }
    if question.answer_shape == NativeAnswerShape::FreeText && !question.options.is_empty() {
        return Err(InboxError::Invalid("free-text options"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for option in question.options {
        if !valid_id(option.id) || !ids.insert(option.id) { return Err(InboxError::Invalid("option id")); }
        vendor_nonempty(option.label,"option label")?;
        if claude_payload {
            if option.description.contains('\0') {return Err(InboxError::Invalid("option description"));}
        } else {vendor_nonempty(option.description,"option description")?;}
    }
    Ok(())
}

fn native_binding_matches(card: &NativeQuestionCard, vendor_request_id: &str,
    seat_id: &str, turn_id: &str, generation: &str) -> bool {
    card.vendor_request_id == vendor_request_id && card.seat_id == seat_id &&
        card.turn_id == turn_id && card.generation == generation
}

fn read_native_card(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
    card_id: &str) -> Result<Option<NativeQuestionCard>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT revision,state,vendor_request_id,vendor_thread_id,vendor_item_id,auto_resolution_ms,question_payload,question_id,question_header,question_text,answer_shape,seat_id,turn_id,generation,answer_kind,answer FROM main.gogoke_v37_qcard_native WHERE domain_id=? AND card_id=?")?;
    statement.bind_text(1,domain_id)?;
    statement.bind_text(2,card_id)?;
    if !statement.step_row()? { return Ok(None); }
    let mut card = NativeQuestionCard {
        domain_id: domain_id.to_owned(), card_id: card_id.to_owned(),
        revision: parse_revision(statement.column_text(0)?)?, state: statement.column_text(1)?,
        vendor_request_id: statement.column_text(2)?, vendor_thread_id: statement.column_text(3)?,
        vendor_item_id: statement.column_text(4)?, auto_resolution_ms: statement.column_text(5)?,
        question_payload: statement.column_text(6)?, question_id: statement.column_text(7)?,
        header: statement.column_text(8)?, question: statement.column_text(9)?,
        answer_shape: statement.column_text(10)?, seat_id: statement.column_text(11)?,
        turn_id: statement.column_text(12)?, generation: statement.column_text(13)?,
        answer_kind: statement.column_text(14)?, answer: statement.column_text(15)?, options: Vec::new(),
    };
    let options = Statement::prepare(connection.as_ptr(), "SELECT option_id,ordinal,label,description FROM main.gogoke_v37_qcard_native_options WHERE domain_id=? AND card_id=? ORDER BY CAST(ordinal AS INTEGER),option_id")?;
    options.bind_text(1,domain_id)?;
    options.bind_text(2,card_id)?;
    while options.step_row()? {
        card.options.push(NativeCardOption {
            id: options.column_text(0)?,
            ordinal: options.column_text(1)?.parse().map_err(|_| InboxError::Invalid("stored native ordinal"))?,
            label: options.column_text(2)?, description: options.column_text(3)?,
        });
    }
    Ok(Some(card))
}

fn read_native_operation(connection: &VerifiedDatabaseConnection<'_>, domain_id: &str,
    request_id: &str) -> Result<Option<NativeCardOperation>, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "SELECT request_hex,card_id,state,previous_revision,revision,vendor_request_id,seat_id,turn_id,generation,answer_kind,answer,reason,native_receipt_id FROM main.gogoke_v37_qcard_native_operations WHERE domain_id=? AND request_id=?")?;
    statement.bind_text(1,domain_id)?;
    statement.bind_text(2,request_id)?;
    if !statement.step_row()? { return Ok(None); }
    Ok(Some(NativeCardOperation {
        request_hex: statement.column_text(0)?, card_id: statement.column_text(1)?,
        phase: statement.column_text(2)?, previous_revision: parse_revision(statement.column_text(3)?)?,
        revision: parse_revision(statement.column_text(4)?)?, vendor_request_id: statement.column_text(5)?,
        seat_id: statement.column_text(6)?, turn_id: statement.column_text(7)?,
        generation: statement.column_text(8)?, answer_kind: statement.column_text(9)?,
        answer: statement.column_text(10)?, reason: statement.column_text(11)?,
        native_receipt_id: statement.column_text(12)?,
    }))
}

fn native_card_replay(connection: &VerifiedDatabaseConnection<'_>, envelope: &CardEnvelope<'_>)
    -> Result<Option<NativeCardOperation>, InboxError> {
    let prior = read_native_operation(connection,envelope.domain_id,envelope.request_id)?;
    if let Some(ref operation) = prior {
        if operation.request_hex != raw_hex(envelope.request_bytes) ||
            operation.card_id != envelope.card_id { return Err(InboxError::Conflict); }
    }
    Ok(prior)
}

fn save_native_operation(connection: &VerifiedDatabaseConnection<'_>, envelope: &CardEnvelope<'_>,
    card: &NativeQuestionCard, phase: &str, answer_kind: &str, answer: &str,
    reason: &str, native_receipt_id: &str) -> Result<NativeCardOperation, InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_qcard_native_operations(domain_id,request_id,request_hex,card_id,previous_revision,revision,state,vendor_request_id,seat_id,turn_id,generation,answer_kind,answer,reason,native_receipt_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")?;
    for (index,value) in [envelope.domain_id,envelope.request_id,&raw_hex(envelope.request_bytes),
        envelope.card_id,&envelope.expected_revision.to_string(),&card.revision.to_string(),phase,
        card.vendor_request_id.as_str(),card.seat_id.as_str(),card.turn_id.as_str(),
        card.generation.as_str(),answer_kind,answer,reason,native_receipt_id].iter().enumerate() {
        statement.bind_text((index + 1) as i32,value)?;
    }
    statement.step_done()?;
    read_native_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
}

pub(crate) fn query_native_card(connection: &mut VerifiedDatabaseConnection<'_>, domain_id: &str,
    card_id: &str,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<Option<NativeQuestionCard>, InboxError> {
    if !valid_id(domain_id) || !valid_id(card_id) { return Err(InboxError::Invalid("card query")); }
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        read_native_card(connection,domain_id,card_id)
    })
}

/// Persists an observed vendor-native card before any answer is sent. The
/// native adapter supplies the vendor request identity and the complete card;
/// this path never creates an own-card fallback.
pub(crate) fn raise_native_card(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, question: &NativeQuestion<'_>,
    authorized_native_card_present: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<NativeCardOperation, InboxError> {
    envelope.validate()?;
    validate_native_question(question)?;
    transact(connection, |connection| {
        if !authorized_native_card_present(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = native_card_replay(connection,envelope)? { return Ok(prior); }
        if envelope.expected_revision != 0 { return Err(InboxError::Stale); }
        // One vendor request in one native process generation has one card,
        // regardless of a caller-selected local card ID or changed metadata.
        let alias = Statement::prepare(connection.as_ptr(), "SELECT 1 FROM main.gogoke_v37_qcard_native WHERE domain_id=? AND seat_id=? AND generation=? AND vendor_request_id=? LIMIT 1")?;
        for (index,value) in [envelope.domain_id,question.seat_id,question.generation,
            question.vendor_request_id].iter().enumerate() {
            alias.bind_text((index + 1) as i32,value)?;
        }
        if alias.step_row()? { return Err(InboxError::Conflict); }
        if read_card(connection,envelope.domain_id,envelope.card_id)?.is_some() ||
            read_native_card(connection,envelope.domain_id,envelope.card_id)?.is_some() {
            return Err(InboxError::Conflict);
        }
        let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_qcard_native(domain_id,card_id,revision,state,vendor_request_id,vendor_thread_id,vendor_item_id,auto_resolution_ms,question_payload,question_id,question_header,question_text,answer_shape,seat_id,turn_id,generation,answer_kind,answer) VALUES(?,?,'1','OPEN',?,?,?,?,?,?,?,?,?,?,?,?,'NONE','')")?;
        for (index,value) in [envelope.domain_id,envelope.card_id,question.vendor_request_id,
            question.vendor_thread_id,question.vendor_item_id,question.auto_resolution_ms,
            question.question_payload,question.question_id,question.header,question.question,
            question.answer_shape.as_str(),question.seat_id,question.turn_id,question.generation].iter().enumerate() {
            statement.bind_text((index + 1) as i32,value)?;
        }
        statement.step_done()?;
        for (ordinal,option) in question.options.iter().enumerate() {
            let statement = Statement::prepare(connection.as_ptr(), "INSERT INTO main.gogoke_v37_qcard_native_options(domain_id,card_id,option_id,ordinal,label,description) VALUES(?,?,?,?,?,?)")?;
            statement.bind_text(1,envelope.domain_id)?;
            statement.bind_text(2,envelope.card_id)?;
            statement.bind_text(3,option.id)?;
            statement.bind_text(4,&ordinal.to_string())?;
            statement.bind_text(5,option.label)?;
            statement.bind_text(6,option.description)?;
            statement.step_done()?;
        }
        let card = read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Unknown)?;
        save_native_operation(connection,envelope,&card,"RAISED","NONE","","","")
    })
}

/// Records the answer intent and moves the card to ANSWER_UNKNOWN. H may send
/// only after this transaction commits; no receipt is inferred from the write.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NativeAnswerDisposition { New, Existing }

pub(crate) struct NativeAnswerIntent {
    pub(crate) operation: NativeCardOperation,
    pub(crate) disposition: NativeAnswerDisposition,
}

pub(crate) fn begin_native_answer(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, vendor_request_id: &str, seat_id: &str, turn_id: &str,
    generation: &str, answer: NativeAnswer<'_>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<NativeCardOperation, InboxError> {
    begin_native_answer_intent(connection,envelope,vendor_request_id,seat_id,turn_id,generation,answer,authorized)
        .map(|intent|intent.operation)
}

/// Only the transaction that creates the original intent grants one send.
/// Re-reading ANSWER_UNKNOWN cannot grant another external write.
pub(crate) fn begin_native_answer_intent(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, vendor_request_id: &str, seat_id: &str, turn_id: &str,
    generation: &str, answer: NativeAnswer<'_>,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<NativeAnswerIntent, InboxError> {
    envelope.validate()?;
    vendor_request_identity(vendor_request_id)?;
    required(seat_id,"seat")?;
    vendor_nonempty(turn_id,"turn")?;
    required(generation,"generation")?;
    let (answer_kind,answer_text) = match answer {
        NativeAnswer::Option(value) => ("OPTION",value),
        NativeAnswer::Free(value) => ("FREE",value),
        NativeAnswer::Wire(value) => ("WIRE",value),
    };
    if matches!(answer_kind,"FREE" | "WIRE") { vendor_nonempty(answer_text,"answer")?; }
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = native_card_replay(connection,envelope)? {
            return Ok(NativeAnswerIntent { operation:prior, disposition:NativeAnswerDisposition::Existing });
        }
        let card = read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Conflict)?;
        if card.revision != envelope.expected_revision { return Err(InboxError::Stale); }
        if card.state != "OPEN" || !native_binding_matches(&card,vendor_request_id,seat_id,turn_id,generation) {
            return Err(InboxError::Conflict);
        }
        match answer_kind {
            "OPTION" => {
                if !matches!(card.answer_shape.as_str(), "OPTIONS" | "OPTIONS_OR_FREE") { return Err(InboxError::Conflict); }
                let statement = Statement::prepare(connection.as_ptr(), "SELECT 1 FROM main.gogoke_v37_qcard_native_options WHERE domain_id=? AND card_id=? AND option_id=?")?;
                statement.bind_text(1,envelope.domain_id)?;
                statement.bind_text(2,envelope.card_id)?;
                statement.bind_text(3,answer_text)?;
                if !statement.step_row()? { return Err(InboxError::Conflict); }
            }
            "FREE" => {
                if !matches!(card.answer_shape.as_str(), "FREE_TEXT" | "OPTIONS_OR_FREE") { return Err(InboxError::Conflict); }
            }
            "WIRE" => {}
            _ => return Err(InboxError::Invalid("answer kind")),
        }
        let next = envelope.expected_revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
        let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_qcard_native SET state='ANSWER_UNKNOWN',answer_kind=?,answer=?,revision=? WHERE domain_id=? AND card_id=? AND revision=? AND state='OPEN'")?;
        for (index,value) in [answer_kind,answer_text,&next.to_string(),envelope.domain_id,envelope.card_id,
            &envelope.expected_revision.to_string()].iter().enumerate() {
            statement.bind_text((index + 1) as i32,value)?;
        }
        statement.step_done()?;
        require_one_change(connection)?;
        let changed = read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Unknown)?;
        let operation=save_native_operation(connection,envelope,&changed,"UNKNOWN",answer_kind,answer_text,"","")?;
        Ok(NativeAnswerIntent { operation, disposition:NativeAnswerDisposition::New })
    })
}

/// Expiry is local durable state and is accepted only while no answer intent
/// is in flight. An expired card can never accept an answer or be replayed as
/// a new send.
pub(crate) fn expire_native_card(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, vendor_request_id: &str, seat_id: &str, turn_id: &str,
    generation: &str,
    authorized: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<bool, InboxError>)
    -> Result<NativeCardOperation, InboxError> {
    envelope.validate()?;
    vendor_request_identity(vendor_request_id)?;
    required(seat_id,"seat")?;
    vendor_nonempty(turn_id,"turn")?;
    required(generation,"generation")?;
    transact(connection, |connection| {
        if !authorized(connection)? { return Err(InboxError::Denied); }
        if let Some(prior) = native_card_replay(connection,envelope)? { return Ok(prior); }
        let card = read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Conflict)?;
        if card.revision != envelope.expected_revision || card.state != "OPEN" ||
            !native_binding_matches(&card,vendor_request_id,seat_id,turn_id,generation) {
            return Err(InboxError::Conflict);
        }
        let next = envelope.expected_revision.checked_add(1).ok_or(InboxError::Invalid("revision overflow"))?;
        let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_qcard_native SET state='EXPIRED',answer_kind='NONE',answer='',revision=? WHERE domain_id=? AND card_id=? AND revision=? AND state='OPEN'")?;
        for (index,value) in [&next.to_string(),envelope.domain_id,envelope.card_id,
            &envelope.expected_revision.to_string()].iter().enumerate() {
            statement.bind_text((index + 1) as i32,value)?;
        }
        statement.step_done()?;
        require_one_change(connection)?;
        let changed = read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Unknown)?;
        save_native_operation(connection,envelope,&changed,"EXPIRED","NONE","","","")
    })
}

/// Private proof fields are built only from the adapter's durable exact
/// native-write receipt below. Text or a Node boolean cannot resolve
/// ANSWER_UNKNOWN.
pub(crate) struct NativeCardAnswerProof {
    domain_id: String,
    card_id: String,
    request_id: String,
    vendor_request_id: String,
    seat_id: String,
    turn_id: String,
    generation: String,
    outcome: NativeAnswerOutcome,
}

enum NativeAnswerOutcome {
    Completed { native_receipt_id: String },
    Failed { reason: String },
}

/// Resolves an answer intent only from a typed H/adapter fact. Until that
/// fact exists the durable card remains ANSWER_UNKNOWN across restart.
pub(crate) fn settle_native_answer(connection: &mut VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, proof: &NativeCardAnswerProof)
    -> Result<NativeCardOperation, InboxError> {
    transact(connection, |connection| settle_native_answer_in_transaction(connection,envelope,proof))
}

fn settle_native_answer_in_transaction(connection: &VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, proof: &NativeCardAnswerProof)
    -> Result<NativeCardOperation, InboxError> {
        let prior = native_card_replay(connection,envelope)?.ok_or(InboxError::Conflict)?;
        if prior.phase != "UNKNOWN" { return Ok(prior); }
        let card = read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Conflict)?;
        if card.state != "ANSWER_UNKNOWN" || proof.domain_id != envelope.domain_id ||
            proof.card_id != envelope.card_id || proof.request_id != envelope.request_id ||
            !native_binding_matches(&card,&proof.vendor_request_id,&proof.seat_id,&proof.turn_id,&proof.generation) {
            return Err(InboxError::Denied);
        }
        match &proof.outcome {
            NativeAnswerOutcome::Completed { native_receipt_id } => {
                required(native_receipt_id,"native receipt")?;
                let used = Statement::prepare(connection.as_ptr(), "SELECT 1 FROM main.gogoke_v37_qcard_native_operations WHERE domain_id=? AND native_receipt_id=? AND state='ANSWERED' LIMIT 1")?;
                used.bind_text(1,envelope.domain_id)?;
                used.bind_text(2,native_receipt_id)?;
                if used.step_row()? { return Err(InboxError::Conflict); }
                let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_qcard_native SET state='ANSWERED' WHERE domain_id=? AND card_id=? AND revision=? AND state='ANSWER_UNKNOWN'")?;
                for (index,value) in [envelope.domain_id,envelope.card_id,&card.revision.to_string()].iter().enumerate() {
                    statement.bind_text((index + 1) as i32,value)?;
                }
                statement.step_done()?;
                require_one_change(connection)?;
                update_native_answer_operation(connection,envelope,"ANSWERED","",native_receipt_id)?;
            }
            NativeAnswerOutcome::Failed { reason } => {
                required(reason,"failure reason")?;
                let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_qcard_native SET state='OPEN',answer_kind='NONE',answer='' WHERE domain_id=? AND card_id=? AND revision=? AND state='ANSWER_UNKNOWN'")?;
                for (index,value) in [envelope.domain_id,envelope.card_id,&card.revision.to_string()].iter().enumerate() {
                    statement.bind_text((index + 1) as i32,value)?;
                }
                statement.step_done()?;
                require_one_change(connection)?;
                update_native_answer_operation(connection,envelope,"FAILED",reason,"")?;
            }
        }
        read_native_operation(connection,envelope.domain_id,envelope.request_id)?.ok_or(InboxError::Unknown)
}

/// For Codex's no-ACK answer response, completion means the original exact
/// frame was written by H and its WRITTEN fact committed. It never means the
/// vendor consumed the answer. Only native Owner authority can build this
/// proof; caller strings, resolved notifications and uncertain writes cannot.
pub(crate) fn settle_native_answer_written(connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &crate::store::authority::OwnerIssuer, envelope: &CardEnvelope<'_>,
    session_id: &str, step_id: &str) -> Result<NativeCardOperation, InboxError> {
    use crate::store::session_transport::codex_rpc::{self,Reply};
    use crate::store::session_transport::provider_evidence::{claude_question,stream_json};
    envelope.validate()?;
    if !valid_id(session_id) || !valid_id(step_id) {return Err(InboxError::Invalid("native answer step"));}
    transact(connection,|connection| {
        crate::store::authority::check_owner_in_current_transaction(connection,owner)
            .map_err(InboxError::Authority)?;
        let prior=native_card_replay(connection,envelope)?.ok_or(InboxError::Conflict)?;
        if prior.phase!="UNKNOWN" {
            return if matches!(prior.phase.as_str(),"ANSWERED"|"FAILED") {Ok(prior)} else {Err(InboxError::Conflict)};
        }
        let card=read_native_card(connection,envelope.domain_id,envelope.card_id)?.ok_or(InboxError::Conflict)?;
        if card.state!="ANSWER_UNKNOWN" || card.revision!=prior.revision || prior.answer_kind!="WIRE" || card.answer_kind!="WIRE"
            || prior.answer!=card.answer || !native_binding_matches(&card,&prior.vendor_request_id,
                &prior.seat_id,&prior.turn_id,&prior.generation) {return Err(InboxError::Conflict);}
        let mut command_bytes=card.answer.as_bytes().to_vec();command_bytes.push(b'\n');
        let command_hex=raw_hex(&command_bytes);
        let step=Statement::prepare(connection.as_ptr(),
            "SELECT s.process_operation_id,s.custodian_nonce,s.open_request_id,s.ticket,i.driver_id
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.gogoke_v37_h_process_episode ep ON ep.domain_id=s.domain_id
                 AND ep.session_id=s.session_id AND ep.generation=s.generation
                 AND ep.process_operation_id=s.process_operation_id
               JOIN main.gogoke_v37_instances i ON i.instance_id=ep.instance_id
               LEFT JOIN main.gogoke_v37_h_claim h ON h.domain_id=ep.domain_id
                 AND h.session_id=ep.session_id AND h.generation=ep.generation
                 AND h.process_operation_id=ep.process_operation_id
               JOIN main.gogoke_coordination_process_custody c ON c.operation_id=s.process_operation_id
                 AND c.ticket=s.ticket AND c.custodian_nonce=s.custodian_nonce
                 AND c.generation=s.generation AND c.domain_id=s.domain_id
                 AND c.pid=s.pid AND c.creation_time_100ns=s.creation_time
                 AND c.image_path=s.image_path AND c.binary_digest_sha256=s.binary_digest
                 AND c.profile_id=s.profile_id
               JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=ep.binding_id
                 AND b.domain_id=ep.domain_id AND b.instance_id=ep.instance_id
                 AND b.kind='SESSION' AND b.owner_id=ep.session_id AND b.generation=ep.generation
               LEFT JOIN main.gogoke_v37_seats e ON e.domain_id=ep.domain_id
                 AND e.seat_id=ep.seat_id AND e.incarnation=ep.seat_incarnation
                 AND CAST(e.generation AS TEXT)=ep.generation AND e.instance_id=ep.instance_id
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.step_id=?3
                AND s.phase='WRITTEN' AND s.requires_response=0 AND s.command_hex=?4
                AND s.generation=?5 AND ep.seat_id=?6
                AND ((ep.phase='ACTIVE' AND h.state='COMMITTED'
                    AND c.state IN ('ACTIVE','UNKNOWN') AND b.state='ACTIVE' AND e.state='BUSY')
                  OR (ep.phase='UNKNOWN' AND h.state='UNKNOWN' AND c.state='UNKNOWN')
                  OR (ep.phase='STOPPED' AND c.state='STOPPED'
                    AND ep.stop_fact_id=c.stop_proof_hash AND length(c.stop_proof_hash)>0))")?;
        for (index,value) in [envelope.domain_id,session_id,step_id,command_hex.as_str(),card.generation.as_str(),card.seat_id.as_str()].iter().enumerate() {
            step.bind_text((index+1) as i32,value)?;
        }
        if !step.step_row()? {return Err(InboxError::Denied);}
        let operation=step.column_text(0)?;let nonce=step.column_text(1)?;let open_id=step.column_text(2)?;let ticket=step.column_text(3)?;let driver=step.column_text(4)?;
        if step.step_row()? {return Err(InboxError::Conflict);}drop(step);
        let decode_hex=|value:&str| -> Result<Vec<u8>,InboxError> {
            if value.len()%2!=0 {return Err(InboxError::Invalid("native answer source hex"));}
            value.as_bytes().chunks_exact(2).map(|pair| {
                let text=std::str::from_utf8(pair).map_err(|error|InboxError::InvalidEvidence(format!("native answer source hex UTF-8: {error}")))?;
                u8::from_str_radix(text,16).map_err(|error|InboxError::InvalidEvidence(format!("native answer source hex number: {error}")))
            }).collect()
        };
        // C stores only A's source descriptor and hash, not a second raw
        // provider history. Recover that exact source under the same custody.
        let raised=Statement::prepare(connection.as_ptr(),
            "SELECT request_hex FROM main.gogoke_v37_qcard_native_operations WHERE domain_id=?1 AND card_id=?2 AND state='RAISED'")?;
        raised.bind_text(1,envelope.domain_id)?;raised.bind_text(2,envelope.card_id)?;
        if !raised.step_row()? {return Err(InboxError::Denied);}
        let descriptor_bytes=decode_hex(&raised.column_text(0)?)?;
        if raised.step_row()? {return Err(InboxError::Conflict);}drop(raised);
        let descriptor_text=std::str::from_utf8(&descriptor_bytes).map_err(|error|InboxError::InvalidEvidence(format!("native card descriptor UTF-8: {error}")))?;
        let descriptor_json=Parser::parse(descriptor_text)?;
        if descriptor_json.canonical()!=descriptor_text {return Err(InboxError::Denied);}
        let Json::Object(descriptor)=descriptor_json else {return Err(InboxError::Denied);};
        if descriptor.len()!=4 {return Err(InboxError::Denied);}
        let source_field=|name:&str|->Result<String,InboxError> {
            match descriptor.get(&JsonString::from_str(name)) {
                Some(Json::String(value))=>value.to_well_formed_string().ok_or(InboxError::Denied),
                _=>Err(InboxError::Denied),
            }
        };
        if source_field("operationId")?!=operation || source_field("sourceEpoch")?!=nonce {return Err(InboxError::Denied);}
        let source_cursor=source_field("sourceCursor")?;let source_digest=source_field("frameSha256")?;
        let source=Statement::prepare(connection.as_ptr(),
            "SELECT hex(raw_bytes) FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2 AND source_cursor=?3 AND custodian_nonce=?2 AND domain_id=?4 AND session_id=?5 AND generation=?6 AND process_ticket=?7")?;
        for (index,value) in [operation.as_str(),nonce.as_str(),source_cursor.as_str(),envelope.domain_id,session_id,card.generation.as_str(),ticket.as_str()].iter().enumerate() {source.bind_text((index+1) as i32,value)?;}
        if !source.step_row()? {return Err(InboxError::Denied);}
        let source_hex=source.column_text(0)?;
        if source.step_row()? {return Err(InboxError::Conflict);}drop(source);
        let raw=decode_hex(&source_hex)?;
        if crate::store::digest::sha256_hex(&raw)!=source_digest {return Err(InboxError::Denied);}
        if driver=="claude" {
            let Some(question)=claude_question::decode(&raw).map_err(|error|
                InboxError::InvalidEvidence(format!("Claude A question: {error:?}")))? else {
                return Err(InboxError::Denied);
            };
            if Json::String(JsonString::from_str(question.request_id())).canonical()!=card.vendor_request_id ||
                question.tool_use_id()!=card.vendor_item_id ||
                question.display_payload()!=card.question_payload ||
                claude_question::validate_answer_wire(&question,&command_bytes).is_err() {
                return Err(InboxError::Denied);
            }
            let original_send=Statement::prepare(connection.as_ptr(),
                "SELECT request_hex FROM main.gogoke_v37_h_stdin_journal
                 WHERE domain_id=?1 AND session_id=?2 AND request_id=?3
                   AND process_operation_id=?4 AND ticket=?5 AND custodian_nonce=?6
                   AND generation=?7 AND operation='send'")?;
            for (index,value) in [envelope.domain_id,session_id,card.turn_id.as_str(),
                operation.as_str(),ticket.as_str(),nonce.as_str(),card.generation.as_str()].iter().enumerate() {
                original_send.bind_text((index+1) as i32,value)?;
            }
            if !original_send.step_row()? {return Err(InboxError::Denied);}
            let send_hex=original_send.column_text(0)?;
            if original_send.step_row()? {return Err(InboxError::Conflict);}
            drop(original_send);
            let send_bytes=decode_hex(&send_hex)?;
            let send=crate::store::session_transport::decode_request(&send_bytes)
                .map_err(|error|InboxError::InvalidEvidence(format!("Claude H send: {error:?}")))?;
            if send.request_id!=card.turn_id || send.operation!="send" ||
                send.domain_id!=envelope.domain_id || send.target_id!=session_id {
                return Err(InboxError::Denied);
            }
            let init=Statement::prepare(connection.as_ptr(),
                "SELECT source_cursor FROM main.v37_ledger_raw_source
                 WHERE operation_id=?1 AND source_epoch=?2 AND
                   CAST(source_cursor AS INTEGER)<CAST(?3 AS INTEGER)
                 ORDER BY CAST(source_cursor AS INTEGER)")?;
            init.bind_text(1,&operation)?;init.bind_text(2,&nonce)?;
            init.bind_text(3,&source_cursor)?;
            let mut saw_session=false;
            while init.step_row()? {
                let candidate=ledger::read_captured_raw_source(connection,&operation,&nonce,
                    &init.column_text(0)?)?.ok_or(InboxError::Denied)?;
                if candidate.domain_id!=envelope.domain_id || candidate.session_id!=session_id
                    || candidate.process_ticket!=ticket {return Err(InboxError::Denied);}
                if matches!(stream_json::decode_claude_line(&candidate.raw_bytes),
                    Ok(stream_json::ClaudeData::Init {session_id,..}) if session_id==card.vendor_thread_id) {
                    saw_session=true;
                }
            }
            if !saw_session {return Err(InboxError::Denied);}
        } else if driver=="codex" {
            let Reply::Question(question)=codex_rpc::decode(&raw,None).map_err(InboxError::Codec)? else {return Err(InboxError::Denied);};
            if question.thread_id!=card.vendor_thread_id || question.turn_id!=card.turn_id || question.item_id!=card.vendor_item_id {return Err(InboxError::Denied);}
            let Json::Object(frame)=Parser::parse(&card.answer)? else {return Err(InboxError::Invalid("native answer object"));};
            if frame.len()!=2 || frame.get(&JsonString::from_str("id")).map(Json::canonical).as_deref()!=Some(card.vendor_request_id.as_str()) {return Err(InboxError::Denied);}
            let Some(Json::Object(result))=frame.get(&JsonString::from_str("result")) else {return Err(InboxError::Invalid("native answer result"));};
            let Some(Json::Object(answers))=result.get(&JsonString::from_str("answers")) else {return Err(InboxError::Invalid("native answers"));};
            let mut answer_values=std::collections::BTreeMap::new();
            for (id,value) in answers {
                let Json::Object(values)=value else {return Err(InboxError::Invalid("native answer values"));};
                let Some(Json::Array(values))=values.get(&JsonString::from_str("answers")) else {return Err(InboxError::Invalid("native answer values"));};
                let mut texts=Vec::new();
                for value in values {let Json::String(text)=value else {return Err(InboxError::Invalid("native answer text"));};
                    texts.push(text.to_well_formed_string().ok_or(InboxError::Invalid("native answer text"))?);}
                answer_values.insert(id.to_well_formed_string().ok_or(InboxError::Invalid("native question ID"))?,texts);
            }
            let expected=question.answer(answer_values).map_err(InboxError::Codec)?.encode(None).map_err(InboxError::Codec)?;
            if expected!=command_bytes {return Err(InboxError::Denied);}
            let observed=crate::store::session_transport::rpc_journal::observed_thread_id(connection,
                envelope.domain_id,session_id,&operation,&card.generation,&open_id,&ticket,&nonce)
                .map_err(|error|InboxError::InvalidEvidence(format!("H observed thread: {error:?}")))?;
            if observed!=card.vendor_thread_id {return Err(InboxError::Denied);}
        } else {return Err(InboxError::Denied);}
        let receipt_basis=format!("{}\n{}\n{}\n{}\n{}\n{}\n{}",envelope.domain_id,session_id,step_id,operation,nonce,command_hex,prior.request_hex);
        let proof=NativeCardAnswerProof {domain_id:envelope.domain_id.into(),card_id:envelope.card_id.into(),
            request_id:envelope.request_id.into(),vendor_request_id:card.vendor_request_id,seat_id:card.seat_id,
            turn_id:card.turn_id,generation:card.generation,outcome:NativeAnswerOutcome::Completed {
                native_receipt_id:format!("h-qanswer-{}",crate::store::digest::sha256_hex(receipt_basis.as_bytes()))}};
        settle_native_answer_in_transaction(connection,envelope,&proof)
    })
}

fn update_native_answer_operation(connection: &VerifiedDatabaseConnection<'_>,
    envelope: &CardEnvelope<'_>, phase: &str, reason: &str, receipt: &str) -> Result<(), InboxError> {
    let statement = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_qcard_native_operations SET state=?,reason=?,native_receipt_id=? WHERE domain_id=? AND request_id=? AND state='UNKNOWN'")?;
    for (index,value) in [phase,reason,receipt,envelope.domain_id,envelope.request_id].iter().enumerate() {
        statement.bind_text((index + 1) as i32,value)?;
    }
    statement.step_done()?;
    require_one_change(connection)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fixed_steer_not_submitted_error_is_narrower_than_an_unknown_remote_error() {
        // Synthetic protocol controls, not a real Codex process/model run.
        let failed=b"{\"id\":4,\"error\":{\"code\":-32600,\"message\":\"no active turn to steer\"}}\n";
        assert_eq!(confirmed_native_failure(failed,NativeDeliveryKind::Steer,"threadA","turnA").unwrap(),
            Some("no active turn to steer".into()));
        assert_eq!(confirmed_native_failure(failed,NativeDeliveryKind::Deliver,"threadA","turnA").unwrap(),None);
        let ambiguous=b"{\"id\":4,\"error\":{\"code\":-32603,\"message\":\"failed to steer turn: transport uncertain\"}}\n";
        assert_eq!(confirmed_native_failure(ambiguous,NativeDeliveryKind::Steer,"threadA","turnA").unwrap(),None);
        let wrong_code=b"{\"id\":4,\"error\":{\"code\":-32603,\"message\":\"no active turn to steer\"}}\n";
        assert_eq!(confirmed_native_failure(wrong_code,NativeDeliveryKind::Steer,"threadA","turnA").unwrap(),None);
        let mismatch=b"{\"id\":4,\"error\":{\"code\":-32600,\"message\":\"expected active turn id `turnA` but found `turnB`\"}}\n";
        assert_eq!(confirmed_native_failure(mismatch,NativeDeliveryKind::Steer,"threadA","turnA").unwrap(),
            Some("expected active turn id `turnA` but found `turnB`".into()));
        assert_eq!(confirmed_native_failure(mismatch,NativeDeliveryKind::Steer,"threadA","turnC").unwrap(),None);
        let missing=b"{\"id\":4,\"error\":{\"code\":-32600,\"message\":\"thread not found: threadA\"}}\n";
        assert_eq!(confirmed_native_failure(missing,NativeDeliveryKind::Deliver,"threadA","turnA").unwrap(),
            Some("thread not found: threadA".into()));
    }

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

    #[test]
    fn legacy_own_card_rows_survive_native_schema_extension() {
        fixture(|connection| {
            for (_, sql) in SCHEMA.iter().take(5) {
                connection.execute(sql).unwrap();
            }
            connection.execute("INSERT INTO gogoke_v37_qcards(domain_id,card_id,revision,state,request_ref,seat_id,turn_id,generation,answer) VALUES('projectA','ownA','1','OPEN','questionA','seatA','turnA','1','')").unwrap();
            initialize_schema(connection).unwrap();
            let own = read_card(connection,"projectA","ownA").unwrap().unwrap();
            assert_eq!(own.request_ref,"questionA");
            let tables = Statement::prepare(connection.as_ptr(), "SELECT COUNT(*) FROM main.sqlite_schema WHERE name LIKE 'gogoke_v37_qcard_native%'").unwrap();
            assert!(tables.step_row().unwrap());
            assert_eq!(tables.column_text(0).unwrap(),"3");
        });
    }

    #[test]
    fn native_card_persists_complete_shape_and_keeps_answer_unknown() {
        fixture(|connection| {
            initialize_schema(connection).unwrap();
            let options = [
                NativeQuestionOption { id: "allow", label: "Allow", description: "Permit once" },
                NativeQuestionOption { id: "deny", label: "Deny", description: "Reject once" },
            ];
            let question = NativeQuestion {
                vendor_request_id: "44", vendor_thread_id: "threadA", vendor_item_id: "itemA",
                auto_resolution_ms: "", question_payload: "{\"questions\":[{\"id\":\"q\",\"header\":\"Choose\",\"question\":\"Which action?\",\"options\":[{\"label\":\"Allow\",\"description\":\"Permit once\"},{\"label\":\"Deny\",\"description\":\"Reject once\"}]},{\"id\":\"q2\",\"header\":\"Reason\",\"question\":\"Why?\",\"options\":null}]}",
                question_id: "q", header: "Choose", question: "Which action?",
                answer_shape: NativeAnswerShape::OptionsOrFree, options: &options,
                seat_id: "seatA", turn_id: "turnA", generation: "1",
            };
            let raise = CardEnvelope { domain_id: "projectA", card_id: "nativeA", request_id: "nativeRaiseA",
                request_bytes: b"native raise exact wire", expected_revision: 0 };
            let raised = raise_native_card(connection,&raise,&question,|_| Ok(true)).unwrap();
            assert_eq!(raised.phase,"RAISED");
            let observed = query_native_card(connection,"projectA","nativeA",|_| Ok(true)).unwrap().unwrap();
            assert_eq!(observed.vendor_request_id,"44");
            assert_eq!(observed.vendor_thread_id,"threadA");
            assert_eq!(observed.vendor_item_id,"itemA");
            assert!(observed.question_payload.contains("questions"));
            assert!(observed.question_payload.contains("q2"));
            assert_eq!(observed.question,"Which action?");
            assert_eq!(observed.answer_shape,"OPTIONS_OR_FREE");
            assert_eq!(observed.options[0].label,"Allow");
            assert_eq!(observed.options[1].description,"Reject once");

            let answer = CardEnvelope { domain_id: "projectA", card_id: "nativeA", request_id: "nativeAnswerA",
                request_bytes: b"native answer exact wire", expected_revision: 1 };
            let intent = begin_native_answer_intent(connection,&answer,"44","seatA","turnA","1",
                NativeAnswer::Option("allow"), |_| Ok(true)).unwrap();
            assert_eq!(intent.disposition,NativeAnswerDisposition::New,"only the original transaction grants a send");
            let unknown=intent.operation;
            assert_eq!(unknown.phase,"UNKNOWN");
            assert_eq!(unknown.answer_kind,"OPTION");
            assert_eq!(query_native_card(connection,"projectA","nativeA",|_| Ok(true)).unwrap().unwrap().state,"ANSWER_UNKNOWN");
            initialize_schema(connection).unwrap();
            assert_eq!(read_native_operation(connection,"projectA","nativeAnswerA").unwrap().unwrap().phase,"UNKNOWN");
            assert_eq!(begin_native_answer_intent(connection,&answer,"44","seatA","turnA","1",
                NativeAnswer::Option("allow"), |_| Ok(true)).unwrap().disposition,
                NativeAnswerDisposition::Existing,"recovered UNKNOWN never grants a second send");
            assert_eq!(begin_native_answer(connection,&answer,"44","seatA","turnA","1",
                NativeAnswer::Option("allow"), |_| Ok(true)).unwrap(),unknown);
            let another = CardEnvelope { request_id: "nativeAnswerB", request_bytes: b"second answer wire", expected_revision: 2, ..answer };
            assert!(matches!(begin_native_answer(connection,&another,"44","seatA","turnA","1",
                NativeAnswer::Free("do it"), |_| Ok(true)),Err(InboxError::Conflict)));
            let expire_unknown = CardEnvelope { request_id: "nativeExpireA", request_bytes: b"expire while unknown", expected_revision: 2, ..answer };
            assert!(matches!(expire_native_card(connection,&expire_unknown,"44","seatA","turnA","1",
                |_| Ok(true)),Err(InboxError::Conflict)));

            let raise_expired = CardEnvelope { card_id: "nativeB", request_id: "nativeRaiseB",
                request_bytes: b"native B raise", expected_revision: 0, ..raise };
            assert!(matches!(raise_native_card(connection,&raise_expired,&question,|_| Ok(true)),
                Err(InboxError::Conflict)), "a second card cannot answer the same vendor request");
            let string_identity = NativeQuestion { vendor_request_id: r#""44""#, ..question };
            let raise_string = CardEnvelope { card_id: "nativeString", request_id: "nativeRaiseString",
                request_bytes: b"native string request raise", expected_revision: 0, ..raise };
            raise_native_card(connection,&raise_string,&string_identity,|_| Ok(true)).unwrap();
            let question_b = NativeQuestion { vendor_request_id: "45", ..question };
            raise_native_card(connection,&raise_expired,&question_b,|_| Ok(true)).unwrap();
            let expire = CardEnvelope { card_id: "nativeB", request_id: "nativeExpireB",
                request_bytes: b"native B expire", expected_revision: 1, ..raise_expired };
            assert_eq!(expire_native_card(connection,&expire,"45","seatA","turnA","1",|_| Ok(true)).unwrap().phase,"EXPIRED");
            let answer_expired = CardEnvelope { card_id: "nativeB", request_id: "nativeAnswerC",
                request_bytes: b"native B answer", expected_revision: 2, ..raise_expired };
            assert!(matches!(begin_native_answer(connection,&answer_expired,"45","seatA","turnA","1",
                NativeAnswer::Option("allow"), |_| Ok(true)),Err(InboxError::Conflict)));

            let raise_wire = CardEnvelope { card_id: "nativeC", request_id: "nativeRaiseC",
                request_bytes: b"native C raise", expected_revision: 0, ..raise };
            let question_c = NativeQuestion { vendor_request_id: "46", ..question };
            raise_native_card(connection,&raise_wire,&question_c,|_| Ok(true)).unwrap();
            let wire_answer = CardEnvelope { card_id: "nativeC", request_id: "nativeAnswerC2",
                request_bytes: b"native C answer", expected_revision: 1, ..raise_wire };
            assert_eq!(begin_native_answer(connection,&wire_answer,"46","seatA","turnA","1",
                NativeAnswer::Wire("{\"answers\":{\"q\":{\"answers\":[\"allow\"]}}}"), |_| Ok(true)).unwrap().answer_kind,"WIRE");

            let unicode = NativeQuestion { vendor_request_id: "47", question_id: "问题.1",
                header: " 选择 ", question: " Why? ",
                question_payload: r#"{"questions":[{"id":"问题.1","header":" 选择 ","question":" Why? ","options":null}]}"#,
                answer_shape: NativeAnswerShape::FreeText, options: &[], ..question };
            let raise_unicode = CardEnvelope { card_id: "nativeD", request_id: "nativeRaiseD",
                request_bytes: b"native D raise", expected_revision: 0, ..raise };
            raise_native_card(connection,&raise_unicode,&unicode,|_| Ok(true)).unwrap();
            let observed = query_native_card(connection,"projectA","nativeD",|_| Ok(true)).unwrap().unwrap();
            assert_eq!(observed.question_id,"问题.1");
            assert_eq!(observed.header," 选择 ");
        });
    }

    #[test]
    fn native_answer_exact_written_fact_rejects_other_bytes_and_recovers_settle_failure() {
        use crate::store::authority;
        use crate::store::ledger;
        use crate::store::session_transport::{codex_rpc::{Command,RpcId},rpc_journal};
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-qcard-written-{}-{stamp}",std::process::id()));
        std::fs::create_dir(&path).unwrap();let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        let owner=authority::initialize_profile(&mut db,&root).unwrap();
        initialize_schema(&mut db).unwrap();rpc_journal::initialize_schema(&mut db).unwrap();
        db.execute("CREATE TABLE orchestration_events(sequence INTEGER PRIMARY KEY,event_id TEXT UNIQUE,stream_id TEXT,occurred_at TEXT,event_type TEXT,payload_json TEXT)").unwrap();
        ledger::initialize_schema(&mut db).unwrap();
        // These rows model the original native facts for the SQL proof and
        // failure controls. This is not an actual CLI/writer/model test.
        db.execute("CREATE TABLE gogoke_v37_h_claim(domain_id TEXT,session_id TEXT,generation TEXT,process_operation_id TEXT,state TEXT,stop_fact_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_seat_binding(domain_id TEXT,session_id TEXT,generation TEXT,seat_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT,request_id TEXT,session_id TEXT,generation TEXT,old_generation TEXT,raw_hex TEXT,previous_revision INTEGER,result_revision INTEGER,process_operation_id TEXT,binding_id TEXT,instance_id TEXT,home_id TEXT,seat_id TEXT,seat_incarnation TEXT,phase TEXT,stop_fact_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_generation(domain_id TEXT,session_id TEXT,generation TEXT,request_id TEXT,process_operation_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_owner_binding(binding_id TEXT,domain_id TEXT,instance_id TEXT,kind TEXT,owner_id TEXT,generation TEXT,state TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_seats(domain_id TEXT,seat_id TEXT,incarnation TEXT,generation TEXT,instance_id TEXT,state TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY,driver_id TEXT,version TEXT) STRICT").unwrap();
        authority::initialize_process_custody_schema(&mut db).unwrap();
        db.execute("INSERT INTO gogoke_v37_instances VALUES('instanceA','codex','0.160.0')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim VALUES('projectA','sessionA','1','operationA','COMMITTED',NULL)").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_seat_binding VALUES('projectA','sessionA','1','seatA')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_process_episode VALUES('projectA','openA','sessionA','1',NULL,'',1,2,'operationA','bindingA','instanceA','homeA','seatA','incarnationA','ACTIVE',NULL)").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_generation VALUES('projectA','sessionA','1','openA','operationA')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','projectA','instanceA','SESSION','sessionA','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO gogoke_v37_seats VALUES('projectA','seatA','incarnationA','1','instanceA','BUSY')").unwrap();
        db.execute("INSERT INTO gogoke_coordination_process_custody VALUES('operationA','ticketA','nonceA','42','99','fixture-image','fixture-digest','profileA','projectA','1','ACTIVE',NULL)").unwrap();
        let question_frame=b"{\"id\":44,\"method\":\"item/tool/requestUserInput\",\"params\":{\"threadId\":\"threadA\",\"turnId\":\"turnA\",\"itemId\":\"itemA\",\"questions\":[{\"id\":\"q\",\"header\":\"Choose\",\"question\":\"Which?\",\"isOther\":true,\"isSecret\":false,\"options\":null}]}}\n";
        let thread_frame=b"{\"id\":3,\"result\":{\"thread\":{\"id\":\"threadA\",\"cwd\":\"sealed-tree\"}}}\n";
        for (cursor,bytes) in [("1",thread_frame.as_slice()),("2",question_frame.as_slice())] {
            let raw=Statement::prepare(db.as_ptr(),"INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES('operationA','ticketA','nonceA','projectA','sessionA','1','nonceA',?1,?2,'PENDING')").unwrap();
            raw.bind_text(1,cursor).unwrap();raw.bind_blob(2,bytes).unwrap();raw.step_done().unwrap();
        }
        db.execute("UPDATE main.v37_ledger_raw_source SET state='NO_EVENT',no_event_reason='CODEX_RPC_RESPONSE' WHERE operation_id='operationA' AND source_cursor='1'").unwrap();
        let descriptor=Json::Object(std::collections::BTreeMap::from([
            (JsonString::from_str("operationId"),Json::String("operationA".into())),
            (JsonString::from_str("sourceEpoch"),Json::String("nonceA".into())),
            (JsonString::from_str("sourceCursor"),Json::String("2".into())),
            (JsonString::from_str("frameSha256"),Json::String(crate::store::digest::sha256_hex(question_frame).into())),
        ])).canonical();
        let Json::Object(question_json)=Parser::parse(std::str::from_utf8(question_frame).unwrap()).unwrap() else {panic!("question fixture object");};
        let payload=question_json.get(&JsonString::from_str("params")).unwrap().canonical();
        let question=NativeQuestion {vendor_request_id:"44",vendor_thread_id:"threadA",vendor_item_id:"itemA",auto_resolution_ms:"",
            question_payload:&payload,question_id:"q",header:"Choose",question:"Which?",answer_shape:NativeAnswerShape::FreeText,
            options:&[],seat_id:"seatA",turn_id:"turnA",generation:"1"};
        let raise=CardEnvelope {domain_id:"projectA",card_id:"cardA",request_id:"raiseA",request_bytes:descriptor.as_bytes(),expected_revision:0};
        raise_native_card(&mut db,&raise,&question,|_|Ok(true)).unwrap();
        let command=Command::QuestionAnswer {request_id:RpcId::Number(44),answers:std::collections::BTreeMap::from([("q".into(),vec!["exact answer".into()])])};
        let wire=command.encode(None).unwrap();let text=std::str::from_utf8(wire.strip_suffix(b"\n").unwrap()).unwrap();
        let answer=CardEnvelope {domain_id:"projectA",card_id:"cardA",request_id:"answerA",request_bytes:b"original exact User answer",expected_revision:1};
        let intent=begin_native_answer_intent(&mut db,&answer,"44","seatA","turnA","1",NativeAnswer::Wire(text),|_|Ok(true)).unwrap();
        assert_eq!(intent.disposition,NativeAnswerDisposition::New);
        let start=Command::ThreadStart {cwd:"sealed-tree".into(),model:"gpt-6-sol".into()}.encode(Some(&RpcId::Number(3))).unwrap();
        for (id,bytes,response,phase,cursor) in [("thread-start",start.as_slice(),1,"OBSERVED",Some("1")),("qanswerA",wire.as_slice(),0,"INTENT",None)] {
            let step=Statement::prepare(db.as_ptr(),"INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase,source_epoch,source_cursor) VALUES('projectA','sessionA','openA',?1,'operationA','ticketA','nonceA','42','99','fixture-image','fixture-digest','profileA','1',?2,?3,?4,?5,?6)").unwrap();
            step.bind_text(1,id).unwrap();step.bind_text(2,&raw_hex(bytes)).unwrap();step.bind_i64(3,response).unwrap();step.bind_text(4,phase).unwrap();
            if let Some(cursor)=cursor {step.bind_text(5,"nonceA").unwrap();step.bind_text(6,cursor).unwrap();}
            // Unbound SQLite parameters are NULL for the INTENT fixture.
            step.step_done().unwrap();
        }
        assert!(matches!(settle_native_answer_written(&mut db,&owner,&answer,"sessionA","qanswerA"),Err(InboxError::Denied)),"INTENT is not a completed native write");
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='WRITTEN',command_hex='7b7d0a' WHERE step_id='qanswerA'").unwrap();
        assert!(matches!(settle_native_answer_written(&mut db,&owner,&answer,"sessionA","qanswerA"),Err(InboxError::Denied)),"another exact command cannot settle this answer");
        let restore=Statement::prepare(db.as_ptr(),"UPDATE main.gogoke_v37_rpc_steps SET command_hex=?1 WHERE step_id='qanswerA'").unwrap();
        restore.bind_text(1,&raw_hex(&wire)).unwrap();restore.step_done().unwrap();drop(restore);
        db.execute("CREATE TRIGGER inject_answer_settle_failure BEFORE UPDATE ON gogoke_v37_qcard_native_operations WHEN NEW.state='ANSWERED' BEGIN SELECT RAISE(ABORT,'injected answer receipt write failure'); END").unwrap();
        assert!(settle_native_answer_written(&mut db,&owner,&answer,"sessionA","qanswerA").is_err());
        assert_eq!(query_native_card(&mut db,"projectA","cardA",|_|Ok(true)).unwrap().unwrap().state,"ANSWER_UNKNOWN");
        db.execute("DROP TRIGGER inject_answer_settle_failure").unwrap();
        db.close_checked().unwrap();
        let mut db=crate::store::same_open::open_existing(&root,&path.join("state.sqlite")).unwrap();
        // Execute the production reopen initializer, not a substitute state
        // assignment: live custody becomes UNKNOWN while WRITTEN is history.
        authority::initialize_process_custody_schema(&mut db).unwrap();
        let custody=Statement::prepare(db.as_ptr(),"SELECT state FROM main.gogoke_coordination_process_custody WHERE operation_id='operationA'").unwrap();
        assert!(custody.step_row().unwrap());assert_eq!(custody.column_text(0).unwrap(),"UNKNOWN");drop(custody);
        let completed=settle_native_answer_written(&mut db,&owner,&answer,"sessionA","qanswerA").unwrap();
        assert_eq!(completed.phase,"ANSWERED");assert!(completed.native_receipt_id.starts_with("h-qanswer-"));
        assert_eq!(settle_native_answer_written(&mut db,&owner,&answer,"sessionA","qanswerA").unwrap(),completed,
            "same stored receipt recovered without another writer invocation");
        db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
    }
}
