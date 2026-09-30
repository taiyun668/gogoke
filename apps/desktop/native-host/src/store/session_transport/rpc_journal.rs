//! Durable native H substep journal for fixed Codex 0.149 JSONL.
//!
//! An RPC ACK is not a K-SESSION receipt. The authoritative open claim,
//! Owner issuer, E seat, process custody, and A raw-source bytes are checked
//! on the same VerifiedDatabaseConnection. This table records only the
//! original command and source key; it does not copy provider output.

use super::codex_rpc::{self, Command, Reply, RpcId};
use crate::process::{OriginBoundFrame, PreparedCustody};
use crate::store::atomic::{AtomicError, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::ledger::{self, RawSourceKey};
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

const SCHEMA: &str = "CREATE TABLE gogoke_v37_rpc_steps(domain_id TEXT NOT NULL,session_id TEXT NOT NULL,open_request_id TEXT NOT NULL,step_id TEXT NOT NULL,process_operation_id TEXT NOT NULL,ticket TEXT NOT NULL,custodian_nonce TEXT NOT NULL,pid TEXT NOT NULL,creation_time TEXT NOT NULL,image_path TEXT NOT NULL,binary_digest TEXT NOT NULL,profile_id TEXT NOT NULL,generation TEXT NOT NULL,command_hex TEXT NOT NULL,requires_response INTEGER NOT NULL CHECK(requires_response IN (0,1)),phase TEXT NOT NULL CHECK(phase IN ('INTENT','WRITTEN','OBSERVED','UNKNOWN')),source_epoch TEXT,source_cursor TEXT,original_error TEXT,CHECK((phase IN ('INTENT','WRITTEN') AND source_epoch IS NULL AND source_cursor IS NULL AND original_error IS NULL) OR (phase='UNKNOWN' AND source_epoch IS NULL AND source_cursor IS NULL AND original_error IS NOT NULL AND length(original_error)>0) OR (phase='OBSERVED' AND source_epoch IS NOT NULL AND source_cursor IS NOT NULL AND original_error IS NULL)),PRIMARY KEY(domain_id,session_id,step_id)) STRICT";
const FAMILY: &str = "gogoke_v37_rpc_";

#[derive(Debug)]
pub(crate) enum RpcJournalError {
    Invalid(&'static str),
    Denied,
    Conflict,
    Unknown,
    Store(AtomicError),
    Open(SameOpenError),
    Authority(crate::store::orchestration::OrchestrationError),
    Codec(codex_rpc::RpcError),
    CommitUnknown(SameOpenError),
    RollbackUnknown {
        primary: Box<RpcJournalError>,
        rollback: SameOpenError,
    },
}
impl From<AtomicError> for RpcJournalError {
    fn from(error: AtomicError) -> Self {
        Self::Store(error)
    }
}
impl From<SameOpenError> for RpcJournalError {
    fn from(error: SameOpenError) -> Self {
        Self::Open(error)
    }
}
impl From<crate::store::orchestration::OrchestrationError> for RpcJournalError {
    fn from(error: crate::store::orchestration::OrchestrationError) -> Self {
        Self::Authority(error)
    }
}
impl From<codex_rpc::RpcError> for RpcJournalError {
    fn from(error: codex_rpc::RpcError) -> Self {
        Self::Codec(error)
    }
}
type Result<T> = std::result::Result<T, RpcJournalError>;
const RPC_RESPONSE_NO_EVENT: &str = "CODEX_RPC_RESPONSE";

fn atom(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
fn raw(value: &[u8]) -> bool {
    !value.is_empty() && value.len() <= 65_536
}
fn hex(value: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(value.len() * 2);
    for byte in value {
        result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char);
    }
    result
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    if value.len()%2!=0 {return Err(RpcJournalError::Invalid("raw source hex"));}
    value.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|_|RpcJournalError::Invalid("raw source hex"))?;
        u8::from_str_radix(text,16).map_err(|_|RpcJournalError::Invalid("raw source hex"))
    }).collect()
}
fn requires_response(command: &Command) -> bool {
    !matches!(
        command,
        Command::Initialized | Command::QuestionAnswer { .. }
    )
}

fn transact<T>(
    db: &mut VerifiedDatabaseConnection<'_>,
    run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T>,
) -> Result<T> {
    db.execute("BEGIN IMMEDIATE")?;
    match run(db) {
        Ok(value) => {
            db.execute("COMMIT")
                .map_err(RpcJournalError::CommitUnknown)?;
            Ok(value)
        }
        Err(primary) => match db.execute("ROLLBACK") {
            Ok(()) => Err(primary),
            Err(rollback) => Err(RpcJournalError::RollbackUnknown {
                primary: Box::new(primary),
                rollback,
            }),
        },
    }
}

fn observed_schema(db: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>> {
    let query = Statement::prepare(
        db.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,?1))=?2 ORDER BY name",
    )?;
    query.bind_i64(1, FAMILY.len() as i64)?;
    query.bind_text(2, FAMILY)?;
    let mut rows = Vec::new();
    while query.step_row()? {
        rows.push((query.column_text(0)?, query.column_text(1)?));
    }
    Ok(rows)
}
fn no_shadow(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(substr(name,1,?1))=?2 OR lower(substr(tbl_name,1,?1))=?2 LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(substr(tbl_name,1,?1))=?2 LIMIT 1",
    ] {
        let query=Statement::prepare(db.as_ptr(),sql)?;
        query.bind_i64(1,FAMILY.len() as i64)?;
        query.bind_text(2,FAMILY)?;
        if query.step_row()? {return Err(RpcJournalError::Denied);}
    }
    Ok(())
}

/// Accept only an absent family or the exact fixed schema. A partial family,
/// TEMP shadow, trigger, index, or changed DDL is never silently repaired.
pub(crate) fn initialize_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    no_shadow(db)?;
    let expected = vec![(format!("{FAMILY}steps"), SCHEMA.to_owned())];
    let prior = observed_schema(db)?;
    if prior == expected {
        return Ok(());
    }
    if !prior.is_empty() {
        return Err(RpcJournalError::Denied);
    }
    transact(db, |db| {
        no_shadow(db)?;
        if !observed_schema(db)?.is_empty() {
            return Err(RpcJournalError::Denied);
        }
        db.execute(SCHEMA)?;
        if observed_schema(db)? != expected {
            return Err(RpcJournalError::Denied);
        }
        Ok(())
    })
}

pub(crate) struct Step<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) open_request_id: &'a str,
    pub(crate) open_request_bytes: &'a [u8],
    pub(crate) step_id: &'a str,
    pub(crate) custody: &'a PreparedCustody,
    pub(crate) rpc_id: Option<&'a RpcId>,
    pub(crate) command: &'a Command,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    Intent,
    Written,
    Observed,
    Unknown,
}
impl Phase {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "INTENT" => Ok(Self::Intent),
            "WRITTEN" => Ok(Self::Written),
            "OBSERVED" => Ok(Self::Observed),
            "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(RpcJournalError::Unknown),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Disposition {
    NewWrite,
    Existing(Phase),
}

#[derive(Debug)]
pub(crate) struct PreparedStep {
    pub(crate) bytes: Vec<u8>,
    pub(crate) disposition: Disposition,
}

fn same_row(
    db: &VerifiedDatabaseConnection<'_>,
    step: &Step<'_>,
    operation: &str,
    encoded: &[u8],
) -> Result<Option<Phase>> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT phase,command_hex,open_request_id,ticket,custodian_nonce,process_operation_id FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2 AND step_id=?3")?;
    q.bind_text(1, step.domain_id)?;
    q.bind_text(2, step.session_id)?;
    q.bind_text(3, step.step_id)?;
    if !q.step_row()? {
        return Ok(None);
    }
    let phase = Phase::parse(&q.column_text(0)?)?;
    if q.column_text(1)? != hex(encoded)
        || q.column_text(2)? != step.open_request_id
        || q.column_text(3)? != step.custody.ticket.opaque()
        || q.column_text(4)? != step.custody.custodian_nonce
    {
        return Err(RpcJournalError::Conflict);
    }
    if q.column_text(5)? != operation || q.step_row()? {
        return Err(RpcJournalError::Unknown);
    }
    Ok(Some(phase))
}

fn has_unresolved(
    db: &VerifiedDatabaseConnection<'_>,
    domain: &str,
    session: &str,
    operation: &str,
) -> Result<bool> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1 AND session_id=?2
           AND process_operation_id=?3
           AND (phase IN ('INTENT','UNKNOWN') OR (phase='WRITTEN' AND requires_response=1)) LIMIT 1")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, session)?;
    q.bind_text(3, operation)?;
    Ok(q.step_row()?)
}

fn candidate_binding(db: &VerifiedDatabaseConnection<'_>, step: &Step<'_>,
    required_state: &[&str]) -> Result<Option<String>> {
    if !has_process_episode_schema(db)? {return Ok(None);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT e.process_operation_id,c.state,e.phase
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation
           JOIN main.gogoke_v37_h_claim a
             ON a.domain_id=e.domain_id AND a.session_id=e.session_id
             AND a.generation=e.old_generation AND a.state='STOPPED'
           JOIN main.gogoke_coordination_process_custody oldc
             ON oldc.operation_id=a.process_operation_id AND oldc.domain_id=a.domain_id
             AND oldc.generation=a.generation AND oldc.state='STOPPED'
             AND oldc.stop_proof_hash=a.stop_fact_id AND a.stop_fact_id IS NOT NULL
           JOIN main.gogoke_v37_h_seat_binding sb
             ON sb.domain_id=a.domain_id AND sb.session_id=a.session_id
             AND sb.generation=a.generation AND sb.seat_id=e.seat_id
             AND sb.seat_incarnation=e.seat_incarnation
           JOIN main.gogoke_v37_seats s
             ON s.domain_id=sb.domain_id AND s.seat_id=sb.seat_id
             AND s.incarnation=sb.seat_incarnation
             AND CAST(s.generation AS TEXT)=sb.generation AND s.state='BUSY'
             AND s.instance_id=a.instance_id
           JOIN main.gogoke_v37_h_owner_binding b
             ON b.binding_id=e.binding_id AND b.instance_id=e.instance_id
             AND b.domain_id=e.domain_id AND b.owner_id=e.session_id
             AND b.generation=e.generation AND b.kind='SESSION' AND b.state='ACTIVE'
           JOIN main.gogoke_v37_instance_homes h
             ON h.home_id=e.home_id AND h.instance_id=e.instance_id
             AND h.domain_id=e.domain_id AND h.owner_id=e.session_id
             AND h.generation=e.generation AND h.kind='SESSION' AND h.state='ACTIVE'
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
             AND i.driver_id='codex' AND i.version='0.149.0'
             AND i.install_state='INSTALLED' AND i.login_state='LOGGED_IN'
             AND i.program_digest=c.binary_digest_sha256
             AND i.program_digest=oldc.binary_digest_sha256
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.generation=?4 AND e.old_generation IS NOT NULL
            AND e.process_operation_id IS NOT NULL
            AND c.ticket=?5 AND c.custodian_nonce=?6 AND c.pid=?7
            AND c.creation_time_100ns=?8 AND c.image_path=?9
            AND c.binary_digest_sha256=?10 AND c.profile_id=?11
            AND e.instance_id=a.instance_id")?;
    let c=step.custody;
    let pid=c.identity.pid.to_string();
    let time=c.identity.creation_time_100ns.to_string();
    let image=c.identity.image_path.to_string_lossy().into_owned();
    for (index,value) in [step.domain_id,step.session_id,step.open_request_id,
        c.binding.generation.as_str(),c.ticket.opaque(),c.custodian_nonce.as_str(),
        pid.as_str(),time.as_str(),image.as_str(),c.binding.binary_digest_sha256.as_str(),
        c.binding.profile_id.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? { return Ok(None); }
    let operation=q.column_text(0)?;
    let state=q.column_text(1)?;
    let phase=q.column_text(2)?;
    let observation_of_unknown=phase=="UNKNOWN" && required_state.contains(&"UNKNOWN");
    if q.step_row()? || !required_state.contains(&state.as_str())
        || !(matches!(phase.as_str(),"PREPARED"|"ACTIVE") || observation_of_unknown) {
        return Err(RpcJournalError::Denied);
    }
    if !matches!(step.command,Command::Initialize { .. }|Command::Initialized
        |Command::ConfigRead { .. }|Command::ThreadResume { .. }) {
        return Err(RpcJournalError::Denied);
    }
    Ok(Some(operation))
}

fn has_process_episode_schema(db: &VerifiedDatabaseConnection<'_>) -> Result<bool> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.sqlite_schema WHERE type='table'
          AND name='gogoke_v37_h_process_episode'")?;
    Ok(q.step_row()?)
}

/// A native-only proof that a prepared C steer has no H writer step and that
/// A captured the exact terminal notification from its original process.
/// The caller must hold BEGIN IMMEDIATE through its C state transition.
#[derive(Debug)]
pub(crate) struct ConfirmedTurnEnd {
    pub(crate) source_epoch: String,
    pub(crate) source_cursor: String,
}

pub(crate) fn confirm_turn_ended_without_step_in_transaction(
    db: &VerifiedDatabaseConnection<'_>, domain: &str, session: &str,
    seat_id: &str, generation: &str, process_operation_id: &str, ticket: &str,
    custodian_nonce: &str, step_id: &str, thread_id: &str, turn_id: &str,
) -> Result<Option<ConfirmedTurnEnd>> {
    for (value,name) in [(domain,"domain"),(session,"session"),(seat_id,"seat"),
        (generation,"generation"),(process_operation_id,"process operation"),
        (ticket,"ticket"),(custodian_nonce,"nonce"),(step_id,"step"),
        (thread_id,"thread"),(turn_id,"turn")] {
        if !atom(value) {return Err(RpcJournalError::Invalid(name));}
    }
    let step=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_rpc_steps
          WHERE domain_id=?1 AND session_id=?2 AND step_id=?3 LIMIT 1")?;
    step.bind_text(1,domain)?;step.bind_text(2,session)?;step.bind_text(3,step_id)?;
    if step.step_row()? {return Ok(None);}
    let source=Statement::prepare(db.as_ptr(),
        "SELECT r.source_epoch,r.source_cursor,hex(r.raw_bytes)
           FROM main.v37_ledger_raw_source r
           JOIN main.gogoke_v37_h_process_episode e
             ON e.process_operation_id=r.operation_id AND e.domain_id=r.domain_id
             AND e.session_id=r.session_id AND e.generation=r.generation
             AND e.seat_id=?7 AND e.seat_incarnation IS NOT NULL
             AND length(e.seat_incarnation)>0
           JOIN main.gogoke_v37_h_generation g
             ON g.domain_id=e.domain_id AND g.session_id=e.session_id
             AND g.generation=e.generation AND g.request_id=e.request_id
             AND g.process_operation_id=e.process_operation_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=r.operation_id AND c.domain_id=r.domain_id
             AND c.generation=r.generation AND c.ticket=r.process_ticket
             AND c.custodian_nonce=r.custodian_nonce
           JOIN main.gogoke_v37_h_owner_binding b
             ON b.binding_id=e.binding_id AND b.instance_id=e.instance_id
             AND b.domain_id=e.domain_id AND b.kind='SESSION'
             AND b.owner_id=e.session_id AND b.generation=e.generation
           LEFT JOIN main.gogoke_v37_h_claim a
             ON a.process_operation_id=e.process_operation_id
             AND a.domain_id=e.domain_id AND a.session_id=e.session_id
             AND a.generation=e.generation
          WHERE r.domain_id=?1 AND r.session_id=?2 AND r.generation=?3
            AND r.operation_id=?4 AND r.process_ticket=?5
            AND r.custodian_nonce=?6 AND r.state='RESOLVED'
             AND ((e.phase='ACTIVE' AND c.state='ACTIVE'
                   AND a.state='COMMITTED' AND b.state='ACTIVE')
               OR (e.phase='STOPPED' AND c.state='STOPPED'
                   AND e.stop_fact_id IS NOT NULL
                   AND e.stop_fact_id=c.stop_proof_hash
                   AND b.state IN ('ACTIVE','REVOKED')))
          ORDER BY CAST(r.source_cursor AS INTEGER) DESC")?;
    for (index,value) in [domain,session,generation,process_operation_id,
        ticket,custodian_nonce,seat_id].iter().enumerate() {
        source.bind_text((index+1) as i32,value)?;
    }
    while source.step_row()? {
        let epoch=source.column_text(0)?;
        let cursor=source.column_text(1)?;
        let bytes=unhex(&source.column_text(2)?)?;
        if let Ok(Reply::TurnNotification {thread_id:found_thread,turn_id:found_turn,
            status,..})=codex_rpc::decode(&bytes,None) {
            if found_thread==thread_id && found_turn==turn_id
                && status!=codex_rpc::TurnStatus::InProgress {
                return Ok(Some(ConfirmedTurnEnd {source_epoch:epoch,source_cursor:cursor}));
            }
        }
    }
    Ok(None)
}

/// Reconstruct this generation's provider thread only from its original H
/// command and the exact A response of the same physical process. It works
/// after the current claim advances to another generation.
pub(crate) fn observed_thread_id(db: &VerifiedDatabaseConnection<'_>,
    domain: &str, session: &str, process_operation_id: &str,
    generation: &str, open_request_id: &str, ticket: &str,
    custodian_nonce: &str) -> Result<String> {
    for (value,name) in [(domain,"domain"),(session,"session"),
        (process_operation_id,"process operation"),(generation,"generation"),
        (open_request_id,"open request"),(ticket,"ticket"),
        (custodian_nonce,"nonce")] {
        if !atom(value) {return Err(RpcJournalError::Invalid(name));}
    }
    let step_id=if let Some(older)=generation_episode_old(db,domain,session,
        process_operation_id,generation,open_request_id)? {
        if older {format!("{process_operation_id}-thread-resume")}
        else {"thread-start".to_owned()}
    } else {return Err(RpcJournalError::Denied)};
    let q=Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes)
           FROM main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id
             AND r.generation=s.generation
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=s.process_operation_id AND c.domain_id=s.domain_id
             AND c.generation=s.generation AND c.ticket=s.ticket
             AND c.custodian_nonce=s.custodian_nonce
          WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
            AND s.generation=?4 AND s.open_request_id=?5 AND s.ticket=?6
            AND s.custodian_nonce=?7 AND s.step_id=?8 AND s.phase='OBSERVED'
            AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
    for (index,value) in [domain,session,process_operation_id,generation,
        open_request_id,ticket,custodian_nonce,step_id.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Err(RpcJournalError::Denied);}
    let command=unhex(&q.column_text(0)?)?;
    let response=unhex(&q.column_text(1)?)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    let thread=if step_id=="thread-start" {
        codex_rpc::decode_stored_thread_start(&command,&response)?
    } else {
        codex_rpc::decode_stored_thread_resume(&command,&response)?
    };
    Ok(thread)
}

fn generation_episode_old(db: &VerifiedDatabaseConnection<'_>, domain: &str,
    session: &str, operation: &str, generation: &str,
    request_id: &str) -> Result<Option<bool>> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT CASE WHEN e.old_generation IS NULL THEN '0' ELSE '1' END
           FROM main.gogoke_v37_h_generation g
           JOIN main.gogoke_v37_h_process_episode e
             ON e.domain_id=g.domain_id AND e.request_id=g.request_id
             AND e.session_id=g.session_id AND e.generation=g.generation
             AND e.process_operation_id=g.process_operation_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation
          WHERE g.domain_id=?1 AND g.session_id=?2 AND g.process_operation_id=?3
            AND g.generation=?4 AND g.request_id=?5
            AND ((e.phase='STOPPED' AND c.state='STOPPED'
                   AND e.stop_fact_id=c.stop_proof_hash AND e.stop_fact_id IS NOT NULL)
              OR (e.phase='ACTIVE' AND c.state IN ('ACTIVE','UNKNOWN')))")?;
    for (index,value) in [domain,session,operation,generation,request_id].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Ok(None);}
    let old=q.column_text(0)?=="1";
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    Ok(Some(old))
}

/// Complete only a command already WRITTEN whose original process bytes A
/// captured before H lost the response commit. It never writes native stdin.
pub(crate) fn reconcile_written_resume_from_a(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    domain: &str, session: &str, request_id: &str,
    raw_request: &[u8], operation: &str, generation: &str,
) -> Result<Option<String>> {
    for (value,name) in [(domain,"domain"),(session,"session"),
        (request_id,"request"),(operation,"operation"),(generation,"generation")] {
        if !atom(value) {return Err(RpcJournalError::Invalid(name));}
    }
    let step_id=format!("{operation}-thread-resume");
    transact(db,|db| {
        check_owner_in_current_transaction(db,owner)?;
        let step=Statement::prepare(db.as_ptr(),
            "SELECT s.command_hex,s.ticket,s.custodian_nonce
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.gogoke_v37_h_process_episode e
                 ON e.domain_id=s.domain_id AND e.session_id=s.session_id
                 AND e.generation=s.generation AND e.process_operation_id=s.process_operation_id
                 AND e.request_id=s.open_request_id
               JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
                 AND a.session_id=e.session_id AND a.generation=e.old_generation
                 AND a.state='STOPPED' AND a.stop_fact_id IS NOT NULL
               JOIN main.gogoke_coordination_process_custody oldc
                 ON oldc.operation_id=a.process_operation_id AND oldc.domain_id=a.domain_id
                 AND oldc.generation=a.generation AND oldc.state='STOPPED'
                 AND oldc.stop_proof_hash=a.stop_fact_id
               JOIN main.gogoke_coordination_process_custody c
                 ON c.operation_id=s.process_operation_id AND c.domain_id=s.domain_id
                 AND c.generation=s.generation AND c.ticket=s.ticket
                 AND c.custodian_nonce=s.custodian_nonce
                 AND c.state IN ('ACTIVE','UNKNOWN')
                 AND c.binary_digest_sha256=oldc.binary_digest_sha256
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.open_request_id=?3
                AND s.process_operation_id=?4 AND s.generation=?5 AND s.step_id=?6
                AND s.phase='WRITTEN' AND s.requires_response=1
                AND e.raw_hex=?7 AND e.phase IN ('PREPARED','UNKNOWN')")?;
        let original=hex(raw_request);
        for (index,value) in [domain,session,request_id,operation,generation,
            step_id.as_str(),original.as_str()].iter().enumerate() {
            step.bind_text((index+1) as i32,value)?;
        }
        if !step.step_row()? {return Ok(None);}
        let command=unhex(&step.column_text(0)?)?;
        let ticket=step.column_text(1)?;
        let nonce=step.column_text(2)?;
        if step.step_row()? {return Err(RpcJournalError::Conflict);}
        drop(step);
        let source=Statement::prepare(db.as_ptr(),
            "SELECT source_epoch,source_cursor,hex(raw_bytes)
               FROM main.v37_ledger_raw_source
              WHERE operation_id=?1 AND domain_id=?2 AND session_id=?3
                AND generation=?4 AND process_ticket=?5 AND custodian_nonce=?6
                AND state='PENDING' ORDER BY CAST(source_cursor AS INTEGER)")?;
        for (index,value) in [operation,domain,session,generation,ticket.as_str(),
            nonce.as_str()].iter().enumerate() {
            source.bind_text((index+1) as i32,value)?;
        }
        let mut matched:Option<(String,String,Option<String>)>=None;
        while source.step_row()? {
            let bytes=unhex(&source.column_text(2)?)?;
            let thread=match codex_rpc::decode_stored_thread_resume(&command,&bytes) {
                Ok(thread)=>Some(thread),
                Err(codex_rpc::RpcError::RemoteResponse(_))=>None,
                Err(_)=>continue,
            };
            if matched.is_some() {return Err(RpcJournalError::Conflict);}
            matched=Some((source.column_text(0)?,source.column_text(1)?,thread));
        }
        drop(source);
        let Some((epoch,cursor,thread))=matched else {return Ok(None)};
        let key=RawSourceKey {operation_id:operation.to_owned(),source_epoch:epoch,
            source_cursor:cursor};
        persist_observation_and_no_event(db,domain,session,&step_id,operation,&key)?;
        Ok(thread)
    })
}

fn assert_native_binding(
    db: &VerifiedDatabaseConnection<'_>,
    step: &Step<'_>,
    required_state: &[&str],
    allow_unknown_claim: bool,
) -> Result<String> {
    if let Some(operation)=candidate_binding(db,step,required_state)? {
        return Ok(operation);
    }
    let c = step.custody;
    if c.binding.domain_id != step.domain_id
        || !atom(&c.binding.generation)
        || !atom(&c.custodian_nonce)
        || !atom(c.ticket.opaque())
    {
        return Err(RpcJournalError::Denied);
    }
    let q = Statement::prepare(
        db.as_ptr(),
        "SELECT c.operation_id,c.state,a.state,b.state,s.state,s.incarnation,s.generation,
                c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,
                c.profile_id,c.domain_id,c.generation,c.ticket,c.custodian_nonce,
                a.generation,b.instance_id,s.instance_id
           FROM main.gogoke_v37_h_claim a
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
            AND c.generation=a.generation
           JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=a.binding_id
            AND b.instance_id=a.instance_id AND b.domain_id=a.domain_id
            AND b.kind='SESSION' AND b.owner_id=a.session_id
            AND b.generation=a.generation
           JOIN main.gogoke_v37_h_seat_binding sb
             ON sb.domain_id=a.domain_id AND sb.session_id=a.session_id
            AND sb.generation=a.generation
           JOIN main.gogoke_v37_seats s
             ON s.domain_id=sb.domain_id AND s.seat_id=sb.seat_id
            AND s.incarnation=sb.seat_incarnation
          WHERE a.domain_id=?1 AND a.session_id=?2",
    )?;
    q.bind_text(1, step.domain_id)?;
    q.bind_text(2, step.session_id)?;
    if !q.step_row()? {
        return Err(RpcJournalError::Denied);
    }
    let operation = q.column_text(0)?;
    let custody_state = q.column_text(1)?;
    let claim_state = q.column_text(2)?;
    let owner_state = q.column_text(3)?;
    let seat_state = q.column_text(4)?;
    let seat_incarnation = q.column_text(5)?;
    let seat_generation = q.column_text(6)?;
    let expected = [
        c.identity.pid.to_string(),
        c.identity.creation_time_100ns.to_string(),
        c.identity.image_path.to_string_lossy().into_owned(),
        c.binding.binary_digest_sha256.clone(),
        c.binding.profile_id.clone(),
        c.binding.domain_id.clone(),
        c.binding.generation.clone(),
        c.ticket.opaque().to_owned(),
        c.custodian_nonce.clone(),
    ];
    for (index, value) in expected.iter().enumerate() {
        if q.column_text((index + 7) as i32)? != *value {
            return Err(RpcJournalError::Denied);
        }
    }
    let claim_generation = q.column_text(16)?;
    let owner_instance = q.column_text(17)?;
    let seat_instance = q.column_text(18)?;
    if !(claim_state == "COMMITTED" || (allow_unknown_claim && claim_state == "UNKNOWN"))
        || owner_state != "ACTIVE"
        || seat_state != "BUSY"
        || !required_state.contains(&custody_state.as_str())
        || seat_incarnation.is_empty()
        || seat_generation != claim_generation
        || claim_generation != c.binding.generation
        || owner_instance != seat_instance
        || q.step_row()?
    {
        return Err(RpcJournalError::Denied);
    }
    Ok(operation)
}

fn original_open(db: &VerifiedDatabaseConnection<'_>, step: &Step<'_>) -> Result<()> {
    if !raw(step.open_request_bytes) {
        return Err(RpcJournalError::Invalid("open request bytes"));
    }
    if has_process_episode_schema(db)? {
    let candidate=Statement::prepare(db.as_ptr(),
        "SELECT raw_hex FROM main.gogoke_v37_h_process_episode
          WHERE domain_id=?1 AND session_id=?2 AND request_id=?3
            AND old_generation IS NOT NULL AND generation=?4")?;
    for (index,value) in [step.domain_id,step.session_id,step.open_request_id,
        step.custody.binding.generation.as_str()].iter().enumerate() {
        candidate.bind_text((index+1) as i32,value)?;
    }
    if candidate.step_row()? {
        if candidate.column_text(0)?!=hex(step.open_request_bytes) || candidate.step_row()? {
            return Err(RpcJournalError::Denied);
        }
        return Ok(());
    }
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT raw_hex,session_id,status FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 AND operation='open'")?;
    q.bind_text(1, step.domain_id)?;
    q.bind_text(2, step.open_request_id)?;
    if !q.step_row()?
        || q.column_text(0)? != hex(step.open_request_bytes)
        || q.column_text(1)? != step.session_id
        || !matches!(q.column_text(2)?.as_str(), "UNKNOWN" | "APPLIED")
        || q.step_row()?
    {
        return Err(RpcJournalError::Denied);
    }
    Ok(())
}

/// INTENT is persisted before H writes a single byte. Existing INTENT,
/// WRITTEN-with-response, or UNKNOWN rows prohibit another write. A matching
/// existing step is returned only as a readback phase, never a send permit.
pub(crate) fn prepare(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    step: &Step<'_>,
) -> Result<PreparedStep> {
    for (value, name) in [
        (step.domain_id, "domain"),
        (step.session_id, "session"),
        (step.open_request_id, "open request"),
        (step.step_id, "step"),
    ] {
        if !atom(value) {
            return Err(RpcJournalError::Invalid(name));
        }
    }
    let encoded = step.command.encode(step.rpc_id)?;
    transact(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, step)?;
        let operation = assert_native_binding(db, step, &["PREPARED", "ACTIVE"], false)?;
        if let Some(phase) = same_row(db, step, &operation, &encoded)? {
            return Ok(PreparedStep {
                bytes: encoded,
                disposition: Disposition::Existing(phase),
            });
        }
        if has_unresolved(db, step.domain_id, step.session_id, &operation)? {
            return Err(RpcJournalError::Unknown);
        }
        let q=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,'INTENT')")?;
        let c = step.custody;
        let pid = c.identity.pid.to_string();
        let creation_time = c.identity.creation_time_100ns.to_string();
        let image_path = c.identity.image_path.to_string_lossy().into_owned();
        let command_hex = hex(&encoded);
        for (index, value) in [
            step.domain_id,
            step.session_id,
            step.open_request_id,
            step.step_id,
            operation.as_str(),
            c.ticket.opaque(),
            c.custodian_nonce.as_str(),
            pid.as_str(),
            creation_time.as_str(),
            image_path.as_str(),
            c.binding.binary_digest_sha256.as_str(),
            c.binding.profile_id.as_str(),
            c.binding.generation.as_str(),
            command_hex.as_str(),
        ]
        .iter()
        .enumerate()
        {
            q.bind_text((index + 1) as i32, value)?;
        }
        q.bind_i64(
            15,
            if requires_response(step.command) {
                1
            } else {
                0
            },
        )?;
        q.step_done()?;
        Ok(PreparedStep {
            bytes: encoded,
            disposition: Disposition::NewWrite,
        })
    })
}

fn transition(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    step: &Step<'_>,
    next: Phase,
    error: Option<&str>,
) -> Result<()> {
    let encoded = step.command.encode(step.rpc_id)?;
    if next == Phase::Unknown && error.map_or(true, |value| value.is_empty() || value.len() > 4096)
    {
        return Err(RpcJournalError::Invalid("original error"));
    }
    transact(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, step)?;
        let operation = if next == Phase::Unknown {
            assert_native_binding(db, step, &["ACTIVE", "UNKNOWN"], true)?
        } else {
            assert_native_binding(db, step, &["ACTIVE"], false)?
        };
        let prior = same_row(db, step, &operation, &encoded)?;
        if !(next == Phase::Written && prior == Some(Phase::Intent)
            || next == Phase::Unknown && matches!(prior, Some(Phase::Intent | Phase::Written)))
        {
            return Err(RpcJournalError::Conflict);
        }
        if next == Phase::Written {
            let q=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_rpc_steps SET phase='WRITTEN' WHERE domain_id=?1 AND session_id=?2 AND step_id=?3 AND process_operation_id=?4 AND phase='INTENT'")?;
            for (index, value) in [
                step.domain_id,
                step.session_id,
                step.step_id,
                operation.as_str(),
            ]
            .iter()
            .enumerate()
            {
                q.bind_text((index + 1) as i32, value)?;
            }
            q.step_done()?;
        } else {
            let q=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_rpc_steps SET phase='UNKNOWN',original_error=?1 WHERE domain_id=?2 AND session_id=?3 AND step_id=?4 AND process_operation_id=?5 AND phase IN ('INTENT','WRITTEN')")?;
            q.bind_text(1, error.ok_or(RpcJournalError::Invalid("original error"))?)?;
            for (index, value) in [
                step.domain_id,
                step.session_id,
                step.step_id,
                operation.as_str(),
            ]
            .iter()
            .enumerate()
            {
                q.bind_text((index + 2) as i32, value)?;
            }
            q.step_done()?;
        }
        if changes(db)? != 1 {
            return Err(RpcJournalError::Conflict);
        }
        Ok(())
    })
}

/// Call only after the exact native persistent writer returned success.
/// `initialized` and question answers end at WRITTEN: neither has an ACK.
pub(crate) fn mark_written(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    step: &Step<'_>,
) -> Result<()> {
    transition(db, owner, step, Phase::Written, None)
}

/// Any uncertain write is terminal. Preserve the original OS/pipe error;
/// callers must not resend this or another step under this session claim.
pub(crate) fn mark_unknown(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    step: &Step<'_>,
    original_error: &str,
) -> Result<()> {
    transition(db, owner, step, Phase::Unknown, Some(original_error))
}

fn changes(db: &VerifiedDatabaseConnection<'_>) -> Result<i64> {
    let q = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !q.step_row()? {
        return Err(RpcJournalError::Unknown);
    }
    q.column_text(0)?
        .parse()
        .map_err(|_| RpcJournalError::Unknown)
}

fn source_matches(
    db: &VerifiedDatabaseConnection<'_>,
    frame: &OriginBoundFrame,
    key: &RawSourceKey,
    operation: &str,
    step: &Step<'_>,
) -> Result<()> {
    if key.operation_id != operation
        || key.source_epoch.is_empty()
        || key.source_epoch.len() > 4096
        || key.source_epoch.contains('\0')
        || key
            .source_cursor
            .parse::<u64>()
            .ok()
            .filter(|number| {
                *number > 0 && *number <= i64::MAX as u64 && number.to_string() == key.source_cursor
            })
            .is_none()
        || frame.custody() != step.custody
    {
        return Err(RpcJournalError::Denied);
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.v37_ledger_raw_source WHERE operation_id=?1 AND source_epoch=?2 AND source_cursor=?3 AND process_ticket=?4 AND custodian_nonce=?5 AND domain_id=?6 AND session_id=?7 AND generation=?8 AND raw_bytes=?9")?;
    for (index, value) in [
        operation,
        &key.source_epoch,
        &key.source_cursor,
        step.custody.ticket.opaque(),
        &step.custody.custodian_nonce,
        step.domain_id,
        step.session_id,
        &step.custody.binding.generation,
    ]
    .iter()
    .enumerate()
    {
        q.bind_text((index + 1) as i32, value)?;
    }
    q.bind_blob(9, frame.bytes())?;
    if !q.step_row()? || q.step_row()? {
        return Err(RpcJournalError::Denied);
    }
    Ok(())
}

fn persist_observation_and_no_event(
    db: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
    step_id: &str,
    operation: &str,
    key: &RawSourceKey,
) -> Result<()> {
    let q=Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_rpc_steps SET phase='OBSERVED',source_epoch=?1,source_cursor=?2 WHERE domain_id=?3 AND session_id=?4 AND step_id=?5 AND process_operation_id=?6 AND phase='WRITTEN' AND requires_response=1")?;
    for (index, value) in [
        key.source_epoch.as_str(),key.source_cursor.as_str(),domain_id,session_id,step_id,operation,
    ].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    q.step_done()?;
    if changes(db)? != 1 {return Err(RpcJournalError::Conflict);}
    // This ledger API owns no transaction. If its exact-source terminalization
    // fails, the caller's BEGIN IMMEDIATE rolls this OBSERVED update back too.
    ledger::resolve_raw_source_no_event(db,&key.operation_id,&key.source_epoch,
        &key.source_cursor,RPC_RESPONSE_NO_EVENT)?;
    Ok(())
}

/// A must capture the exact OriginBoundFrame first. The response's RPC
/// observation and A no-event terminalization commit as one native write.
/// Notifications do not advance the waiting request.
pub(crate) fn complete_response(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    step: &Step<'_>,
    frame: &OriginBoundFrame,
    key: &RawSourceKey,
) -> Result<Reply> {
    let rpc_id = step.rpc_id.ok_or(RpcJournalError::Invalid("response id"))?;
    let reply = codex_rpc::decode(frame.bytes(), Some((rpc_id, step.command)))?;
    if !matches!(
        reply,
        Reply::Initialized { .. }
            | Reply::MemoryOff { .. }
            | Reply::Thread { .. }
            | Reply::Turn { .. }
            | Reply::Ack { .. }
            | Reply::FeaturePage { .. }
            | Reply::RemoteError { .. }
    ) {
        return Err(RpcJournalError::Invalid("not a response"));
    }
    let encoded = step.command.encode(step.rpc_id)?;
    transact(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, step)?;
        let operation = assert_native_binding(db, step, &["ACTIVE","UNKNOWN"], false)?;
        source_matches(db, frame, key, &operation, step)?;
        if !matches!(
            same_row(db, step, &operation, &encoded)?,
            Some(Phase::Written)
        ) || !requires_response(step.command)
        {
            return Err(RpcJournalError::Conflict);
        }
        persist_observation_and_no_event(db,step.domain_id,step.session_id,step.step_id,
            &operation,key)
    })?;
    Ok(reply)
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ReconciledResponse {
    pub(crate) key: RawSourceKey,
    pub(crate) newly_resolved: bool,
}

/// Repair only the old split-commit window: a previously OBSERVED RPC step
/// whose exact A source still says PENDING. No process is contacted and no RPC
/// ID or command is resent. The native OwnerIssuer is checked on the same
/// verified database; A checks its recovery binding for a PENDING source.
pub(crate) fn reconcile_observed_no_event(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    domain_id: &str,
    session_id: &str,
    step_id: &str,
) -> Result<ReconciledResponse> {
    for (value,name) in [(domain_id,"domain"),(session_id,"session"),(step_id,"step")] {
        if !atom(value) {return Err(RpcJournalError::Invalid(name));}
    }
    transact(db,|db| {
        check_owner_in_current_transaction(db,owner)?;
        let q=Statement::prepare(db.as_ptr(),
            "SELECT s.process_operation_id,s.source_epoch,s.source_cursor,r.state,
                    COALESCE(r.no_event_reason,'')
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r
                 ON r.operation_id=s.process_operation_id
                AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                AND r.domain_id=s.domain_id AND r.session_id=s.session_id
                AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.step_id=?3
                AND s.phase='OBSERVED' AND s.requires_response=1")?;
        q.bind_text(1,domain_id)?;
        q.bind_text(2,session_id)?;
        q.bind_text(3,step_id)?;
        if !q.step_row()? {return Err(RpcJournalError::Denied);}
        let key=RawSourceKey {operation_id:q.column_text(0)?,
            source_epoch:q.column_text(1)?,source_cursor:q.column_text(2)?};
        let state=q.column_text(3)?;
        let reason=q.column_text(4)?;
        if q.step_row()? {return Err(RpcJournalError::Conflict);}
        let newly_resolved=match state.as_str() {
            "PENDING" if reason.is_empty() => true,
            "NO_EVENT" if reason==RPC_RESPONSE_NO_EVENT => false,
            _ => return Err(RpcJournalError::Conflict),
        };
        ledger::resolve_raw_source_no_event(db,&key.operation_id,&key.source_epoch,
            &key.source_cursor,RPC_RESPONSE_NO_EVENT)?;
        Ok(ReconciledResponse {key,newly_resolved})
    })
}

/// H can check a notification's A source without altering an in-flight RPC.
pub(crate) fn observe_event(
    db: &VerifiedDatabaseConnection<'_>,
    frame: &OriginBoundFrame,
    key: &RawSourceKey,
    step: &Step<'_>,
) -> Result<Reply> {
    let operation = assert_native_binding(db, step, &["ACTIVE"], false)?;
    source_matches(db, frame, key, &operation, step)?;
    let reply = codex_rpc::decode(frame.bytes(), None)?;
    if matches!(
        reply,
        Reply::Event { .. }
            | Reply::ServerRequest { .. }
            | Reply::Question(_)
            | Reply::TurnNotification { .. }
            | Reply::CompactionItem { .. }
    ) {
        Ok(reply)
    } else {
        Err(RpcJournalError::Invalid("not an event"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::authority;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scalar(db: &VerifiedDatabaseConnection<'_>, sql: &str) -> String {
        let query=Statement::prepare(db.as_ptr(),sql).unwrap();
        assert!(query.step_row().unwrap());
        let value=query.column_text(0).unwrap();
        assert!(!query.step_row().unwrap());
        value
    }

    #[test]
    fn stopped_generation_can_confirm_original_turn_end_without_writer_step() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-stopped-steer-abort-{}-{stamp}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        db.execute("CREATE TABLE orchestration_events(sequence INTEGER PRIMARY KEY,event_id TEXT UNIQUE,stream_id TEXT,occurred_at TEXT,event_type TEXT,payload_json TEXT)").unwrap();
        ledger::initialize_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        // This is a SQL control for H's historical predicate. It does not
        // claim an observed provider turn or exercise a model.
        db.execute("CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT,request_id TEXT,session_id TEXT,generation TEXT,process_operation_id TEXT,seat_id TEXT,seat_incarnation TEXT,binding_id TEXT,instance_id TEXT,phase TEXT,stop_fact_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_generation(domain_id TEXT,session_id TEXT,generation TEXT,request_id TEXT,process_operation_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT,domain_id TEXT,generation TEXT,ticket TEXT,custodian_nonce TEXT,state TEXT,stop_proof_hash TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_owner_binding(binding_id TEXT,instance_id TEXT,domain_id TEXT,kind TEXT,owner_id TEXT,generation TEXT,state TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_claim(domain_id TEXT,session_id TEXT,generation TEXT,process_operation_id TEXT,state TEXT) STRICT").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_process_episode VALUES('project','open1','session','1','old-process','seatA','incarnationA','bindingA','instanceA','STOPPED','proofA')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_generation VALUES('project','session','1','open1','old-process')").unwrap();
        db.execute("INSERT INTO gogoke_coordination_process_custody VALUES('old-process','project','1','ticketA','nonceA','STOPPED','proofA')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','project','SESSION','session','1','REVOKED')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim VALUES('project','session','2','new-process','COMMITTED')").unwrap();
        let terminal=b"{\"method\":\"turn/completed\",\"params\":{\"threadId\":\"threadA\",\"turn\":{\"id\":\"turnA\",\"status\":\"completed\"}}}\n";
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,resolved_event_id) VALUES('old-process','ticketA','nonceA','project','session','1','epochA','1',?1,'RESOLVED','terminal-event')").unwrap();
        insert.bind_blob(1,terminal).unwrap();insert.step_done().unwrap();drop(insert);
        db.execute("BEGIN IMMEDIATE").unwrap();
        let proof=confirm_turn_ended_without_step_in_transaction(&db,"project","session",
            "seatA","1","old-process","ticketA","nonceA","steer-original",
            "threadA","turnA").unwrap().unwrap();
        assert_eq!(proof.source_epoch,"epochA");
        assert_eq!(proof.source_cursor,"1");
        for (seat,ticket,turn) in [("otherSeat","ticketA","turnA"),
            ("seatA","otherTicket","turnA"),("seatA","ticketA","otherTurn")] {
            assert!(confirm_turn_ended_without_step_in_transaction(&db,"project","session",
                seat,"1","old-process",ticket,"nonceA","steer-original",
                "threadA",turn).unwrap().is_none());
        }
        db.execute("INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase) VALUES('project','session','open1','steer-original','old-process','ticketA','nonceA','1','2','image','digest','profile','1','7b7d0a',0,'INTENT')").unwrap();
        assert!(confirm_turn_ended_without_step_in_transaction(&db,"project","session",
            "seatA","1","old-process","ticketA","nonceA","steer-original",
            "threadA","turnA").unwrap().is_none(),"any old H step forbids confirmed abort");
        db.execute("DELETE FROM gogoke_v37_rpc_steps WHERE step_id='steer-original'").unwrap();
        db.execute("UPDATE gogoke_v37_h_process_episode SET stop_fact_id='different-proof' WHERE process_operation_id='old-process'").unwrap();
        assert!(confirm_turn_ended_without_step_in_transaction(&db,"project","session",
            "seatA","1","old-process","ticketA","nonceA","steer-original",
            "threadA","turnA").unwrap().is_none(),"a mismatched stop proof cannot authorize abort");
        db.execute("COMMIT").unwrap();
        db.close_checked().unwrap();drop(root);fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn no_event_sql_failure_rolls_back_observed_and_same_source_can_finish() {
        let _guard=route_b_test_guard();
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-rpc-atomic-{}-{stamp}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
        let owner=authority::initialize_profile(&mut db,&root).unwrap();
        db.execute("CREATE TABLE orchestration_events(sequence INTEGER PRIMARY KEY,event_id TEXT UNIQUE,stream_id TEXT,occurred_at TEXT,event_type TEXT,payload_json TEXT)").unwrap();
        ledger::initialize_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        db.execute("CREATE TABLE gogoke_v37_h_claim(domain_id TEXT,session_id TEXT,generation TEXT,process_operation_id TEXT,state TEXT,stop_fact_id TEXT) STRICT").unwrap();
        db.execute("CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT,ticket TEXT,custodian_nonce TEXT,domain_id TEXT,generation TEXT,state TEXT,stop_proof_hash TEXT) STRICT").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim VALUES('domain','session','1','operation','COMMITTED',NULL)").unwrap();
        db.execute("INSERT INTO gogoke_coordination_process_custody VALUES('operation','ticket','nonce','domain','1','ACTIVE',NULL)").unwrap();
        db.execute("INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase) VALUES('domain','session','open','step','operation','ticket','nonce','1','2','image','digest','profile','1','7b7d0a',1,'WRITTEN')").unwrap();
        db.execute("INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state) VALUES('operation','ticket','nonce','domain','session','1','epoch','1',X'7B226964223A312C22726573756C74223A7B7D7D0A','PENDING')").unwrap();
        db.execute("CREATE TRIGGER injected_no_event_failure BEFORE UPDATE ON v37_ledger_raw_source WHEN NEW.state='NO_EVENT' BEGIN SELECT RAISE(FAIL,'injected A write failure'); END").unwrap();
        let key=RawSourceKey {operation_id:"operation".into(),source_epoch:"epoch".into(),source_cursor:"1".into()};
        assert!(transact(&mut db,|db| persist_observation_and_no_event(db,"domain","session","step","operation",&key)).is_err());
        assert_eq!(scalar(&db,"SELECT phase FROM gogoke_v37_rpc_steps"),"WRITTEN");
        assert_eq!(scalar(&db,"SELECT source_epoch IS NULL FROM gogoke_v37_rpc_steps"),"1");
        assert_eq!(scalar(&db,"SELECT state FROM v37_ledger_raw_source"),"PENDING");
        db.execute("DROP TRIGGER injected_no_event_failure").unwrap();
        transact(&mut db,|db| persist_observation_and_no_event(db,"domain","session","step","operation",&key)).unwrap();
        assert_eq!(scalar(&db,"SELECT phase FROM gogoke_v37_rpc_steps"),"OBSERVED");
        assert_eq!(scalar(&db,"SELECT state || ':' || no_event_reason FROM v37_ledger_raw_source"),"NO_EVENT:CODEX_RPC_RESPONSE");
        // Simulate only the historical split-commit residue, with the exact
        // OBSERVED step and source key still durable. No frame is fabricated.
        db.execute("UPDATE v37_ledger_raw_source SET state='PENDING',no_event_reason=NULL WHERE operation_id='operation'").unwrap();
        let repaired=reconcile_observed_no_event(&mut db,&owner,"domain","session","step").unwrap();
        assert_eq!(repaired.key,key);
        assert!(repaired.newly_resolved);
        assert_eq!(scalar(&db,"SELECT state || ':' || no_event_reason FROM v37_ledger_raw_source"),"NO_EVENT:CODEX_RPC_RESPONSE");
        assert!(!reconcile_observed_no_event(&mut db,&owner,"domain","session","step").unwrap().newly_resolved);
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn schema_is_exact_and_refuses_temp_or_trigger() {
        let _guard = route_b_test_guard();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("gogoke-rpc-journal-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        initialize_schema(&mut db).unwrap();
        initialize_schema(&mut db).unwrap();
        db.execute("CREATE TRIGGER foreign_trigger AFTER INSERT ON gogoke_v37_rpc_steps BEGIN SELECT 1; END").unwrap();
        assert!(matches!(
            initialize_schema(&mut db),
            Err(RpcJournalError::Denied)
        ));
        db.execute("DROP TRIGGER foreign_trigger").unwrap();
        db.execute("CREATE TEMP TABLE gogoke_v37_rpc_steps(dummy TEXT)")
            .unwrap();
        assert!(matches!(
            initialize_schema(&mut db),
            Err(RpcJournalError::Denied)
        ));
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn pending_states_block_new_writes_and_noack_remains_written() {
        let _guard = route_b_test_guard();
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("gogoke-rpc-state-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,generation,command_hex,requires_response,phase) VALUES('domain','session','open','step','operation','ticket','nonce','1','2','image','digest','profile','3','7b7d0a',1,'INTENT')").unwrap();
        assert!(has_unresolved(&db, "domain", "session", "operation").unwrap());
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='WRITTEN',requires_response=0")
            .unwrap();
        assert!(!has_unresolved(&db, "domain", "session", "operation").unwrap());
        db.execute("UPDATE gogoke_v37_rpc_steps SET requires_response=1")
            .unwrap();
        assert!(has_unresolved(&db, "domain", "session", "operation").unwrap());
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='OBSERVED',source_epoch='epoch',source_cursor='1'").unwrap();
        assert!(!has_unresolved(&db, "domain", "session", "operation").unwrap());
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='UNKNOWN',source_epoch=NULL,source_cursor=NULL,original_error='pipe error'").unwrap();
        assert!(has_unresolved(&db, "domain", "session", "operation").unwrap());
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
        assert!(requires_response(&Command::Initialize {
            client_version: "0.1".into()
        }));
        assert!(!requires_response(&Command::Initialized));
        assert!(!requires_response(&Command::QuestionAnswer {
            request_id: RpcId::Number(1),
            answers: std::collections::BTreeMap::from([("q".into(), vec!["yes".into()])])
        }));
        assert_eq!(Phase::parse("INTENT").unwrap(), Phase::Intent);
        assert_eq!(Phase::parse("WRITTEN").unwrap(), Phase::Written);
        assert_eq!(Phase::parse("UNKNOWN").unwrap(), Phase::Unknown);
        assert!(Phase::parse("RECEIPTED").is_err());
    }
}
