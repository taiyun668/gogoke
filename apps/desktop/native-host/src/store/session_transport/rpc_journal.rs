//! Durable native H substep journal for fixed Codex 0.149 JSONL.
//!
//! An RPC ACK is not a K-SESSION receipt. The authoritative open claim,
//! Owner issuer, E seat, process custody, and A raw-source bytes are checked
//! on the same VerifiedDatabaseConnection. This table records only the
//! original command and source key; it does not copy provider output.

use super::codex_rpc::{self, Command, Reply, RpcId};
use super::provider_evidence::{acp, claude_question, commands, stream_json};
use crate::process::{OriginBoundFrame, PreparedCustody};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::ledger::{self, RawSourceKey, RawSourceState};
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
    AcpEncode(commands::EncodeError),
    AcpDecode { reason: String, raw_frame: Vec<u8> },
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
const ACP_RESPONSE_NO_EVENT: &str = "ACP_RPC_RESPONSE";
const CLAUDE_ACK_NO_EVENT: &str = "CLAUDE_STDIN_ACK";

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
            | Command::DynamicToolResponse { .. }
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

/// Native H supplies the same open and prepared custody as a Codex step.
/// The vendor is intentionally absent: it is read from the current H instance.
pub(crate) struct AcpStep<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) open_request_id: &'a str,
    pub(crate) open_request_bytes: &'a [u8],
    pub(crate) step_id: &'a str,
    pub(crate) custody: &'a PreparedCustody,
    pub(crate) rpc_id: Option<&'a acp::RpcId>,
    pub(crate) command: &'a commands::AcpCommand<'a>,
}

/// One original Claude control request or User input. The actual provider is
/// derived from H's current instance, never from this caller's label.
pub(crate) struct ClaudeStep<'a> {
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) open_request_id: &'a str,
    pub(crate) open_request_bytes: &'a [u8],
    pub(crate) step_id: &'a str,
    pub(crate) custody: &'a PreparedCustody,
    pub(crate) command: &'a commands::ClaudeCommand<'a>,
}

/// A Claude question reply is a no-ACK control_response on the original
/// process stdin. Its source is the captured AskUserQuestion request, and its
/// intent is the already committed C User answer operation.
pub(crate) struct ClaudeQuestionStep<'a> {
    pub(crate) domain_id:&'a str,
    pub(crate) session_id:&'a str,
    pub(crate) open_request_id:&'a str,
    pub(crate) open_request_bytes:&'a [u8],
    pub(crate) step_id:&'a str,
    pub(crate) custody:&'a PreparedCustody,
    pub(crate) card_id:&'a str,
    pub(crate) answer_request_id:&'a str,
    pub(crate) source:&'a RawSourceKey,
    pub(crate) wire:&'a [u8],
}
impl<'a> ClaudeQuestionStep<'a> {
    fn fields(&self)->StepFields<'a> {
        StepFields {domain_id:self.domain_id,session_id:self.session_id,
            open_request_id:self.open_request_id,open_request_bytes:self.open_request_bytes,
            step_id:self.step_id,custody:self.custody}
    }
}

pub(crate) fn claude_initialize_identity(open_request_bytes: &[u8]) -> (String, String) {
    let digest = crate::store::digest::sha256_hex(open_request_bytes);
    (format!("claude-init-{}", &digest[..40]),
        format!("gogoke-claude-init-{}", &digest[..40]))
}

#[derive(Clone, Copy)]
struct StepFields<'a> {
    domain_id: &'a str,
    session_id: &'a str,
    open_request_id: &'a str,
    open_request_bytes: &'a [u8],
    step_id: &'a str,
    custody: &'a PreparedCustody,
}
impl<'a> Step<'a> {
    fn fields(&self) -> StepFields<'a> {
        StepFields { domain_id: self.domain_id, session_id: self.session_id,
            open_request_id: self.open_request_id, open_request_bytes: self.open_request_bytes,
            step_id: self.step_id, custody: self.custody }
    }
}
impl<'a> AcpStep<'a> {
    fn fields(&self) -> StepFields<'a> {
        StepFields { domain_id: self.domain_id, session_id: self.session_id,
            open_request_id: self.open_request_id, open_request_bytes: self.open_request_bytes,
            step_id: self.step_id, custody: self.custody }
    }
}
impl<'a> ClaudeStep<'a> {
    fn fields(&self) -> StepFields<'a> {
        StepFields { domain_id: self.domain_id, session_id: self.session_id,
            open_request_id: self.open_request_id, open_request_bytes: self.open_request_bytes,
            step_id: self.step_id, custody: self.custody }
    }
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
    step: &StepFields<'_>,
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

fn candidate_binding_fields(db: &VerifiedDatabaseConnection<'_>,
    fields: &StepFields<'_>, required_state: &[&str]) -> Result<Option<(String,String)>> {
    if !has_process_episode_schema(db)? {return Ok(None);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT e.process_operation_id,c.state,e.phase,i.driver_id,i.version
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
             AND i.login_state='LOGGED_IN'
             AND i.program_digest=c.binary_digest_sha256
             AND i.program_digest=oldc.binary_digest_sha256
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.generation=?4 AND e.old_generation IS NOT NULL
            AND e.process_operation_id IS NOT NULL
            AND c.ticket=?5 AND c.custodian_nonce=?6 AND c.pid=?7
            AND c.creation_time_100ns=?8 AND c.image_path=?9
            AND c.binary_digest_sha256=?10 AND c.profile_id=?11
            AND e.instance_id=a.instance_id")?;
    let c=fields.custody;
    let pid=c.identity.pid.to_string();
    let time=c.identity.creation_time_100ns.to_string();
    let image=c.identity.image_path.to_string_lossy().into_owned();
    for (index,value) in [fields.domain_id,fields.session_id,fields.open_request_id,
        c.binding.generation.as_str(),c.ticket.opaque(),c.custodian_nonce.as_str(),
        pid.as_str(),time.as_str(),image.as_str(),c.binding.binary_digest_sha256.as_str(),
        c.binding.profile_id.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? { return Ok(None); }
    let operation=q.column_text(0)?;
    let state=q.column_text(1)?;
    let phase=q.column_text(2)?;
    let driver=q.column_text(3)?;
    let version=q.column_text(4)?;
    let observation_of_unknown=phase=="UNKNOWN" && required_state.contains(&"UNKNOWN");
    if q.step_row()? || !required_state.contains(&state.as_str())
        || !(matches!(phase.as_str(),"PREPARED"|"ACTIVE") || observation_of_unknown)
        || !matches!((driver.as_str(),version.as_str()),
            ("codex","0.160.0") | ("opencode","1.18.32") | ("grok","1.0.41")) {
        return Err(RpcJournalError::Denied);
    }
    Ok(Some((operation,driver)))
}

fn candidate_binding(db: &VerifiedDatabaseConnection<'_>, step: &Step<'_>,
    required_state: &[&str]) -> Result<Option<String>> {
    let Some((operation,driver))=candidate_binding_fields(db,&step.fields(),required_state)?
        else {return Ok(None)};
    if driver!="codex" {return Err(RpcJournalError::Denied);}
    if !matches!(step.command,Command::Initialize { .. }
        |Command::InitializeHostTools { .. }|Command::Initialized
        |Command::ConfigRead { .. }|Command::ThreadResume { .. }) {
        return Err(RpcJournalError::Denied);
    }
    if let Some(change)=super::generation_change::active_for_session(db,
        step.domain_id,step.session_id)? {
        if change.request_id!=step.open_request_id || change.stage!="OLD_STOPPED"
            || change.owner_stop_request_id.is_some()
            || change.raw_hex!=hex(step.open_request_bytes)
            || change.old_generation==step.custody.binding.generation {
            return Err(RpcJournalError::Denied);
        }
        let old=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_process_episode e
               JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
                 AND a.session_id=e.session_id AND a.generation=e.old_generation
                 AND a.process_operation_id=?5 AND a.state='STOPPED'
               JOIN main.gogoke_coordination_process_custody c
                 ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
                 AND c.generation=a.generation AND c.state='STOPPED'
                 AND c.stop_proof_hash=a.stop_fact_id
              WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
                AND e.process_operation_id=?4 AND e.old_generation=?6
                AND e.seat_id=?7 AND c.ticket=?8 AND c.custodian_nonce=?9")?;
        for (index,value) in [step.domain_id,step.session_id,step.open_request_id,
            operation.as_str(),change.old_operation.as_str(),
            change.old_generation.as_str(),change.seat_id.as_str(),
            change.old_ticket.as_str(),change.old_nonce.as_str()]
            .iter().enumerate() {old.bind_text((index+1) as i32,value)?;}
        if !old.step_row()? || old.step_row()? {return Err(RpcJournalError::Denied);}
    } else if !plain_protocol_resume_episode(db,step.domain_id,step.session_id,
        step.open_request_id,&operation,&step.custody.binding.generation,
        step.open_request_bytes)? {return Err(RpcJournalError::Denied);}
    if let Command::ThreadResume {thread_id,..}=step.command {
        if observed_old_acp_session_id(db,step.domain_id,step.session_id,&operation,
            &step.custody.binding.generation,step.open_request_id)?.as_str()!=thread_id.as_str() {
            return Err(RpcJournalError::Denied);
        }
    }
    Ok(Some(operation))
}

fn has_process_episode_schema(db: &VerifiedDatabaseConnection<'_>) -> Result<bool> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.sqlite_schema WHERE type='table'
          AND name='gogoke_v37_h_process_episode'")?;
    Ok(q.step_row()?)
}

/// A public K-SESSION resume has an original episode but no compact/renew
/// generation-change row. Reconstruct its authority only from that original
/// request, stopped old physical generation, and held candidate identity.
fn plain_protocol_resume_episode(db: &VerifiedDatabaseConnection<'_>, domain: &str,
    session: &str, request_id: &str, operation: &str, generation: &str,
    raw_request: &[u8]) -> Result<bool> {
    let request=super::decode_request(raw_request).map_err(|_|RpcJournalError::Denied)?;
    if request.family!="K-SESSION" || request.operation!="resume"
        || request.domain_id!=domain || request.target_id!=session
        || request.request_id!=request_id || request.raw_bytes.as_slice()!=raw_request {
        return Ok(false);
    }
    if super::generation_change::active_for_session(db,domain,session)?.is_some()
        || super::generation_change::read(db,domain,request_id)?.is_some() {
        return Ok(false);
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT e.old_generation,e.previous_revision,COALESCE(e.result_revision,''),
                a.revision,e.phase,i.driver_id,i.version
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
             AND a.session_id=e.session_id AND a.generation=e.old_generation
             AND a.instance_id=e.instance_id AND a.state='STOPPED'
             AND a.stop_fact_id IS NOT NULL
           JOIN main.gogoke_v37_h_process_episode olde
             ON olde.domain_id=a.domain_id AND olde.session_id=a.session_id
             AND olde.generation=a.generation
             AND olde.process_operation_id=a.process_operation_id
             AND olde.phase='STOPPED' AND olde.stop_fact_id=a.stop_fact_id
           JOIN main.gogoke_coordination_process_custody oldc
             ON oldc.operation_id=a.process_operation_id AND oldc.domain_id=a.domain_id
             AND oldc.generation=a.generation AND oldc.state='STOPPED'
             AND oldc.stop_proof_hash=a.stop_fact_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation AND c.state IN ('PREPARED','ACTIVE','UNKNOWN')
             AND c.binary_digest_sha256=oldc.binary_digest_sha256
           JOIN main.gogoke_v37_h_seat_binding sb
             ON sb.domain_id=a.domain_id AND sb.session_id=a.session_id
             AND sb.generation=a.generation AND sb.seat_id=e.seat_id
             AND sb.seat_incarnation=e.seat_incarnation
           JOIN main.gogoke_v37_seats s ON s.domain_id=sb.domain_id
             AND s.seat_id=sb.seat_id AND s.incarnation=sb.seat_incarnation
             AND CAST(s.generation AS TEXT)=sb.generation AND s.state='BUSY'
             AND s.instance_id=e.instance_id
           JOIN main.gogoke_v37_h_owner_binding b ON b.binding_id=e.binding_id
             AND b.instance_id=e.instance_id AND b.domain_id=e.domain_id
             AND b.owner_id=e.session_id AND b.generation=e.generation
             AND b.kind='SESSION' AND b.state='ACTIVE'
           JOIN main.gogoke_v37_instance_homes h ON h.home_id=e.home_id
             AND h.instance_id=e.instance_id AND h.domain_id=e.domain_id
             AND h.owner_id=e.session_id AND h.generation=e.generation
             AND h.kind='SESSION' AND h.state='ACTIVE'
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
             AND i.login_state='LOGGED_IN' AND i.program_digest=c.binary_digest_sha256
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.process_operation_id=?4 AND e.generation=?5
            AND e.old_generation IS NOT NULL AND e.raw_hex=?6
            AND e.phase IN ('PREPARED','UNKNOWN')")?;
    let original_hex=hex(raw_request);
    for (index,value) in [domain,session,request_id,operation,generation,
        original_hex.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Ok(false);}
    let old=q.column_text(0)?;
    let before=q.column_text(1)?.parse::<u64>().map_err(|_|RpcJournalError::Denied)?;
    let after=q.column_text(2)?;
    let claim=q.column_text(3)?.parse::<u64>().map_err(|_|RpcJournalError::Denied)?;
    let phase=q.column_text(4)?;
    let driver=q.column_text(5)?;
    let version=q.column_text(6)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    let adjacent=old.parse::<u64>().ok().and_then(|value|value.checked_add(1))
        ==generation.parse::<u64>().ok();
    let claim_matches=if phase=="PREPARED" {claim==before}
        else {after.parse::<u64>().ok()==Some(claim)};
    Ok(adjacent && before==request.expected_revision && claim_matches
        && matches!((driver.as_str(),version.as_str()),
            ("codex","0.160.0")|("opencode","1.18.32")|("grok","1.0.41")))
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
    let (older,driver)=if let Some(episode)=generation_episode_old(db,domain,session,
        process_operation_id,generation,open_request_id)? {
        episode
    } else {return Err(RpcJournalError::Denied)};
    let step_id=if older {
        if driver=="codex" {format!("{process_operation_id}-thread-resume")}
        else {format!("{process_operation_id}-session-resume")}
    } else {"thread-start".to_owned()};
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
            AND r.state='NO_EVENT' AND r.no_event_reason=?9")?;
    for (index,value) in [domain,session,process_operation_id,generation,
        open_request_id,ticket,custodian_nonce,step_id.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    q.bind_text(9,if driver=="codex" {"CODEX_RPC_RESPONSE"} else {ACP_RESPONSE_NO_EVENT})?;
    if !q.step_row()? {return Err(RpcJournalError::Denied);}
    let command=unhex(&q.column_text(0)?)?;
    let response=unhex(&q.column_text(1)?)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    let thread=if driver=="codex" {
        if !older {codex_rpc::decode_stored_thread_start(&command,&response)?}
        else {codex_rpc::decode_stored_thread_resume(&command,&response)?}
    } else {
        let needed=if !older {AcpCapability::Initialize}
            else if driver=="opencode" {AcpCapability::Resume}
            else {AcpCapability::Load};
        if !observed_acp_capability(db,domain,session,process_operation_id,
            ticket,custodian_nonce,generation,open_request_id,needed)? {
            return Err(RpcJournalError::Denied);
        }
        let (id,method,requested)=stored_acp_pending(&command)
            .ok_or(RpcJournalError::Denied)?;
        let expected_method=if !older {acp::PendingMethod::SessionNew}
            else if driver=="opencode" {acp::PendingMethod::SessionResume}
            else {acp::PendingMethod::SessionLoad};
        if method!=expected_method {return Err(RpcJournalError::Denied);}
        let pending=acp::Pending {id:&id,method,requested_session_id:requested.as_deref()};
        match acp::decode(&response,Some(&pending)).map_err(|error|
            RpcJournalError::AcpDecode {reason:error.reason,raw_frame:error.raw_frame})? {
            acp::Observation::SessionNew {session_id,..} if !older => session_id,
            acp::Observation::SessionResume {..} | acp::Observation::SessionLoad {..} if older => {
                let requested=requested.ok_or(RpcJournalError::Denied)?;
                let old=observed_old_acp_session_id(db,domain,session,
                    process_operation_id,generation,open_request_id)?;
                if requested!=old {return Err(RpcJournalError::Denied);}
                old
            }
            _=>return Err(RpcJournalError::Denied),
        }
    };
    Ok(thread)
}

fn generation_episode_old(db: &VerifiedDatabaseConnection<'_>, domain: &str,
    session: &str, operation: &str, generation: &str,
    request_id: &str) -> Result<Option<(bool,String)>> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT CASE WHEN e.old_generation IS NULL THEN '0' ELSE '1' END,
                i.driver_id,i.version,e.phase,e.raw_hex,
                CASE WHEN g.request_id IS NULL THEN '1' ELSE '0' END,
                COALESCE(e.old_generation,'')
           FROM main.gogoke_v37_h_process_episode e
           LEFT JOIN main.gogoke_v37_h_generation g
             ON g.domain_id=e.domain_id AND g.request_id=e.request_id
             AND g.session_id=e.session_id AND g.generation=e.generation
             AND g.process_operation_id=e.process_operation_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.process_operation_id=?3
            AND e.generation=?4 AND e.request_id=?5
            AND ((g.request_id IS NOT NULL AND e.phase='STOPPED' AND c.state='STOPPED'
                   AND e.stop_fact_id=c.stop_proof_hash AND e.stop_fact_id IS NOT NULL)
              OR (g.request_id IS NOT NULL AND e.phase='ACTIVE'
                  AND c.state IN ('ACTIVE','UNKNOWN'))
              OR (e.phase IN ('PREPARED','UNKNOWN') AND e.old_generation IS NOT NULL
                  AND c.state IN ('ACTIVE','UNKNOWN')
                  AND EXISTS(SELECT 1 FROM main.gogoke_v37_h_claim a
                    JOIN main.gogoke_coordination_process_custody oldc
                      ON oldc.operation_id=a.process_operation_id
                     AND oldc.domain_id=a.domain_id AND oldc.generation=a.generation
                    WHERE a.domain_id=e.domain_id AND a.session_id=e.session_id
                      AND a.generation=e.old_generation AND a.state='STOPPED'
                      AND a.stop_fact_id IS NOT NULL
                      AND oldc.state='STOPPED'
                      AND oldc.stop_proof_hash=a.stop_fact_id)))")?;
    for (index,value) in [domain,session,operation,generation,request_id].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Ok(None);}
    let old=q.column_text(0)?=="1";
    let driver=q.column_text(1)?;
    let version=q.column_text(2)?;
    let phase=q.column_text(3)?;
    let original_hex=q.column_text(4)?;
    let candidate=q.column_text(5)?=="1";
    let old_generation=q.column_text(6)?;
    if q.step_row()? || !matches!((driver.as_str(),version.as_str()),
        ("codex","0.160.0")|("opencode","1.18.32")|("grok","1.0.41")) {
        return Err(RpcJournalError::Conflict);
    }
    if candidate && old && matches!(phase.as_str(),"PREPARED"|"UNKNOWN") {
        let change=super::generation_change::active_for_session(db,domain,session)?;
        let authorized=if let Some(change)=change {
            let same=change.request_id==request_id && change.session_id==session
                && change.old_generation==old_generation && change.stage=="OLD_STOPPED"
                && change.owner_stop_request_id.is_none()
                && change.raw_hex==original_hex;
            if !same {false} else {
                let old=Statement::prepare(db.as_ptr(),
                    "SELECT 1 FROM main.gogoke_v37_h_process_episode e
                       JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
                         AND a.session_id=e.session_id AND a.generation=e.old_generation
                         AND a.process_operation_id=?6 AND a.state='STOPPED'
                       JOIN main.gogoke_coordination_process_custody c
                         ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
                         AND c.generation=a.generation AND c.state='STOPPED'
                         AND c.stop_proof_hash=a.stop_fact_id
                      WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
                        AND e.process_operation_id=?4 AND e.generation=?5
                        AND e.seat_id=?7 AND c.ticket=?8 AND c.custodian_nonce=?9")?;
                for (index,value) in [domain,session,request_id,operation,generation,
                    change.old_operation.as_str(),change.seat_id.as_str(),
                    change.old_ticket.as_str(),change.old_nonce.as_str()]
                    .iter().enumerate() {old.bind_text((index+1) as i32,value)?;}
                old.step_row()? && !old.step_row()?
            }
        } else {
            plain_protocol_resume_episode(db,domain,session,request_id,operation,
                generation,&unhex(&original_hex)?)?
        };
        if !authorized {return Err(RpcJournalError::Denied);}
    }
    Ok(Some((old,driver)))
}

pub(super) fn observed_old_acp_session_id(db:&VerifiedDatabaseConnection<'_>,
    domain:&str,session:&str,operation:&str,generation:&str,
    request_id:&str)->Result<String> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT olde.process_operation_id,olde.generation,olde.request_id,
                oldc.ticket,oldc.custodian_nonce
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_process_episode olde
             ON olde.domain_id=e.domain_id AND olde.session_id=e.session_id
            AND olde.generation=e.old_generation AND olde.instance_id=e.instance_id
            AND olde.phase='STOPPED' AND olde.stop_fact_id IS NOT NULL
           JOIN main.gogoke_coordination_process_custody oldc
             ON oldc.operation_id=olde.process_operation_id
            AND oldc.domain_id=olde.domain_id AND oldc.generation=olde.generation
            AND oldc.state='STOPPED' AND oldc.stop_proof_hash=olde.stop_fact_id
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.process_operation_id=?3
            AND e.generation=?4 AND e.request_id=?5 AND e.old_generation IS NOT NULL")?;
    for (index,value) in [domain,session,operation,generation,request_id].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Err(RpcJournalError::Denied);}
    let old_operation=q.column_text(0)?;
    let old_generation=q.column_text(1)?;
    let old_request=q.column_text(2)?;
    let old_ticket=q.column_text(3)?;
    let old_nonce=q.column_text(4)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    drop(q);
    let Some((old,new))=old_generation.parse::<u64>().ok()
        .zip(generation.parse::<u64>().ok()) else {return Err(RpcJournalError::Denied)};
    if old>=new {return Err(RpcJournalError::Denied);}
    observed_thread_id(db,domain,session,&old_operation,&old_generation,
        &old_request,&old_ticket,&old_nonce)
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
    transact(db,|db| {
        check_owner_in_current_transaction(db,owner)?;
        let Some((true,driver))=generation_episode_old(db,domain,session,
            operation,generation,request_id)? else {return Err(RpcJournalError::Denied)};
        let step_id=if driver=="codex" {format!("{operation}-thread-resume")}
            else {format!("{operation}-session-resume")};
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
            let thread=if driver=="codex" {
                match codex_rpc::decode_stored_thread_resume(&command,&bytes) {
                    Ok(thread)=>Some(thread),
                    Err(codex_rpc::RpcError::RemoteResponse(_))=>None,
                    Err(_)=>continue,
                }
            } else {
                let Some((id,method,requested))=stored_acp_pending(&command) else {continue};
                let expected=if driver=="opencode" {acp::PendingMethod::SessionResume}
                    else {acp::PendingMethod::SessionLoad};
                if method!=expected {continue;}
                let requested=requested.ok_or(RpcJournalError::Denied)?;
                if !observed_acp_capability(db,domain,session,operation,
                    &ticket,&nonce,generation,request_id,
                    if driver=="opencode" {AcpCapability::Resume}
                    else {AcpCapability::Load})? {return Err(RpcJournalError::Denied);}
                let pending=acp::Pending {id:&id,method,
                    requested_session_id:Some(&requested)};
                match acp::decode(&bytes,Some(&pending)) {
                    Ok(acp::Observation::SessionResume {..})
                    | Ok(acp::Observation::SessionLoad {..}) => {
                        let old=observed_old_acp_session_id(db,domain,session,
                            operation,generation,request_id)?;
                        if requested!=old {return Err(RpcJournalError::Denied);}
                        Some(old)
                    },
                    Ok(acp::Observation::RemoteError {..})=>None,
                    _=>continue,
                }
            };
            if matched.is_some() {return Err(RpcJournalError::Conflict);}
            matched=Some((source.column_text(0)?,source.column_text(1)?,thread));
        }
        drop(source);
        let Some((epoch,cursor,thread))=matched else {return Ok(None)};
        let key=RawSourceKey {operation_id:operation.to_owned(),source_epoch:epoch,
            source_cursor:cursor};
        persist_observation_and_no_event_with_reason(db,domain,session,
            &step_id,operation,&key,if driver=="codex" {RPC_RESPONSE_NO_EVENT}
                else {ACP_RESPONSE_NO_EVENT})?;
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
    let (operation, vendor) = assert_current_binding(db, &step.fields(), required_state,
        allow_unknown_claim)?;
    if vendor != "codex" { return Err(RpcJournalError::Denied); }
    Ok(operation)
}

/// All providers use this same current H/seat/instance/home/custody binding.
/// The driver is returned from the joined instance, never accepted from input.
fn assert_current_binding(
    db: &VerifiedDatabaseConnection<'_>,
    step: &StepFields<'_>,
    required_state: &[&str],
    allow_unknown_claim: bool,
) -> Result<(String, String)> {
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
                a.generation,b.instance_id,s.instance_id,
                i.driver_id,i.version,i.login_state,i.program_digest
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
           JOIN main.gogoke_v37_instance_homes h ON h.home_id=a.home_id
            AND h.instance_id=a.instance_id AND h.domain_id=a.domain_id
            AND h.owner_id=a.session_id AND h.generation=a.generation
            AND h.kind='SESSION' AND h.state='ACTIVE'
           JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id
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
    let driver = q.column_text(19)?;
    let version = q.column_text(20)?;
    let login_state = q.column_text(21)?;
    let program_digest = q.column_text(22)?;
    if !(claim_state == "COMMITTED" || (allow_unknown_claim && claim_state == "UNKNOWN"))
        || owner_state != "ACTIVE"
        || seat_state != "BUSY"
        || !required_state.contains(&custody_state.as_str())
        || seat_incarnation.is_empty()
        || seat_generation != claim_generation
        || claim_generation != c.binding.generation
        || owner_instance != seat_instance
        || login_state != "LOGGED_IN"
        || program_digest != c.binding.binary_digest_sha256
        || !matches!((driver.as_str(), version.as_str()),
            ("codex", "0.160.0") | ("opencode", "1.18.32") | ("grok", "1.0.41")
            | ("claude", "2.1.196"))
        || q.step_row()?
    {
        return Err(RpcJournalError::Denied);
    }
    Ok((operation, driver))
}

/// Reuse the ordinary H claim/seat/instance/home and full physical custody
/// check for a captured Codex model call. The open bytes come from H's own
/// episode, not from the tool arguments or the caller.
pub(super) fn current_codex_model_binding(
    db:&VerifiedDatabaseConnection<'_>, custody:&PreparedCustody,
    domain:&str, session:&str,
) -> Result<(String,String,String,String)> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT e.request_id,e.raw_hex,e.seat_id,e.seat_incarnation
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
             AND a.session_id=e.session_id AND a.generation=e.generation
             AND a.process_operation_id=e.process_operation_id
             AND a.state='COMMITTED'
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.generation=?3
            AND e.phase='ACTIVE' AND e.stop_request_id IS NULL")?;
    q.bind_text(1,domain)?;q.bind_text(2,session)?;
    q.bind_text(3,&custody.binding.generation)?;
    if !q.step_row()? {return Err(RpcJournalError::Denied);}
    let open_id=q.column_text(0)?;
    let open_bytes=unhex(&q.column_text(1)?)?;
    let seat_id=q.column_text(2)?;
    let incarnation=q.column_text(3)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    drop(q);
    let fields=StepFields {domain_id:domain,session_id:session,
        open_request_id:&open_id,open_request_bytes:&open_bytes,
        step_id:"model-call-source",custody};
    original_open(db,&fields)?;
    let (operation,driver)=assert_current_binding(db,&fields,&["ACTIVE"],false)?;
    if driver!="codex" || !atom(&seat_id) || !atom(&incarnation) {
        return Err(RpcJournalError::Denied);
    }
    Ok((operation,open_id,seat_id,incarnation))
}

fn original_open(db: &VerifiedDatabaseConnection<'_>, step: &StepFields<'_>) -> Result<()> {
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
    prepare_with_model_caller(db,owner,step,None)
}

pub(crate) fn prepare_model_tool_response(
    db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    caller:&crate::store::seat::NativeSeatCall,step:&Step<'_>,
) -> Result<PreparedStep> {
    prepare_with_model_caller(db,owner,step,Some(caller))
}

fn prepare_with_model_caller(
    db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    step:&Step<'_>,caller:Option<&crate::store::seat::NativeSeatCall>,
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
        if let Command::DynamicToolResponse {request_id,..}=step.command {
            let caller=caller.ok_or(RpcJournalError::Denied)?;
            super::model_call::revalidate_model_call_in_transaction(db,caller)
                .map_err(|_|RpcJournalError::Denied)?;
            let proof=caller.model_proof().ok_or(RpcJournalError::Denied)?;
            if step.step_id!=proof.host_request_id() || step.rpc_id.is_some()
                || step.custody!=proof.custody()
                || step.domain_id!=proof.domain_id()
                || step.session_id!=proof.session_id()
                || request_id!=proof.typed_rpc_id() {
                return Err(RpcJournalError::Denied);
            }
        } else if caller.is_some() {return Err(RpcJournalError::Denied);}
        original_open(db, &step.fields())?;
        let operation = assert_native_binding(db, step, &["PREPARED", "ACTIVE"], false)?;
        if let Some(change)=super::generation_change::active_for_session(db,
            step.domain_id,step.session_id)? {
            if change.owner_stop_request_id.is_some() {return Err(RpcJournalError::Denied);}
            let compact_step=format!("compact-{}",
                &crate::store::digest::sha256_hex(
                    &unhex(&change.raw_hex)?)[..40]);
            let original_compact=change.operation=="compact" && change.stage=="INTENT"
                && operation==change.old_operation && step.step_id==compact_step
                && matches!(step.command,Command::ThreadCompactStart {ref thread_id}
                    if thread_id==&change.thread_id);
            let candidate_handshake=change.stage=="OLD_STOPPED"
                && step.open_request_id==change.request_id && operation!=change.old_operation
                && matches!(step.command,Command::Initialize {..}
                    |Command::InitializeHostTools {..}|Command::Initialized
                    |Command::ConfigRead {..}|Command::ThreadResume {..});
            if !original_compact && !candidate_handshake {return Err(RpcJournalError::Denied);}
        }
        if let Some(phase) = same_row(db, &step.fields(), &operation, &encoded)? {
            return Ok(PreparedStep {
                bytes: encoded,
                disposition: Disposition::Existing(phase),
            });
        }
        if has_unresolved(db, step.domain_id, step.session_id, &operation)? {
            return Err(RpcJournalError::Unknown);
        }
        insert_intent(db, &step.fields(), &operation, &encoded,
            requires_response(step.command))?;
        Ok(PreparedStep {
            bytes: encoded,
            disposition: Disposition::NewWrite,
        })
    })
}

fn insert_intent(db: &VerifiedDatabaseConnection<'_>, step: &StepFields<'_>,
    operation: &str, encoded: &[u8], needs_response: bool) -> Result<()> {
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
            operation,
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
            if needs_response {
                1
            } else {
                0
            },
        )?;
        q.step_done()?;
        Ok(())
}

fn transition(
    db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    step: &Step<'_>,
    next: Phase,
    error: Option<&str>,
) -> Result<()> {
    let encoded = step.command.encode(step.rpc_id)?;
    // The private Codex failure slot can retain a 4096-byte unfinished stdout
    // tail as hex beside the original read error. No successful frame uses it.
    if next == Phase::Unknown && error.map_or(true, |value| value.is_empty() || value.len() > 16_384)
    {
        return Err(RpcJournalError::Invalid("original error"));
    }
    transact(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, &step.fields())?;
        let operation = if next == Phase::Unknown {
            assert_native_binding(db, step, &["ACTIVE", "UNKNOWN"], true)?
        } else {
            assert_native_binding(db, step, &["ACTIVE"], false)?
        };
        transition_row(db, &step.fields(), &operation, &encoded, next, error)
    })
}

fn transition_row(db: &VerifiedDatabaseConnection<'_>, step: &StepFields<'_>,
    operation: &str, encoded: &[u8], next: Phase, error: Option<&str>) -> Result<()> {
        let prior = same_row(db, step, operation, encoded)?;
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
                operation,
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
                operation,
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
    step: &StepFields<'_>,
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
    persist_observation_and_no_event_with_reason(db, domain_id, session_id,
        step_id, operation, key, RPC_RESPONSE_NO_EVENT)
}

fn persist_observation_and_no_event_with_reason(
    db: &mut VerifiedDatabaseConnection<'_>, domain_id: &str,
    session_id: &str, step_id: &str, operation: &str,
    key: &RawSourceKey, reason: &str,
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
        &key.source_cursor,reason)?;
    Ok(())
}

fn observed_source_is_exact(db: &VerifiedDatabaseConnection<'_>,
    step: &StepFields<'_>, operation: &str, key: &RawSourceKey) -> Result<bool> {
    let q = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_rpc_steps WHERE domain_id=?1
          AND session_id=?2 AND step_id=?3 AND process_operation_id=?4
          AND source_epoch=?5 AND source_cursor=?6 AND phase='OBSERVED'")?;
    for (index, value) in [step.domain_id, step.session_id, step.step_id,
        operation, key.source_epoch.as_str(), key.source_cursor.as_str()].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    Ok(q.step_row()? && !q.step_row()?)
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
        original_open(db, &step.fields())?;
        let operation = assert_native_binding(db, step, &["ACTIVE","UNKNOWN"], false)?;
        source_matches(db, frame, key, &operation, &step.fields())?;
        if !matches!(
            same_row(db, &step.fields(), &operation, &encoded)?,
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

fn acp_vendor(driver: &str) -> Result<commands::Vendor> {
    match driver {
        "opencode" => Ok(commands::Vendor::OpenCode),
        "grok" => Ok(commands::Vendor::Grok),
        _ => Err(RpcJournalError::Denied),
    }
}

fn assert_acp_binding(db: &VerifiedDatabaseConnection<'_>, step: &AcpStep<'_>,
    states: &[&str], allow_unknown_claim: bool) -> Result<(String,String,bool)> {
    if let Some((operation,driver))=candidate_binding_fields(db,&step.fields(),states)? {
        if !matches!((driver.as_str(),step.command),
            ("opencode", commands::AcpCommand::Initialize { .. })
            | ("opencode", commands::AcpCommand::SessionResume { .. })
            | ("opencode", commands::AcpCommand::SetConfigOption { .. })
            | ("grok", commands::AcpCommand::Initialize { .. })
            | ("grok", commands::AcpCommand::SessionLoad { .. })) {
            return Err(RpcJournalError::Denied);
        }
        let change=super::generation_change::active_for_session(db,
            step.domain_id,step.session_id)?;
        let authorized=if let Some(change)=change.as_ref() {
            candidate_acp_change_matches(db,step,&operation,change)?
        } else {
            acp_candidate_step_id(&operation,step.command)?==step.step_id
                && plain_protocol_resume_episode(db,step.domain_id,step.session_id,
                    step.open_request_id,&operation,&step.custody.binding.generation,
                    step.open_request_bytes)?
        };
        if !authorized {return Err(RpcJournalError::Denied);}
        return Ok((operation,driver,true));
    }
    let (operation,driver)=assert_current_binding(db,&step.fields(),states,
        allow_unknown_claim)?;
    acp_vendor(&driver)?;
    Ok((operation,driver,false))
}

pub(crate) fn acp_candidate_step_id(operation:&str,
    command:&commands::AcpCommand<'_>)->Result<String> {
    if !atom(operation) {return Err(RpcJournalError::Invalid("candidate operation"));}
    let suffix=match command {
        commands::AcpCommand::Initialize {..}=>"initialize",
        commands::AcpCommand::SessionResume {..}
        | commands::AcpCommand::SessionLoad {..}=>"session-resume",
        commands::AcpCommand::SetConfigOption {config_id,..} if *config_id=="model"=>"setting-model",
        commands::AcpCommand::SetConfigOption {config_id,..} if *config_id=="effort"=>"setting-effort",
        _=>return Err(RpcJournalError::Denied),
    };
    Ok(format!("{operation}-{suffix}"))
}

fn candidate_acp_change_matches(db:&VerifiedDatabaseConnection<'_>,
    step:&AcpStep<'_>,operation:&str,
    change:&super::generation_change::Change)->Result<bool> {
    if change.stage!="OLD_STOPPED" || change.owner_stop_request_id.is_some()
        || change.request_id!=step.open_request_id
        || change.raw_hex!=hex(step.open_request_bytes)
        || acp_candidate_step_id(operation,step.command)?!=step.step_id {
        return Ok(false);
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id
             AND a.session_id=e.session_id AND a.generation=e.old_generation
             AND a.state='STOPPED' AND a.process_operation_id=?5
           JOIN main.gogoke_coordination_process_custody oldc
             ON oldc.operation_id=a.process_operation_id AND oldc.domain_id=a.domain_id
             AND oldc.generation=a.generation AND oldc.state='STOPPED'
             AND oldc.stop_proof_hash=a.stop_fact_id
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.process_operation_id=?4 AND e.old_generation=?6
            AND e.raw_hex=?7 AND oldc.ticket=?8 AND oldc.custodian_nonce=?9")?;
    for (index,value) in [step.domain_id,step.session_id,step.open_request_id,
        operation,change.old_operation.as_str(),change.old_generation.as_str(),
        change.raw_hex.as_str(),change.old_ticket.as_str(),change.old_nonce.as_str()]
        .iter().enumerate() {q.bind_text((index+1) as i32,value)?;}
    Ok(q.step_row()? && !q.step_row()?)
}

fn acp_command_copy<'a>(command: &commands::AcpCommand<'a>)
    -> commands::AcpCommand<'a> {
    use commands::AcpCommand as C;
    match command {
        C::Initialize { client_version } => C::Initialize { client_version },
        C::SessionNew { cwd } => C::SessionNew { cwd },
        C::SessionLoad { session_id, cwd, advertised } =>
            C::SessionLoad { session_id, cwd, advertised: *advertised },
        C::SessionResume { session_id, cwd, advertised } =>
            C::SessionResume { session_id, cwd, advertised: *advertised },
        C::SetConfigOption { session_id, config_id, value } =>
            C::SetConfigOption { session_id, config_id, value },
        C::Prompt { session_id, text } => C::Prompt { session_id, text },
        C::Cancel { session_id } => C::Cancel { session_id },
        C::PermissionResponse => C::PermissionResponse,
        C::Steer => C::Steer,
    }
}

fn encode_acp_step(step: &AcpStep<'_>, driver: &str) -> Result<Vec<u8>> {
    commands::encode_acp(acp_vendor(driver)?, step.rpc_id,
        acp_command_copy(step.command)).map_err(RpcJournalError::AcpEncode)
}

fn acp_pending<'a>(step: &'a AcpStep<'_>) -> Result<Option<acp::Pending<'a>>> {
    use commands::AcpCommand as C;
    let (method, requested_session_id) = match step.command {
        C::Initialize { .. } => (acp::PendingMethod::Initialize, None),
        C::SessionNew { .. } => (acp::PendingMethod::SessionNew, None),
        C::SessionLoad { session_id, .. } =>
            (acp::PendingMethod::SessionLoad, Some(*session_id)),
        C::SessionResume { session_id, .. } =>
            (acp::PendingMethod::SessionResume, Some(*session_id)),
        C::SetConfigOption { session_id, .. } =>
            (acp::PendingMethod::SessionSetConfigOption, Some(*session_id)),
        C::Prompt { .. } => (acp::PendingMethod::SessionPrompt, None),
        C::Cancel { .. } => return Ok(None),
        C::PermissionResponse | C::Steer =>
            return Err(RpcJournalError::AcpEncode(commands::EncodeError::Unsupported(
                "no frozen ACP command capability"))),
    };
    let id = step.rpc_id.ok_or(RpcJournalError::Invalid("ACP request id"))?;
    Ok(Some(acp::Pending { id, method, requested_session_id }))
}

fn stored_method_for_session(encoded: &[u8], session_id: &str, method: &str) -> bool {
    let Ok(text) = std::str::from_utf8(encoded) else { return false };
    let Ok(Json::Object(fields)) = Parser::parse(text.trim_end_matches('\n')) else {
        return false;
    };
    let key = |name| JsonString::from_str(name);
    if !matches!(fields.get(&key("method")), Some(Json::String(value))
        if value.to_well_formed_string().as_deref() == Some(method)) {
        return false;
    }
    let Some(Json::Object(params)) = fields.get(&key("params")) else { return false };
    matches!(params.get(&key("sessionId")), Some(Json::String(value))
        if value.to_well_formed_string().as_deref() == Some(session_id))
}

/// Cancellation can follow exactly a WRITTEN matching prompt. An INTENT,
/// UNKNOWN, or unrelated response waiter still prevents another stdin write.
fn cancel_follows_prompt(db: &VerifiedDatabaseConnection<'_>, domain: &str,
    session: &str, operation: &str, session_id: &str) -> Result<Option<String>> {
    let q = Statement::prepare(db.as_ptr(),
        "SELECT step_id,phase,command_hex FROM main.gogoke_v37_rpc_steps
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND (phase IN ('INTENT','UNKNOWN') OR
                 (phase='WRITTEN' AND requires_response=1))")?;
    q.bind_text(1, domain)?;
    q.bind_text(2, session)?;
    q.bind_text(3, operation)?;
    let mut found = None;
    while q.step_row()? {
        if q.column_text(1)? != "WRITTEN"
            || !stored_method_for_session(&unhex(&q.column_text(2)?)?,
                session_id, "session/prompt") || found.is_some() { return Ok(None); }
        found = Some(q.column_text(0)?);
    }
    Ok(found)
}

/// One stable cancel identity per original prompt; a retry reads the same
/// journal row and cannot send again, while a later prompt may be cancelled.
pub(crate) fn acp_cancel_step_id(prompt_step_id: &str) -> Result<String> {
    if !atom(prompt_step_id) { return Err(RpcJournalError::Invalid("prompt step")); }
    Ok(format!("cancel-{}", &crate::store::digest::sha256_hex(
        prompt_step_id.as_bytes())[..40]))
}

fn stored_initialize_id(encoded: &[u8]) -> Option<acp::RpcId> {
    let text = std::str::from_utf8(encoded).ok()?;
    let Json::Object(fields) = Parser::parse(text.trim_end_matches('\n')).ok()? else {
        return None;
    };
    let key = |name| JsonString::from_str(name);
    if !matches!(fields.get(&key("method")), Some(Json::String(value))
        if value.to_well_formed_string().as_deref() == Some("initialize")) {
        return None;
    }
    match fields.get(&key("id"))? {
        Json::String(value) => Some(acp::RpcId::String(value.to_well_formed_string()?)),
        Json::Number(value) => Some(acp::RpcId::Number(value.parse().ok()?)),
        _ => None,
    }
}

/// The caller's advertised flag is only a request to use the capability. Its
/// authority comes from the original initialize ACK captured by A on this
/// exact process, and from the journal's matching typed request ID.
#[derive(Clone,Copy)]
enum AcpCapability { Initialize, Load, Resume }

fn observed_acp_capability(db: &VerifiedDatabaseConnection<'_>,
    domain:&str, session:&str, operation:&str, ticket:&str, nonce:&str,
    generation:&str, open_request_id:&str, needed:AcpCapability) -> Result<bool> {
    let q = Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes)
           FROM main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r
             ON r.operation_id=s.process_operation_id
            AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
            AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
            AND r.domain_id=s.domain_id AND r.session_id=s.session_id
            AND r.generation=s.generation
          WHERE s.domain_id=?1 AND s.session_id=?2
            AND s.process_operation_id=?3 AND s.ticket=?4
            AND s.custodian_nonce=?5 AND s.generation=?6
            AND s.open_request_id=?7
            AND s.phase='OBSERVED' AND s.requires_response=1
            AND r.state='NO_EVENT' AND r.no_event_reason='ACP_RPC_RESPONSE'")?;
    for (index, value) in [domain, session, operation, ticket, nonce,
        generation, open_request_id].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    let mut found = None;
    while q.step_row()? {
        let command = unhex(&q.column_text(0)?)?;
        let Some(id) = stored_initialize_id(&command) else { continue };
        if found.is_some() { return Err(RpcJournalError::Conflict); }
        let response = unhex(&q.column_text(1)?)?;
        let pending = acp::Pending { id: &id, method: acp::PendingMethod::Initialize,
            requested_session_id: None };
        let observation = acp::decode(&response, Some(&pending)).map_err(|error|
            RpcJournalError::AcpDecode { reason: error.reason, raw_frame: error.raw_frame })?;
        let acp::Observation::Initialize { declared_capabilities: Json::Object(capabilities), .. }
            = observation else { return Err(RpcJournalError::Denied) };
        found = Some(match needed {
            AcpCapability::Initialize => true,
            AcpCapability::Load => matches!(capabilities.get(&JsonString::from_str("loadSession")),
                Some(Json::Bool(true))),
            AcpCapability::Resume => matches!(capabilities.get(&JsonString::from_str("sessionCapabilities")),
                Some(Json::Object(session_caps)) if matches!(session_caps.get(&JsonString::from_str("resume")),
                    Some(Json::Object(_)))),
        });
    }
    Ok(found.unwrap_or(false))
}

fn stored_acp_pending(encoded: &[u8])
    -> Option<(acp::RpcId, acp::PendingMethod, Option<String>)> {
    let text = std::str::from_utf8(encoded).ok()?;
    let Json::Object(fields) = Parser::parse(text.trim_end_matches('\n')).ok()? else {
        return None;
    };
    let key = |name| JsonString::from_str(name);
    let method = match fields.get(&key("method"))? {
        Json::String(value) => value.to_well_formed_string()?,
        _ => return None,
    };
    let kind = match method.as_str() {
        "initialize" => acp::PendingMethod::Initialize,
        "session/new" => acp::PendingMethod::SessionNew,
        "session/load" => acp::PendingMethod::SessionLoad,
        "session/resume" => acp::PendingMethod::SessionResume,
        _ => return None,
    };
    let id = match fields.get(&key("id"))? {
        Json::String(value) => acp::RpcId::String(value.to_well_formed_string()?),
        Json::Number(value) => acp::RpcId::Number(value.parse().ok()?),
        _ => return None,
    };
    let requested = if matches!(kind, acp::PendingMethod::SessionLoad
        | acp::PendingMethod::SessionResume) {
        let Json::Object(params) = fields.get(&key("params"))? else { return None };
        let Json::String(value) = params.get(&key("sessionId"))? else { return None };
        Some(value.to_well_formed_string()?)
    } else { None };
    Some((id, kind, requested))
}

fn stored_acp_config(encoded:&[u8])->Option<(acp::RpcId,String,String,String)> {
    let text=std::str::from_utf8(encoded).ok()?;
    let Json::Object(fields)=Parser::parse(text.trim_end_matches('\n')).ok()? else {return None};
    let key=|name|JsonString::from_str(name);
    if !matches!(fields.get(&key("method")),Some(Json::String(value))
        if value.to_well_formed_string().as_deref()==Some("session/set_config_option")) {
        return None;
    }
    let id=match fields.get(&key("id"))? {
        Json::String(value)=>acp::RpcId::String(value.to_well_formed_string()?),
        Json::Number(value)=>acp::RpcId::Number(value.parse().ok()?),
        _=>return None,
    };
    let Json::Object(params)=fields.get(&key("params"))? else {return None};
    let string=|name|match params.get(&key(name))? {
        Json::String(value)=>value.to_well_formed_string(),_=>None,
    };
    Some((id,string("sessionId")?,string("configId")?,string("value")?))
}

/// Original candidate OpenCode model/effort ACKs, in that order, must report
/// the bound E values for the same actual resumed native session. Grok has no
/// verified ACP config method; its settings are checked at launch by H.
pub(super) fn observed_acp_configuration_for_generation(
    db:&VerifiedDatabaseConnection<'_>,domain:&str,session:&str,operation:&str,
    generation:&str,open_request_id:&str,ticket:&str,nonce:&str,
    vendor_session:&str)->Result<()> {
    let settings=Statement::prepare(db.as_ptr(),
        "SELECT ss.settings_json FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_seats s ON s.domain_id=e.domain_id
             AND s.seat_id=e.seat_id AND s.incarnation=e.seat_incarnation
             AND s.instance_id=e.instance_id AND s.state='BUSY'
           JOIN main.gogoke_v37_seat_settings ss ON ss.domain_id=s.domain_id
             AND ss.seat_id=s.seat_id
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.process_operation_id=?3
            AND e.generation=?4 AND e.request_id=?5")?;
    for (index,value) in [domain,session,operation,generation,open_request_id]
        .iter().enumerate() {settings.bind_text((index+1) as i32,value)?;}
    if !settings.step_row()? {return Err(RpcJournalError::Denied);}
    let raw=settings.column_text(0)?;
    if settings.step_row()? {return Err(RpcJournalError::Conflict);}
    drop(settings);
    let Json::Object(fields)=Parser::parse(&raw)? else {return Err(RpcJournalError::Denied)};
    let field=|name|match fields.get(&JsonString::from_str(name)) {
        Some(Json::String(value))=>value.to_well_formed_string()
            .filter(|value|!value.is_empty()&&!value.contains('\0')),
        _=>None,
    };
    let model=field("model").ok_or(RpcJournalError::Denied)?;
    let effort=field("effort").ok_or(RpcJournalError::Denied)?;
    let mut order=Vec::new();
    for (config_id,value) in [("model",model.as_str()),("effort",effort.as_str())] {
        let step_id=format!("{operation}-setting-{config_id}");
        let q=Statement::prepare(db.as_ptr(),
            "SELECT s.command_hex,hex(r.raw_bytes),s.source_epoch,s.source_cursor
               FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id
                 AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
                AND s.generation=?4 AND s.open_request_id=?5 AND s.ticket=?6
                AND s.custodian_nonce=?7 AND s.step_id=?8 AND s.phase='OBSERVED'
                AND s.requires_response=1 AND r.state='NO_EVENT'
                AND r.no_event_reason='ACP_RPC_RESPONSE'")?;
        for (index,item) in [domain,session,operation,generation,open_request_id,
            ticket,nonce,step_id.as_str()].iter().enumerate() {
            q.bind_text((index+1) as i32,item)?;
        }
        if !q.step_row()? {return Err(RpcJournalError::Denied);}
        let command=unhex(&q.column_text(0)?)?;
        let response=unhex(&q.column_text(1)?)?;
        let epoch=q.column_text(2)?;
        let cursor_raw=q.column_text(3)?;
        let cursor=cursor_raw.parse::<u64>()
            .map_err(|_|RpcJournalError::Denied)?;
        if cursor==0 || cursor>i64::MAX as u64 || cursor.to_string()!=cursor_raw {
            return Err(RpcJournalError::Denied);
        }
        if q.step_row()? {return Err(RpcJournalError::Conflict);}
        drop(q);
        let (id,requested_session,requested_config,requested_value)=
            stored_acp_config(&command).ok_or(RpcJournalError::Denied)?;
        if requested_session!=vendor_session || requested_config!=config_id
            || requested_value!=value {return Err(RpcJournalError::Denied);}
        let pending=acp::Pending {id:&id,method:acp::PendingMethod::SessionSetConfigOption,
            requested_session_id:Some(vendor_session)};
        let observed=acp::decode(&response,Some(&pending)).map_err(|error|
            RpcJournalError::AcpDecode {reason:error.reason,raw_frame:error.raw_frame})?;
        if !acp::confirms_config_value(&observed,config_id,value)
            || (config_id=="effort" && !acp::confirms_config_value(&observed,"model",&model)) {
            return Err(RpcJournalError::Denied);
        }
        order.push((epoch,cursor));
    }
    if order.len()!=2 || order[0].0!=order[1].0 || order[0].1>=order[1].1 {
        return Err(RpcJournalError::Denied);
    }
    Ok(())
}

/// One actual initialized ACP process and one native session acquisition
/// establish the provider session for H. Neither a caller label nor a
/// session/update notification can select the session sent to stdin.
pub(super) fn observed_acp_session_id_in_transaction(
    db: &VerifiedDatabaseConnection<'_>, domain_id: &str, session_id: &str,
    open_request_id: &str, open_request_bytes: &[u8],
    custody: &PreparedCustody, allow_unknown_claim: bool,
) -> Result<String> {
    let fields = StepFields { domain_id, session_id, open_request_id,
        open_request_bytes, step_id: "acp-session-proof", custody };
    original_open(db, &fields)?;
    let (operation, driver) = assert_current_binding(db, &fields,
        &["ACTIVE", "UNKNOWN"], allow_unknown_claim)?;
    acp_vendor(&driver)?;
    let q = Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,s.source_epoch,s.source_cursor
           FROM main.gogoke_v37_rpc_steps s
          WHERE s.domain_id=?1 AND s.session_id=?2
            AND s.open_request_id=?3 AND s.process_operation_id=?4
            AND s.ticket=?5 AND s.custodian_nonce=?6 AND s.generation=?7
            AND s.phase='OBSERVED' AND s.requires_response=1")?;
    for (index, value) in [domain_id, session_id,
        open_request_id, operation.as_str(), custody.ticket.opaque(),
        custody.custodian_nonce.as_str(),
        custody.binding.generation.as_str()].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    let mut initialized = false;
    let mut session = None;
    while q.step_row()? {
        let command = unhex(&q.column_text(0)?)?;
        let Some((id, method, requested)) = stored_acp_pending(&command) else { continue };
        let epoch = q.column_text(1)?;
        let cursor = q.column_text(2)?;
        let source = ledger::read_captured_raw_source(db, &operation, &epoch, &cursor)?
            .ok_or(RpcJournalError::Denied)?;
        if source.state != RawSourceState::NoEvent
            || source.no_event_reason.as_deref() != Some(ACP_RESPONSE_NO_EVENT)
            || source.process_ticket != custody.ticket.opaque()
            || source.custodian_nonce != custody.custodian_nonce
            || source.domain_id != domain_id || source.session_id != session_id
            || source.generation != custody.binding.generation {
            return Err(RpcJournalError::Denied);
        }
        let pending = acp::Pending { id: &id, method,
            requested_session_id: requested.as_deref() };
        let observation = acp::decode(&source.raw_bytes, Some(&pending)).map_err(|error|
            RpcJournalError::AcpDecode { reason: error.reason, raw_frame: error.raw_frame })?;
        match observation {
            acp::Observation::Initialize { .. } => {
                if initialized { return Err(RpcJournalError::Conflict); }
                initialized = true;
            }
            acp::Observation::SessionNew { session_id, .. } => {
                if session.replace(session_id).is_some() { return Err(RpcJournalError::Conflict); }
            }
            acp::Observation::SessionLoad { .. }
            | acp::Observation::SessionResume { .. } => {
                let requested = requested.ok_or(RpcJournalError::Denied)?;
                if session.replace(requested).is_some() { return Err(RpcJournalError::Conflict); }
            }
            _ => return Err(RpcJournalError::Denied),
        }
    }
    if !initialized { return Err(RpcJournalError::Denied); }
    session.ok_or(RpcJournalError::Denied)
}

/// Persist INTENT before H writes ACP stdin. The returned bytes are a send
/// permit only when disposition is NewWrite; all readbacks prohibit resend.
pub(crate) fn prepare_acp(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>) -> Result<PreparedStep> {
    transact(db, |db| prepare_acp_in_transaction(db, owner, step))
}

/// For the H User intent composite transaction; the caller already holds
/// BEGIN IMMEDIATE on this same verified connection.
pub(super) fn prepare_acp_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>) -> Result<PreparedStep> {
    for (value, name) in [(step.domain_id, "domain"), (step.session_id, "session"),
        (step.open_request_id, "open request"), (step.step_id, "step")] {
        if !atom(value) { return Err(RpcJournalError::Invalid(name)); }
    }
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, &step.fields())?;
        let (operation, driver, candidate) = assert_acp_binding(db, step,
            &["PREPARED", "ACTIVE"], false)?;
        let encoded = encode_acp_step(step, &driver)?;
        let pending = acp_pending(step)?;
        if candidate {
            if let commands::AcpCommand::SessionResume {session_id,..}
                | commands::AcpCommand::SessionLoad {session_id,..}=step.command {
                if observed_old_acp_session_id(db,step.domain_id,step.session_id,
                    &operation,&step.custody.binding.generation,
                    step.open_request_id)?!=*session_id {
                    return Err(RpcJournalError::Denied);
                }
            }
        }
        if let commands::AcpCommand::SetConfigOption { session_id, .. } = step.command {
            let observed=if candidate {observed_thread_id(db,step.domain_id,
                step.session_id,&operation,&step.custody.binding.generation,
                step.open_request_id,step.custody.ticket.opaque(),
                &step.custody.custodian_nonce)?} else {
                observed_acp_session_id_in_transaction(db, step.domain_id,
                step.session_id, step.open_request_id, step.open_request_bytes,
                step.custody, false)?};
            if observed != *session_id {
                return Err(RpcJournalError::Denied);
            }
        }
        if matches!(step.command, commands::AcpCommand::SessionLoad { .. }
            | commands::AcpCommand::SessionResume { .. })
            && !observed_acp_capability(db, step.domain_id, step.session_id,
                &operation, step.custody.ticket.opaque(), &step.custody.custodian_nonce,
                &step.custody.binding.generation, step.open_request_id,
                if matches!(step.command, commands::AcpCommand::SessionResume { .. }) {
                    AcpCapability::Resume
                } else { AcpCapability::Load })? {
            return Err(RpcJournalError::AcpEncode(commands::EncodeError::Unsupported(
                "session load capability not observed on this process")));
        }
        let change=super::generation_change::active_for_session(db,
            step.domain_id, step.session_id)?;
        if !candidate && change.is_some() {
            return Err(RpcJournalError::Denied);
        }
        if let Some(phase) = same_row(db, &step.fields(), &operation, &encoded)? {
            return Ok(PreparedStep { bytes: encoded,
                disposition: Disposition::Existing(phase) });
        }
        let allowed = match step.command {
            commands::AcpCommand::Cancel { session_id } => {
                let prompt_step = cancel_follows_prompt(db, step.domain_id,
                    step.session_id, &operation, session_id)?;
                if let Some(prompt) = prompt_step {
                    acp_cancel_step_id(&prompt)? == step.step_id
                } else { false }
            },
            _ => !has_unresolved(db, step.domain_id, step.session_id, &operation)?,
        };
        if !allowed { return Err(RpcJournalError::Unknown); }
        insert_intent(db, &step.fields(), &operation, &encoded, pending.is_some())?;
        Ok(PreparedStep { bytes: encoded, disposition: Disposition::NewWrite })
}

fn transition_acp(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    step: &AcpStep<'_>, next: Phase, error: Option<&str>) -> Result<()> {
    transact(db, |db| transition_acp_in_transaction(db, owner, step, next, error))
}

fn transition_acp_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>, next: Phase,
    error: Option<&str>) -> Result<()> {
    if next == Phase::Unknown && error.map_or(true, |value| value.is_empty()
        || value.len() > 4096) { return Err(RpcJournalError::Invalid("original error")); }
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, &step.fields())?;
        let states = if next == Phase::Unknown { &["ACTIVE", "UNKNOWN"][..] }
            else { &["ACTIVE"][..] };
        let (operation, driver, _) = assert_acp_binding(db, step, states,
            next == Phase::Unknown)?;
        let encoded = encode_acp_step(step, &driver)?;
        transition_row(db, &step.fields(), &operation, &encoded, next, error)
}

/// Call only after the exact native persistent writer returned success.
pub(crate) fn mark_acp_written(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>) -> Result<()> {
    transition_acp(db, owner, step, Phase::Written, None)
}

pub(super) fn mark_acp_written_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>) -> Result<()> {
    transition_acp_in_transaction(db, owner, step, Phase::Written, None)
}

/// Preserve the original OS/pipe error; this step must never be resent.
pub(crate) fn mark_acp_unknown(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>, original_error: &str) -> Result<()> {
    transition_acp(db, owner, step, Phase::Unknown, Some(original_error))
}

pub(super) fn mark_acp_unknown_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>, original_error: &str) -> Result<()> {
    transition_acp_in_transaction(db, owner, step, Phase::Unknown,
        Some(original_error))
}

/// A first captures this exact source. Only its correlated raw response can
/// advance WRITTEN to OBSERVED; vendor data grants no K-SESSION authority.
pub(crate) fn observe_acp_response(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>, frame: &OriginBoundFrame,
    key: &RawSourceKey) -> Result<acp::Observation> {
    transact(db, |db| observe_acp_response_in_transaction(db, owner, step, frame, key))
}

/// Decode once, then commit the exact RPC row and A no-event source in the
/// caller's transaction. The H stdin receipt may join this same commit.
pub(super) fn observe_acp_response_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &AcpStep<'_>, frame: &OriginBoundFrame,
    key: &RawSourceKey) -> Result<acp::Observation> {
    source_matches(db, frame, key, &key.operation_id, &step.fields())?;
    observe_acp_captured_response_in_transaction(db, owner, step, key)
        .map(|(observation, _)| observation)
}

/// Complete from A's durable original capture; the output loop need retain
/// only RawSourceKey, never a second pipe read or a fabricated frame.
pub(super) fn observe_acp_captured_response_in_transaction(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    step: &AcpStep<'_>, key: &RawSourceKey,
) -> Result<(acp::Observation, Vec<u8>)> {
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, &step.fields())?;
        let (operation, driver, _) = assert_acp_binding(db, step,
            &["ACTIVE", "UNKNOWN"], false)?;
        let encoded = encode_acp_step(step, &driver)?;
        if key.operation_id != operation { return Err(RpcJournalError::Denied); }
        let source = ledger::read_captured_raw_source(db, &key.operation_id,
            &key.source_epoch, &key.source_cursor)?
            .ok_or(RpcJournalError::Denied)?;
        if source.process_ticket != step.custody.ticket.opaque()
            || source.custodian_nonce != step.custody.custodian_nonce
            || source.domain_id != step.domain_id
            || source.session_id != step.session_id
            || source.generation != step.custody.binding.generation {
            return Err(RpcJournalError::Denied);
        }
    let pending = acp_pending(step)?.ok_or(RpcJournalError::Invalid("ACP notification has no ACK"))?;
    let observation = acp::decode(&source.raw_bytes, Some(&pending)).map_err(|error|
        RpcJournalError::AcpDecode { reason: error.reason, raw_frame: error.raw_frame })?;
    if let commands::AcpCommand::SetConfigOption { config_id, value, .. } = step.command {
        if matches!(&observation, acp::Observation::SessionConfigOption { .. })
            && !acp::confirms_config_value(&observation, config_id, value) {
            return Err(RpcJournalError::Denied);
        }
    }
    if !matches!(&observation, acp::Observation::Initialize { .. }
        | acp::Observation::SessionNew { .. } | acp::Observation::SessionLoad { .. }
        | acp::Observation::SessionResume { .. } | acp::Observation::SessionConfigOption { .. }
        | acp::Observation::Prompt { .. }
        | acp::Observation::RemoteError { .. }) {
        return Err(RpcJournalError::Invalid("not an ACP response"));
    }
        match (same_row(db, &step.fields(), &operation, &encoded)?, source.state) {
            (Some(Phase::Written), RawSourceState::Pending) =>
                persist_observation_and_no_event_with_reason(db, step.domain_id,
                    step.session_id, step.step_id, &operation, key, ACP_RESPONSE_NO_EVENT)?,
            (Some(Phase::Observed), RawSourceState::NoEvent)
                if source.no_event_reason.as_deref() == Some(ACP_RESPONSE_NO_EVENT)
                    && observed_source_is_exact(db, &step.fields(), &operation, key)? => {},
            _ => return Err(RpcJournalError::Conflict),
        }
    Ok((observation, source.raw_bytes))
}

/// Read back one already committed ACP ACK from its original A source. This
/// never reads stdout or writes stdin. The caller supplies the same original
/// command and typed RPC ID; the ordinary observe path rechecks every current
/// binding, original command byte, source identity, and config currentValue.
pub(crate) fn read_observed_acp_response(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    step: &AcpStep<'_>,
) -> Result<Option<(acp::Observation, Vec<u8>)>> {
    transact(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        original_open(db, &step.fields())?;
        let (operation, driver, _) = assert_acp_binding(db, step,
            &["ACTIVE", "UNKNOWN"], false)?;
        let encoded = encode_acp_step(step, &driver)?;
        if same_row(db, &step.fields(), &operation, &encoded)? != Some(Phase::Observed) {
            return Ok(None);
        }
        let q = Statement::prepare(db.as_ptr(),
            "SELECT source_epoch,source_cursor FROM main.gogoke_v37_rpc_steps
              WHERE domain_id=?1 AND session_id=?2 AND step_id=?3
                AND open_request_id=?4 AND process_operation_id=?5
                AND phase='OBSERVED' AND requires_response=1")?;
        for (index, value) in [step.domain_id, step.session_id, step.step_id,
            step.open_request_id, operation.as_str()].iter().enumerate() {
            q.bind_text((index + 1) as i32, value)?;
        }
        if !q.step_row()? { return Err(RpcJournalError::Conflict); }
        let key = RawSourceKey { operation_id: operation,
            source_epoch: q.column_text(0)?, source_cursor: q.column_text(1)? };
        if q.step_row()? { return Err(RpcJournalError::Conflict); }
        drop(q);
        observe_acp_captured_response_in_transaction(db, owner, step, &key).map(Some)
    })
}

fn claude_bytes(step: &ClaudeStep<'_>, driver: &str) -> Result<Vec<u8>> {
    if driver != "claude" { return Err(RpcJournalError::Denied); }
    let command = match step.command {
        commands::ClaudeCommand::Initialize { request_id } =>
            commands::ClaudeCommand::Initialize { request_id },
        commands::ClaudeCommand::User { uuid, text } =>
            commands::ClaudeCommand::User { uuid, text },
    };
    commands::encode_claude(command).map_err(RpcJournalError::AcpEncode)
}

fn claude_question_bound(db:&VerifiedDatabaseConnection<'_>,
    step:&ClaudeQuestionStep<'_>,operation:&str)->Result<()> {
    if step.source.operation_id!=operation ||
        step.source.source_epoch!=step.custody.custodian_nonce ||
        step.wire.is_empty() || step.wire.len()>65_536 ||
        !step.wire.ends_with(b"\n") {
        return Err(RpcJournalError::Denied);
    }
    let original=ledger::read_captured_raw_source(db,&step.source.operation_id,
        &step.source.source_epoch,&step.source.source_cursor)?
        .ok_or(RpcJournalError::Denied)?;
    if original.domain_id!=step.domain_id || original.session_id!=step.session_id
        || original.process_ticket!=step.custody.ticket.opaque()
        || original.custodian_nonce!=step.custody.custodian_nonce
        || original.generation!=step.custody.binding.generation {
        return Err(RpcJournalError::Denied);
    }
    let question=claude_question::decode(&original.raw_bytes)
        .map_err(|_|RpcJournalError::Denied)?
        .ok_or(RpcJournalError::Denied)?;
    claude_question::validate_answer_wire(&question,step.wire)
        .map_err(|_|RpcJournalError::Denied)?;
    let basis=format!("{}\n{}\n{}\n{}\n{}",step.domain_id,step.session_id,
        step.source.operation_id,step.source.source_epoch,step.source.source_cursor);
    if step.card_id!=format!("card{}",crate::store::digest::sha256_hex(basis.as_bytes())) {
        return Err(RpcJournalError::Denied);
    }
    let answer_basis=format!("{}\n{}\n{}",step.domain_id,step.session_id,
        step.answer_request_id);
    if step.step_id!=format!("qanswer{}",crate::store::digest::sha256_hex(answer_basis.as_bytes())) {
        return Err(RpcJournalError::Denied);
    }
    let q=Statement::prepare(db.as_ptr(),
        "SELECT c.vendor_request_id,c.vendor_item_id,c.question_payload,c.turn_id,
                c.vendor_thread_id,c.seat_id,c.generation,c.answer,o.answer,o.answer_kind
           FROM main.gogoke_v37_qcard_native c
           JOIN main.gogoke_v37_qcard_native_operations o
             ON o.domain_id=c.domain_id AND o.card_id=c.card_id
          WHERE c.domain_id=?1 AND c.card_id=?2 AND o.request_id=?3
            AND c.state='ANSWER_UNKNOWN' AND o.state='UNKNOWN'")?;
    q.bind_text(1,step.domain_id)?;q.bind_text(2,step.card_id)?;
    q.bind_text(3,step.answer_request_id)?;
    if !q.step_row()? {return Err(RpcJournalError::Denied);}
    let vendor_request=q.column_text(0)?;let tool_use=q.column_text(1)?;
    let payload=q.column_text(2)?;let host_turn=q.column_text(3)?;
    let vendor_session=q.column_text(4)?;let seat=q.column_text(5)?;
    let generation=q.column_text(6)?;let card_answer=q.column_text(7)?;
    let operation_answer=q.column_text(8)?;let answer_kind=q.column_text(9)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    drop(q);
    if vendor_request!=Json::String(JsonString::from_str(question.request_id())).canonical()
        || tool_use!=question.tool_use_id()
        || payload!=question.display_payload() || generation!=step.custody.binding.generation
        || answer_kind!="WIRE" || card_answer!=operation_answer
        || card_answer.as_bytes()!=&step.wire[..step.wire.len()-1] {
        return Err(RpcJournalError::Denied);
    }
    let h=Statement::prepare(db.as_ptr(),
        "SELECT h.request_hex,e.seat_id FROM main.gogoke_v37_h_stdin_journal h
           JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=h.domain_id
             AND e.session_id=h.session_id AND e.generation=h.generation
             AND e.process_operation_id=h.process_operation_id
          WHERE h.domain_id=?1 AND h.session_id=?2 AND h.request_id=?3
            AND h.process_operation_id=?4 AND h.ticket=?5
            AND h.custodian_nonce=?6 AND h.generation=?7
            AND h.operation='send' AND h.phase='PREPARED'
            AND e.phase IN ('ACTIVE','UNKNOWN')")?;
    for (index,value) in [step.domain_id,step.session_id,host_turn.as_str(),operation,
        step.custody.ticket.opaque(),step.custody.custodian_nonce.as_str(),
        step.custody.binding.generation.as_str()].iter().enumerate() {
        h.bind_text((index+1) as i32,value)?;
    }
    if !h.step_row()? {return Err(RpcJournalError::Denied);}
    let original_send=unhex(&h.column_text(0)?)?;
    if h.column_text(1)?!=seat || h.step_row()? {return Err(RpcJournalError::Denied);}
    drop(h);
    let send=super::decode_request(&original_send).map_err(|_|RpcJournalError::Denied)?;
    if send.request_id!=host_turn || send.operation!="send" ||
        send.domain_id!=step.domain_id || send.target_id!=step.session_id {
        return Err(RpcJournalError::Denied);
    }
    let send_digest=crate::store::digest::sha256_hex(&original_send);
    let send_step=format!("claude-send-{}",&send_digest[..40]);
    let send_uuid=format!("{}-{}-5{}-8{}-{}",&send_digest[..8],&send_digest[8..12],
        &send_digest[13..16],&send_digest[17..20],&send_digest[20..32]);
    let Some(Json::String(body))=send.payload.get(&JsonString::from_str("body")) else {
        return Err(RpcJournalError::Denied);
    };
    let body=body.to_well_formed_string().ok_or(RpcJournalError::Denied)?;
    let expected=commands::encode_claude(commands::ClaudeCommand::User {
        uuid:&send_uuid,text:&body,
    }).map_err(RpcJournalError::AcpEncode)?;
    let echo=Statement::prepare(db.as_ptr(),
        "SELECT source_epoch,source_cursor,command_hex FROM main.gogoke_v37_rpc_steps
         WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
           AND step_id=?4 AND ticket=?5 AND custodian_nonce=?6
           AND generation=?7 AND phase='OBSERVED' AND requires_response=1")?;
    for (index,value) in [step.domain_id,step.session_id,operation,send_step.as_str(),
        step.custody.ticket.opaque(),step.custody.custodian_nonce.as_str(),
        step.custody.binding.generation.as_str()].iter().enumerate() {
        echo.bind_text((index+1) as i32,value)?;
    }
    if !echo.step_row()? {return Err(RpcJournalError::Denied);}
    let echo_epoch=echo.column_text(0)?;
    let echo_cursor=echo.column_text(1)?;
    let echo_command=unhex(&echo.column_text(2)?)?;
    if echo.step_row()? || echo_command!=expected || echo_epoch!=step.source.source_epoch {
        return Err(RpcJournalError::Denied);
    }
    drop(echo);
    let echo_number=echo_cursor.parse::<u64>().map_err(|_|RpcJournalError::Denied)?;
    let question_number=step.source.source_cursor.parse::<u64>()
        .map_err(|_|RpcJournalError::Denied)?;
    if echo_number==0 || echo_number>=question_number ||
        echo_number.to_string()!=echo_cursor || question_number.to_string()!=step.source.source_cursor {
        return Err(RpcJournalError::Denied);
    }
    let echo_source=ledger::read_captured_raw_source(db,operation,&echo_epoch,&echo_cursor)?
        .ok_or(RpcJournalError::Denied)?;
    if echo_source.domain_id!=step.domain_id || echo_source.session_id!=step.session_id
        || echo_source.process_ticket!=step.custody.ticket.opaque() ||
        !matches!(stream_json::decode_claude_line(&echo_source.raw_bytes),
            Ok(stream_json::ClaudeData::UserReplay {session_id,uuid,text})
                if session_id==vendor_session && uuid==send_uuid && text==body) {
        return Err(RpcJournalError::Denied);
    }
    let pending=Statement::prepare(db.as_ptr(),
        "SELECT step_id,phase,requires_response FROM main.gogoke_v37_rpc_steps
          WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
            AND (phase IN ('INTENT','UNKNOWN') OR (phase='WRITTEN' AND requires_response=1))")?;
    pending.bind_text(1,step.domain_id)?;pending.bind_text(2,step.session_id)?;
    pending.bind_text(3,operation)?;
    while pending.step_row()? {
        let pending_id=pending.column_text(0)?;
        if pending_id==step.step_id {continue;}
        return Err(RpcJournalError::Unknown);
    }
    // This session ID must have come from the same A process before the
    // question. No control_request field is treated as a session identifier.
    let init=Statement::prepare(db.as_ptr(),
        "SELECT source_cursor FROM main.v37_ledger_raw_source
          WHERE operation_id=?1 AND source_epoch=?2 AND
                CAST(source_cursor AS INTEGER)<CAST(?3 AS INTEGER)
          ORDER BY CAST(source_cursor AS INTEGER)")?;
    init.bind_text(1,operation)?;init.bind_text(2,&step.source.source_epoch)?;
    init.bind_text(3,&step.source.source_cursor)?;
    let mut session_seen=false;
    while init.step_row()? {
        let source=ledger::read_captured_raw_source(db,operation,&step.source.source_epoch,
            &init.column_text(0)?)?.ok_or(RpcJournalError::Denied)?;
        if source.domain_id!=step.domain_id || source.session_id!=step.session_id
            || source.process_ticket!=step.custody.ticket.opaque() {
            return Err(RpcJournalError::Denied);
        }
        if matches!(stream_json::decode_claude_line(&source.raw_bytes),
            Ok(stream_json::ClaudeData::Init {session_id,..}) if session_id==vendor_session) {
            session_seen=true;
        }
    }
    if !session_seen {return Err(RpcJournalError::Denied);}
    Ok(())
}

pub(crate) fn prepare_claude_question(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,step:&ClaudeQuestionStep<'_>)->Result<PreparedStep> {
    transact(db,|db| {
        check_owner_in_current_transaction(db,owner)?;
        original_open(db,&step.fields())?;
        let (operation,driver)=assert_current_binding(db,&step.fields(),&["ACTIVE"],false)?;
        if driver!="claude" {return Err(RpcJournalError::Denied);}
        claude_question_bound(db,step,&operation)?;
        if let Some(phase)=same_row(db,&step.fields(),&operation,step.wire)? {
            return Ok(PreparedStep {bytes:step.wire.to_vec(),
                disposition:Disposition::Existing(phase)});
        }
        insert_intent(db,&step.fields(),&operation,step.wire,false)?;
        Ok(PreparedStep {bytes:step.wire.to_vec(),disposition:Disposition::NewWrite})
    })
}

fn transition_claude_question(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,step:&ClaudeQuestionStep<'_>,phase:Phase,error:Option<&str>)->Result<()> {
    transact(db,|db| {
        check_owner_in_current_transaction(db,owner)?;
        original_open(db,&step.fields())?;
        let states=if phase==Phase::Unknown {&["ACTIVE","UNKNOWN"][..]} else {&["ACTIVE"][..]};
        let (operation,driver)=assert_current_binding(db,&step.fields(),states,
            phase==Phase::Unknown)?;
        if driver!="claude" {return Err(RpcJournalError::Denied);}
        // The original C/A binding remains immutable between intent and the
        // physical write. UNKNOWN never grants another prepare or retry.
        claude_question_bound(db,step,&operation)?;
        transition_row(db,&step.fields(),&operation,step.wire,phase,error)
    })
}
pub(crate) fn mark_claude_question_written(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,step:&ClaudeQuestionStep<'_>)->Result<()> {
    transition_claude_question(db,owner,step,Phase::Written,None)
}
pub(crate) fn mark_claude_question_unknown(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,step:&ClaudeQuestionStep<'_>,original_error:&str)->Result<()> {
    if original_error.is_empty() || original_error.len()>4096 {
        return Err(RpcJournalError::Invalid("original error"));
    }
    transition_claude_question(db,owner,step,Phase::Unknown,Some(original_error))
}

/// Persist original Claude stdin intent before the physical writer. An
/// existing row is readback only; neither replay nor UNKNOWN permits resend.
pub(crate) fn prepare_claude(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>) -> Result<PreparedStep> {
    let (expected_step, expected_id) = claude_initialize_identity(step.open_request_bytes);
    if !matches!(step.command, commands::ClaudeCommand::Initialize { request_id }
        if *request_id == expected_id.as_str())
        || step.step_id != expected_step.as_str() {
        return Err(RpcJournalError::Invalid("Claude initialize identity"));
    }
    transact(db, |db| prepare_claude_in_transaction(db, owner, step))
}

pub(super) fn prepare_claude_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>) -> Result<PreparedStep> {
    for (value, name) in [(step.domain_id, "domain"), (step.session_id, "session"),
        (step.open_request_id, "open request"), (step.step_id, "step")] {
        if !atom(value) { return Err(RpcJournalError::Invalid(name)); }
    }
    check_owner_in_current_transaction(db, owner)?;
    original_open(db, &step.fields())?;
    let (operation, driver) = assert_current_binding(db, &step.fields(),
        &["PREPARED", "ACTIVE"], false)?;
    let encoded = claude_bytes(step, &driver)?;
    if let Some(phase) = same_row(db, &step.fields(), &operation, &encoded)? {
        return Ok(PreparedStep { bytes: encoded,
            disposition: Disposition::Existing(phase) });
    }
    if has_unresolved(db, step.domain_id, step.session_id, &operation)? {
        return Err(RpcJournalError::Unknown);
    }
    insert_intent(db, &step.fields(), &operation, &encoded, true)?;
    Ok(PreparedStep { bytes: encoded, disposition: Disposition::NewWrite })
}

fn transition_claude_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>, next: Phase,
    error: Option<&str>) -> Result<()> {
    if next == Phase::Unknown && error.map_or(true, |value| value.is_empty()
        || value.len() > 4096) { return Err(RpcJournalError::Invalid("original error")); }
    check_owner_in_current_transaction(db, owner)?;
    original_open(db, &step.fields())?;
    let states = if next == Phase::Unknown { &["ACTIVE", "UNKNOWN"][..] }
        else { &["ACTIVE"][..] };
    let (operation, driver) = assert_current_binding(db, &step.fields(), states,
        next == Phase::Unknown)?;
    let encoded = claude_bytes(step, &driver)?;
    transition_row(db, &step.fields(), &operation, &encoded, next, error)
}

pub(crate) fn mark_claude_written(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>) -> Result<()> {
    transact(db, |db| transition_claude_in_transaction(db, owner, step, Phase::Written, None))
}

pub(super) fn mark_claude_written_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>) -> Result<()> {
    transition_claude_in_transaction(db, owner, step, Phase::Written, None)
}

pub(crate) fn mark_claude_unknown(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>, original_error: &str) -> Result<()> {
    transact(db, |db| transition_claude_in_transaction(db, owner, step,
        Phase::Unknown, Some(original_error)))
}

pub(super) fn mark_claude_unknown_in_transaction(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>, original_error: &str) -> Result<()> {
    transition_claude_in_transaction(db, owner, step, Phase::Unknown, Some(original_error))
}

/// An ACK is only the matching control response or exact echoed User UUID and
/// text. Assistant/result frames cannot advance this stdin step.
pub(super) fn observe_claude_captured_ack_in_transaction(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    step: &ClaudeStep<'_>, key: &RawSourceKey,
) -> Result<(stream_json::ClaudeData, Vec<u8>)> {
    check_owner_in_current_transaction(db, owner)?;
    original_open(db, &step.fields())?;
    let (operation, driver) = assert_current_binding(db, &step.fields(),
        &["ACTIVE", "UNKNOWN"], false)?;
    let encoded = claude_bytes(step, &driver)?;
    if key.operation_id != operation { return Err(RpcJournalError::Denied); }
    let source = ledger::read_captured_raw_source(db, &key.operation_id,
        &key.source_epoch, &key.source_cursor)?.ok_or(RpcJournalError::Denied)?;
    if source.process_ticket != step.custody.ticket.opaque()
        || source.custodian_nonce != step.custody.custodian_nonce
        || source.domain_id != step.domain_id || source.session_id != step.session_id
        || source.generation != step.custody.binding.generation {
        return Err(RpcJournalError::Denied);
    }
    let observation = stream_json::decode_claude_line(&source.raw_bytes)
        .map_err(|_| RpcJournalError::Denied)?;
    let matching = match (&observation, step.command) {
        (stream_json::ClaudeData::ControlResponse { request_id: found, .. },
            commands::ClaudeCommand::Initialize { request_id }) => found.as_str() == *request_id,
        (stream_json::ClaudeData::UserReplay { uuid: found, text: echoed, .. },
            commands::ClaudeCommand::User { uuid, text }) =>
                found.as_str() == *uuid && echoed.as_str() == *text,
        _ => false,
    };
    if !matching { return Err(RpcJournalError::Denied); }
    match (same_row(db, &step.fields(), &operation, &encoded)?, source.state) {
        (Some(Phase::Written), RawSourceState::Pending) =>
            persist_observation_and_no_event_with_reason(db, step.domain_id,
                step.session_id, step.step_id, &operation, key, CLAUDE_ACK_NO_EVENT)?,
        (Some(Phase::Observed), RawSourceState::NoEvent)
            if source.no_event_reason.as_deref() == Some(CLAUDE_ACK_NO_EVENT)
                && observed_source_is_exact(db, &step.fields(), &operation, key)? => {},
        _ => return Err(RpcJournalError::Conflict),
    }
    Ok((observation, source.raw_bytes))
}

pub(crate) fn observe_claude_ack(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, step: &ClaudeStep<'_>, frame: &OriginBoundFrame,
    key: &RawSourceKey) -> Result<stream_json::ClaudeData> {
    transact(db, |db| {
        source_matches(db, frame, key, &key.operation_id, &step.fields())?;
        observe_claude_captured_ack_in_transaction(db, owner, step, key)
            .map(|(observation, _)| observation)
    })
}

pub(super) fn read_observed_claude_ack_in_transaction(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    step: &ClaudeStep<'_>,
) -> Result<Option<(stream_json::ClaudeData, Vec<u8>)>> {
    check_owner_in_current_transaction(db, owner)?;
    original_open(db, &step.fields())?;
    let (operation, driver) = assert_current_binding(db, &step.fields(),
        &["ACTIVE", "UNKNOWN"], false)?;
    let encoded = claude_bytes(step, &driver)?;
    if same_row(db, &step.fields(), &operation, &encoded)? != Some(Phase::Observed) {
        return Ok(None);
    }
    let q = Statement::prepare(db.as_ptr(),
        "SELECT source_epoch,source_cursor FROM main.gogoke_v37_rpc_steps
          WHERE domain_id=?1 AND session_id=?2 AND step_id=?3
            AND open_request_id=?4 AND process_operation_id=?5
            AND phase='OBSERVED' AND requires_response=1")?;
    for (index, value) in [step.domain_id, step.session_id, step.step_id,
        step.open_request_id, operation.as_str()].iter().enumerate() {
        q.bind_text((index + 1) as i32, value)?;
    }
    if !q.step_row()? { return Err(RpcJournalError::Conflict); }
    let key = RawSourceKey { operation_id: operation,
        source_epoch: q.column_text(0)?, source_cursor: q.column_text(1)? };
    if q.step_row()? { return Err(RpcJournalError::Conflict); }
    drop(q);
    observe_claude_captured_ack_in_transaction(db, owner, step, &key).map(Some)
}

pub(crate) fn read_observed_claude_ack(
    db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    step: &ClaudeStep<'_>,
) -> Result<Option<(stream_json::ClaudeData, Vec<u8>)>> {
    transact(db, |db| read_observed_claude_ack_in_transaction(db, owner, step))
}

/// Historical proof of the original Claude open handshake. This only reads
/// the original open intent, episode, RPC step and captured A ACK. It does not
/// grant live custody, infer a vendor session ID, or contact the process.
pub(crate) fn read_original_claude_initialize_ack(
    db: &VerifiedDatabaseConnection<'_>, domain_id: &str, session_id: &str,
    open_request_id: &str, open_request_bytes: &[u8],
    process_operation_id: &str, generation: &str,
) -> Result<Option<stream_json::ClaudeData>> {
    if !atom(domain_id) || !atom(session_id) || !atom(open_request_id)
        || !atom(process_operation_id) || !atom(generation)
        || !raw(open_request_bytes) {
        return Err(RpcJournalError::Invalid("original Claude open identity"));
    }
    let open_hex = hex(open_request_bytes);
    let original = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_operation
          WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
            AND raw_hex=?4 AND operation='open' AND status IN ('APPLIED','UNKNOWN')")?;
    for (index, value) in [domain_id, open_request_id, session_id,
        open_hex.as_str()].iter().enumerate() {
        original.bind_text((index + 1) as i32, value)?;
    }
    if !original.step_row()? || original.step_row()? {
        return Err(RpcJournalError::Denied);
    }
    drop(original);
    let (step_id, request_id) = claude_initialize_identity(open_request_bytes);
    let expected = commands::encode_claude(commands::ClaudeCommand::Initialize {
        request_id: &request_id,
    }).map_err(RpcJournalError::AcpEncode)?;
    let observed = Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes)
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
            AND c.generation=e.generation
           JOIN main.gogoke_v37_rpc_steps s
             ON s.domain_id=e.domain_id AND s.session_id=e.session_id
            AND s.open_request_id=e.request_id
            AND s.process_operation_id=e.process_operation_id
            AND s.generation=e.generation
            AND s.ticket=c.ticket AND s.custodian_nonce=c.custodian_nonce
           JOIN main.v37_ledger_raw_source r
             ON r.operation_id=s.process_operation_id
            AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
            AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
            AND r.domain_id=s.domain_id AND r.session_id=s.session_id
            AND r.generation=s.generation
          WHERE e.domain_id=?1 AND e.session_id=?2 AND e.request_id=?3
            AND e.raw_hex=?4 AND e.process_operation_id=?5
            AND e.generation=?6 AND e.old_generation IS NULL
            AND i.driver_id='claude' AND i.version='2.1.196'
            AND ((e.phase='STOPPED' AND c.state='STOPPED'
                  AND e.stop_fact_id IS NOT NULL
                  AND e.stop_fact_id=c.stop_proof_hash)
              OR (e.phase IN ('ACTIVE','UNKNOWN')
                  AND c.state IN ('ACTIVE','UNKNOWN')))
            AND s.step_id=?7 AND s.phase='OBSERVED' AND s.requires_response=1
            AND r.state='NO_EVENT' AND r.no_event_reason=?8")?;
    for (index, value) in [domain_id, session_id, open_request_id,
        open_hex.as_str(), process_operation_id, generation, step_id.as_str(),
        CLAUDE_ACK_NO_EVENT].iter().enumerate() {
        observed.bind_text((index + 1) as i32, value)?;
    }
    if !observed.step_row()? { return Ok(None); }
    let command = unhex(&observed.column_text(0)?)?;
    let response = unhex(&observed.column_text(1)?)?;
    if observed.step_row()? { return Err(RpcJournalError::Conflict); }
    if command != expected { return Err(RpcJournalError::Denied); }
    let observation = stream_json::decode_claude_line(&response)
        .map_err(|_| RpcJournalError::Denied)?;
    if !matches!(&observation, stream_json::ClaudeData::ControlResponse {
        request_id: found, .. } if found == &request_id) {
        return Err(RpcJournalError::Denied);
    }
    Ok(Some(observation))
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

/// The original compact command's ACK proves submission only. Its durable A
/// response is checked with the exact command and RPC ID, never interpreted
/// as a compaction-completed event or permission to change generation.
pub(crate) fn observed_compact_ack(db:&VerifiedDatabaseConnection<'_>,
    domain:&str,session:&str,operation:&str,generation:&str,ticket:&str,
    nonce:&str,step_id:&str,thread:&str)->Result<Option<RawSourceKey>> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes),s.source_epoch,s.source_cursor
           FROM main.gogoke_v37_rpc_steps s
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id
             AND r.generation=s.generation
          WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
            AND s.generation=?4 AND s.ticket=?5 AND s.custodian_nonce=?6
            AND s.step_id=?7 AND s.phase='OBSERVED' AND s.requires_response=1
            AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
    for (index,value) in [domain,session,operation,generation,ticket,nonce,step_id].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Ok(None);}
    let command=unhex(&q.column_text(0)?)?;
    let response=unhex(&q.column_text(1)?)?;
    let epoch=q.column_text(2)?;let cursor=q.column_text(3)?;
    if q.step_row()? {return Err(RpcJournalError::Conflict);}
    let id=stored_compact_id(&command,thread)?;
    let expected=Command::ThreadCompactStart {thread_id:thread.to_owned()};
    match codex_rpc::decode(&response,Some((&id,&expected)))? {
        Reply::Ack {..}=>Ok(Some(RawSourceKey {operation_id:operation.to_owned(),
            source_epoch:epoch,source_cursor:cursor})),
        Reply::RemoteError {raw_frame,..}=>Err(RpcJournalError::Codec(
            codex_rpc::RpcError::RemoteResponse(raw_frame))),
        _=>Err(RpcJournalError::Conflict),
    }
}

fn stored_compact_id(command:&[u8],thread:&str)->Result<RpcId> {
    let body=command.strip_suffix(b"\n").ok_or(RpcJournalError::Invalid("compact command LF"))?;
    let Json::Object(fields)=Parser::parse(std::str::from_utf8(body).map_err(|_|RpcJournalError::Invalid("compact command UTF-8"))?)? else {
        return Err(RpcJournalError::Invalid("compact command object"));
    };
    let Some(Json::Number(number))=fields.get(&JsonString::from_str("id")) else {
        return Err(RpcJournalError::Invalid("compact command ID"));
    };
    let number=number.parse::<u64>().map_err(|_|RpcJournalError::Invalid("compact command ID"))?;
    let id=RpcId::client(number)?;
    let expected=Command::ThreadCompactStart {thread_id:thread.to_owned()};
    if expected.encode(Some(&id))?!=command {return Err(RpcJournalError::Conflict);}
    Ok(id)
}

/// Settle a command already physically WRITTEN from its one captured A reply.
/// The original writer step remains the only send authority.
/// A delayed response to an original ordinary send is already captured before
/// this safe point. Reconcile that original WRITTEN step without any pipe IO.
pub(crate) fn reconcile_written_sends_from_a(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,domain:&str,session:&str,generation:&str)->Result<()> {
    let pending=Statement::prepare(db.as_ptr(),
        "SELECT request_id,ticket FROM main.gogoke_v37_h_stdin_journal
         WHERE domain_id=?1 AND session_id=?2 AND generation=?3
           AND operation IN ('send','append-without-turn') AND phase IN ('PREPARED','UNKNOWN')")?;
    pending.bind_text(1,domain)?;pending.bind_text(2,session)?;pending.bind_text(3,generation)?;
    let mut keys=Vec::new();while pending.step_row()? {keys.push((pending.column_text(0)?,pending.column_text(1)?));}
    drop(pending);
    for (request,ticket) in keys {
        transact(db,|db| {
            check_owner_in_current_transaction(db,owner)?;
            let key=super::journal::StdinJournalKey {domain_id:domain,request_id:&request,
                session_id:session,ticket:&ticket,generation};
            let record=super::journal::read_stdin_journal(db,&key)
                .map_err(|error|RpcJournalError::Authority(crate::store::orchestration::OrchestrationError::V37StoreFailure(
                    format!("original H send read: {error:?}"))))?.ok_or(RpcJournalError::Denied)?;
            if record.state==super::journal::JournalState::Receipted {return Ok(());}
            let step_id=format!("{}-{}",if record.operation=="append-without-turn" {"append"} else {"send"},
                &crate::store::digest::sha256_hex(&record.request_bytes)[..40]);
            let step=Statement::prepare(db.as_ptr(),
                "SELECT command_hex FROM main.gogoke_v37_rpc_steps
                 WHERE domain_id=?1 AND session_id=?2 AND process_operation_id=?3
                   AND generation=?4 AND ticket=?5 AND custodian_nonce=?6
                   AND step_id=?7 AND phase='WRITTEN' AND requires_response=1")?;
            for (index,value) in [domain,session,record.process_operation_id.as_str(),generation,
                ticket.as_str(),record.custodian_nonce.as_str(),step_id.as_str()].iter().enumerate() {
                step.bind_text((index+1) as i32,value)?;
            }
            if !step.step_row()? {return Ok(());}
            let bytes=unhex(&step.column_text(0)?)?;
            if step.step_row()? {return Err(RpcJournalError::Conflict);}
            let (id,command)=if record.operation=="append-without-turn" {
                codex_rpc::decode_stored_append(&bytes)?
            } else {codex_rpc::decode_stored_turn_start(&bytes)?};
            let source=Statement::prepare(db.as_ptr(),
                "SELECT source_epoch,source_cursor,hex(raw_bytes) FROM main.v37_ledger_raw_source
                 WHERE operation_id=?1 AND domain_id=?2 AND session_id=?3
                   AND generation=?4 AND process_ticket=?5 AND custodian_nonce=?6
                   AND state='PENDING' ORDER BY CAST(source_cursor AS INTEGER)")?;
            for (index,value) in [record.process_operation_id.as_str(),domain,session,generation,
                ticket.as_str(),record.custodian_nonce.as_str()].iter().enumerate() {
                source.bind_text((index+1) as i32,value)?;
            }
            let mut found=None;
            while source.step_row()? {
                let bytes=unhex(&source.column_text(2)?)?;
                let matching=match codex_rpc::decode(&bytes,Some((&id,&command))) {
                    Ok(Reply::Turn {..})=>matches!(&command,Command::TurnStart {..}),
                    Ok(Reply::Ack {..})=>matches!(&command,Command::AppendWithoutTurn {..}),
                    Ok(Reply::RemoteError {..})=>true,
                    _=>false,
                };
                if matching {
                    if found.is_some() {return Err(RpcJournalError::Conflict);}
                    found=Some(RawSourceKey {operation_id:record.process_operation_id.clone(),
                        source_epoch:source.column_text(0)?,source_cursor:source.column_text(1)?});
                }
            }
            if let Some(key)=found {persist_observation_and_no_event(db,domain,session,&step_id,
                &record.process_operation_id,&key)?;}
            Ok(())
        })?;
    }
    Ok(())
}

pub(crate) fn reconcile_written_compact_from_a(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,domain:&str,session:&str,operation:&str,generation:&str,
    ticket:&str,nonce:&str,step_id:&str,thread:&str)->Result<bool> {
    transact(db,|db| {
        check_owner_in_current_transaction(db,owner)?;
        let step=Statement::prepare(db.as_ptr(),
            "SELECT s.command_hex FROM main.gogoke_v37_rpc_steps s
               JOIN main.gogoke_v37_h_process_episode e
                 ON e.process_operation_id=s.process_operation_id AND e.domain_id=s.domain_id
                 AND e.session_id=s.session_id AND e.generation=s.generation
               JOIN main.gogoke_coordination_process_custody c
                 ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
                 AND c.generation=e.generation AND c.ticket=s.ticket
                 AND c.custodian_nonce=s.custodian_nonce
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.process_operation_id=?3
                AND s.generation=?4 AND s.ticket=?5 AND s.custodian_nonce=?6
                AND s.step_id=?7 AND s.phase='WRITTEN' AND s.requires_response=1
                AND e.phase='ACTIVE' AND c.state IN ('ACTIVE','UNKNOWN')")?;
        for (index,value) in [domain,session,operation,generation,ticket,nonce,step_id].iter().enumerate() {
            step.bind_text((index+1) as i32,value)?;
        }
        if !step.step_row()? {return Ok(false);}
        let command=unhex(&step.column_text(0)?)?;
        if step.step_row()? {return Err(RpcJournalError::Conflict);}
        let id=stored_compact_id(&command,thread)?;
        let expected=Command::ThreadCompactStart {thread_id:thread.to_owned()};
        let source=Statement::prepare(db.as_ptr(),
            "SELECT source_epoch,source_cursor,hex(raw_bytes)
               FROM main.v37_ledger_raw_source
              WHERE operation_id=?1 AND domain_id=?2 AND session_id=?3
                AND generation=?4 AND process_ticket=?5 AND custodian_nonce=?6
                AND state='PENDING' ORDER BY CAST(source_cursor AS INTEGER)")?;
        for (index,value) in [operation,domain,session,generation,ticket,nonce].iter().enumerate() {
            source.bind_text((index+1) as i32,value)?;
        }
        let mut found=None;
        while source.step_row()? {
            let bytes=unhex(&source.column_text(2)?)?;
            if matches!(codex_rpc::decode(&bytes,Some((&id,&expected))),
                Ok(Reply::Ack {..}|Reply::RemoteError {..})) {
                if found.is_some() {return Err(RpcJournalError::Conflict);}
                found=Some(RawSourceKey {operation_id:operation.to_owned(),
                    source_epoch:source.column_text(0)?,source_cursor:source.column_text(1)?});
            }
        }
        let Some(key)=found else {return Ok(false)};
        persist_observation_and_no_event(db,domain,session,step_id,operation,&key)?;
        Ok(true)
    })
}

/// Completion is a later original A notification from the old physical
/// episode, after the request's source watermark. The prior empty ACK never
/// satisfies this proof. A duplicate matching item is ambiguous and denied.
pub(crate) fn observed_compaction_completion(db:&VerifiedDatabaseConnection<'_>,
    domain:&str,session:&str,operation:&str,generation:&str,ticket:&str,
    nonce:&str,thread:&str,watermark:i64)->Result<Option<(RawSourceKey,String)>> {
    let q=Statement::prepare(db.as_ptr(),
        "SELECT r.source_epoch,r.source_cursor,hex(r.raw_bytes)
           FROM main.v37_ledger_raw_source r
           JOIN main.gogoke_v37_h_process_episode e
             ON e.process_operation_id=r.operation_id AND e.domain_id=r.domain_id
             AND e.session_id=r.session_id AND e.generation=r.generation
           JOIN main.gogoke_coordination_process_custody c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation AND c.ticket=r.process_ticket
             AND c.custodian_nonce=r.custodian_nonce
          WHERE r.operation_id=?1 AND r.domain_id=?2 AND r.session_id=?3
            AND r.generation=?4 AND r.process_ticket=?5 AND r.custodian_nonce=?6
            AND r.state='RESOLVED' AND CAST(r.source_cursor AS INTEGER)>?7
            AND ((e.phase='ACTIVE' AND c.state IN ('ACTIVE','UNKNOWN'))
              OR (e.phase='STOPPED' AND c.state='STOPPED'
                  AND e.stop_fact_id=c.stop_proof_hash AND e.stop_fact_id IS NOT NULL))
          ORDER BY CAST(r.source_cursor AS INTEGER)")?;
    for (index,value) in [operation,domain,session,generation,ticket,nonce].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    q.bind_i64(7,watermark)?;
    let mut found=None;
    while q.step_row()? {
        let bytes=unhex(&q.column_text(2)?)?;
        if let Ok(Reply::CompactionItem {thread_id,item_id,..})=codex_rpc::decode(&bytes,None) {
            if thread_id!=thread {continue;}
            if found.is_some() {return Err(RpcJournalError::Conflict);}
            found=Some((RawSourceKey {operation_id:operation.to_owned(),
                source_epoch:q.column_text(0)?,source_cursor:q.column_text(1)?},item_id));
        }
    }
    Ok(found)
}

/// H can check a notification's A source without altering an in-flight RPC.
pub(crate) fn observe_event(
    db: &VerifiedDatabaseConnection<'_>,
    frame: &OriginBoundFrame,
    key: &RawSourceKey,
    step: &Step<'_>,
) -> Result<Reply> {
    let operation = assert_native_binding(db, step, &["ACTIVE"], false)?;
    source_matches(db, frame, key, &operation, &step.fields())?;
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
        super::super::admission::initialize_admission_schema(&mut db).unwrap();
        db.execute("CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT,ticket TEXT,custodian_nonce TEXT,domain_id TEXT,generation TEXT,state TEXT,stop_proof_hash TEXT) STRICT").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_owner_binding(binding_id,instance_id,domain_id,kind,owner_id,generation,state) VALUES('binding','instance','domain','SESSION','session','1','ACTIVE')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('domain','session','instance','home','binding','1','COMMITTED',1,'operation')").unwrap();
        db.execute("INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,phase) VALUES('domain','open','session','1','7b7d0a',0,1,'operation','instance','home','binding','ACTIVE')").unwrap();
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
        // Synthetic source-link control: ACK is NO_EVENT and cannot prove
        // completion; only a later RESOLVED native item from the same old
        // process, ticket, nonce, generation and thread can do so.
        for (cursor,thread) in [("2","other-thread"),("3","thread-a")] {
            let frame=format!("{{\"method\":\"item/completed\",\"params\":{{\"threadId\":\"{thread}\",\"item\":{{\"type\":\"contextCompaction\",\"id\":\"item-{cursor}\"}}}}}}\n");
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,
                   domain_id,session_id,generation,source_epoch,source_cursor,raw_bytes,state,
                   resolved_event_id) VALUES('operation','ticket','nonce','domain','session','1',
                   'epoch',?1,?2,'RESOLVED',?3)").unwrap();
            insert.bind_text(1,cursor).unwrap();insert.bind_blob(2,frame.as_bytes()).unwrap();
            insert.bind_text(3,&format!("event-{cursor}")).unwrap();insert.step_done().unwrap();
        }
        assert!(observed_compaction_completion(&db,"domain","session","operation","1",
            "ticket","nonce","thread-a",3).unwrap().is_none(),"source watermark excludes earlier item");
        let completed=observed_compaction_completion(&db,"domain","session","operation","1",
            "ticket","nonce","thread-a",1).unwrap().unwrap();
        assert_eq!(completed.0.source_cursor,"3");assert_eq!(completed.1,"item-3");
        assert!(observed_compaction_completion(&db,"domain","session","operation","1",
            "ticket","nonce","other-thread",3).unwrap().is_none());
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

    #[test]
    fn acp_cancel_binds_to_one_pending_prompt_and_stable_step_id() {
        let _guard = route_b_test_guard();
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-acp-cancel-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        initialize_schema(&mut db).unwrap();
        assert!(cancel_follows_prompt(&db, "domain", "session", "operation", "native-a").unwrap().is_none());
        let prompt = b"{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"session/prompt\",\"params\":{\"sessionId\":\"native-a\"}}\n";
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,
             process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,
             binary_digest,profile_id,generation,command_hex,requires_response,phase)
             VALUES('domain','session','open',?1,'operation','ticket','nonce','1','2',
             'image','digest','profile','1',?2,?3,'WRITTEN')").unwrap();
        insert.bind_text(1, "prompt").unwrap();
        insert.bind_text(2, &hex(prompt)).unwrap();
        insert.bind_i64(3, 1).unwrap();
        insert.step_done().unwrap();
        assert_eq!(cancel_follows_prompt(&db, "domain", "session", "operation", "native-a").unwrap().as_deref(), Some("prompt"));
        assert!(cancel_follows_prompt(&db, "domain", "session", "operation", "native-b").unwrap().is_none());
        drop(insert);
        let cancel = b"{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"native-a\"}}\n";
        let cancel_step = acp_cancel_step_id("prompt").unwrap();
        assert_eq!(cancel_step, acp_cancel_step_id("prompt").unwrap());
        assert_ne!(cancel_step, acp_cancel_step_id("later-prompt").unwrap());
        let second = Statement::prepare(db.as_ptr(),
            "INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,
             process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,
             binary_digest,profile_id,generation,command_hex,requires_response,phase)
             VALUES('domain','session','open',?1,'operation','ticket','nonce','1','2',
             'image','digest','profile','1',?2,0,'WRITTEN')").unwrap();
        second.bind_text(1, &cancel_step).unwrap();
        second.bind_text(2, &hex(cancel)).unwrap();
        second.step_done().unwrap();
        assert_eq!(cancel_follows_prompt(&db, "domain", "session", "operation", "native-a").unwrap().as_deref(), Some("prompt"));
        drop(second);
        db.execute("UPDATE gogoke_v37_rpc_steps SET phase='OBSERVED',source_epoch='epoch',
            source_cursor='1' WHERE step_id='prompt'").unwrap();
        let later = Statement::prepare(db.as_ptr(),
            "INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,
             process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,
             binary_digest,profile_id,generation,command_hex,requires_response,phase)
             VALUES('domain','session','open','later-prompt','operation','ticket','nonce','1','2',
             'image','digest','profile','1',?1,1,'WRITTEN')").unwrap();
        later.bind_text(1, &hex(prompt)).unwrap();
        later.step_done().unwrap();
        assert_eq!(cancel_follows_prompt(&db, "domain", "session", "operation", "native-a").unwrap().as_deref(), Some("later-prompt"));
        drop(later);
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
}
