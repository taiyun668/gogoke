//! Owner-authored instance presentation and source-bound observations.
//! A missing profile or observation stays unknown; registration never invents
//! an enabled state, account, model, or default capacity.

use super::registry::valid_id;
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::atomic::{Json,JsonString,Parser};
use crate::store::session_transport::codex_rpc;
use std::collections::BTreeSet;

#[derive(Debug)]
pub(crate) enum InstanceManagementError {
    Store(OrchestrationError),
    Invalid(&'static str),
    Conflict,
}
impl From<OrchestrationError> for InstanceManagementError {
    fn from(error: OrchestrationError) -> Self { Self::Store(error) }
}
impl From<crate::store::atomic::AtomicError> for InstanceManagementError {
    fn from(error: crate::store::atomic::AtomicError) -> Self {
        Self::Store(OrchestrationError::Atomic(error))
    }
}
impl From<crate::store::same_open::SameOpenError> for InstanceManagementError {
    fn from(error: crate::store::same_open::SameOpenError) -> Self {
        Self::Store(OrchestrationError::Atomic(error.into()))
    }
}

pub(crate) struct InstanceProfile {
    pub(crate) instance_id: String,
    pub(crate) driver_id: String,
    pub(crate) display_name: Option<String>,
    pub(crate) enabled: Option<bool>,
    pub(crate) connected_model_source: Option<String>,
    pub(crate) revision: Option<i64>,
}

pub(crate) struct InstanceEvidence {
    pub(crate) instance_id: String,
    pub(crate) masked_account: Option<String>,
    pub(crate) subscription: Option<String>,
    pub(crate) account_confirmed_at: Option<String>,
    pub(crate) account_source: Option<String>,
    pub(crate) available_models_json: Option<String>,
    pub(crate) models_source: Option<String>,
    pub(crate) models_observed_at: Option<String>,
    pub(crate) detect_error: Option<String>,
    pub(crate) detect_error_at: Option<String>,
}

/// Optional fields recovered from the same original Codex model/list pages
/// that supplied `available_models_json`. Absence remains absence; an E scope
/// or a vendor default cannot manufacture a supported effort.
#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct VerifiedModelEffort {
    pub(crate) model: String,
    pub(crate) default_effort: Option<String>,
    pub(crate) supported_efforts: Option<Vec<String>>,
}

#[derive(Clone,Debug,Eq,PartialEq)]
pub(crate) struct VerifiedModelEfforts {
    pub(crate) instance_id: String,
    pub(crate) models_source: String,
    pub(crate) models_observed_at: String,
    pub(crate) models: Vec<VerifiedModelEffort>,
}

fn optional_text(row: &Statement, index: i32) -> Result<Option<String>, InstanceManagementError> {
    let value = row.column_text(index)?;
    Ok(if value.is_empty() { None } else { Some(value) })
}

fn valid_label(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 160 &&
        !value.chars().any(char::is_control)
}

fn valid_source(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 160 &&
        !value.chars().any(char::is_control)
}

fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>,
    action: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<T, InstanceManagementError>)
    -> Result<T, InstanceManagementError> {
    db.execute("BEGIN IMMEDIATE")?;
    match action(db) {
        Ok(value) => {
            db.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
            Ok(value)
        },
        Err(error) => {
            db.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        },
    }
}

/// First profile creation and later changes use the same Owner issuer and
/// revision CAS. `None` is only valid for the first profile write.
pub(crate) fn set_instance_profile(db: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    instance_id: &str, display_name: &str, enabled: bool,
    connected_model_source: Option<&str>, expected_revision: Option<i64>)
    -> Result<i64, InstanceManagementError> {
    if !valid_id(instance_id) || !valid_label(display_name) ||
        connected_model_source.is_some_and(|value| !valid_source(value)) ||
        expected_revision.is_some_and(|value| value < 1 || value == i64::MAX) {
        return Err(InstanceManagementError::Invalid("instance profile"));
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let instance = Statement::prepare(db.as_ptr(),
            "SELECT driver_id FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
        instance.bind_text(1, instance_id)?;
        if !instance.step_row()? { return Err(InstanceManagementError::Conflict); }
        let driver = instance.column_text(0)?;
        if instance.step_row()? || (driver != "opencode" && connected_model_source.is_some()) ||
            (driver == "opencode" && connected_model_source.is_none()) {
            return Err(InstanceManagementError::Invalid("connected model source"));
        }
        let prior = Statement::prepare(db.as_ptr(),
            "SELECT revision FROM main.gogoke_v37_instance_profiles WHERE instance_id=?1 AND tombstoned=0")?;
        prior.bind_text(1, instance_id)?;
        let observed = if prior.step_row()? {
            Some(prior.column_text(0)?.parse::<i64>()
                .map_err(|_| InstanceManagementError::Conflict)?)
        } else { None };
        if prior.step_row()? || observed != expected_revision {
            return Err(InstanceManagementError::Conflict);
        }
        let revision = observed.map_or(1, |value| value + 1);
        let sql = if observed.is_some() {
            "UPDATE main.gogoke_v37_instance_profiles SET display_name=?1,enabled=?2,connected_model_source=?3,revision=?4 WHERE instance_id=?5 AND revision=?6 AND tombstoned=0"
        } else {
            "INSERT INTO main.gogoke_v37_instance_profiles(display_name,enabled,connected_model_source,revision,instance_id) VALUES(?1,?2,?3,?4,?5)"
        };
        let write = Statement::prepare(db.as_ptr(), sql)?;
        write.bind_text(1, display_name)?;
        write.bind_i64(2, i64::from(enabled))?;
        if let Some(source) = connected_model_source { write.bind_text(3, source)?; }
        write.bind_i64(4, revision)?;
        write.bind_text(5, instance_id)?;
        if let Some(previous) = observed { write.bind_i64(6, previous)?; }
        write.step_done()?;
        Ok(revision)
    })
}

pub(crate) fn read_instance_profiles(db: &VerifiedDatabaseConnection<'_>)
    -> Result<Vec<InstanceProfile>, InstanceManagementError> {
    let rows = Statement::prepare(db.as_ptr(), "SELECT i.instance_id,i.driver_id,COALESCE(p.display_name,''),COALESCE(CAST(p.enabled AS TEXT),''),COALESCE(p.connected_model_source,''),COALESCE(CAST(p.revision AS TEXT),'') FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE p.tombstoned IS NULL OR p.tombstoned=0 ORDER BY i.instance_id")?;
    let mut out = Vec::new();
    while rows.step_row()? {
        let enabled = rows.column_text(3)?;
        if !matches!(enabled.as_str(), "" | "0" | "1") { return Err(InstanceManagementError::Conflict); }
        out.push(InstanceProfile { instance_id: rows.column_text(0)?, driver_id: rows.column_text(1)?,
            display_name: optional_text(&rows, 2)?, enabled: if enabled.is_empty() { None } else { Some(enabled == "1") },
            connected_model_source: optional_text(&rows, 4)?,
            revision: optional_text(&rows, 5)?.map(|value| value.parse()
                .map_err(|_| InstanceManagementError::Conflict)).transpose()? });
    }
    Ok(out)
}

/// Logical deletion retains the original physical home, credentials, and
/// history. It cannot be undone by profile creation or registration replay.
pub(crate) fn tombstone_unused_instance(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, instance_id: &str, expected_revision: i64)
    -> Result<(), InstanceManagementError> {
    if !valid_id(instance_id) || expected_revision < 1 || expected_revision == i64::MAX {
        return Err(InstanceManagementError::Invalid("delete instance"));
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let current = Statement::prepare(db.as_ptr(),
            "SELECT revision,tombstoned FROM main.gogoke_v37_instance_profiles WHERE instance_id=?1")?;
        current.bind_text(1, instance_id)?;
        if !current.step_row()? || current.column_text(0)? != expected_revision.to_string() ||
            current.column_text(1)? != "0" || current.step_row()? {
            return Err(InstanceManagementError::Conflict);
        }
        // Any assigned seat, unsettled session/holder, nested home, pending
        // operation, or credential namespace transition refuses deletion.
        for sql in [
            "SELECT 1 FROM main.gogoke_v37_seats WHERE instance_id=?1 LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND state!='RELEASED' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_h_owner_binding WHERE instance_id=?1 AND state='ACTIVE' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_h_process_episode e LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE e.instance_id=?1 AND (e.phase NOT IN ('STOPPED','FAILED') OR (e.process_operation_id IS NOT NULL AND (e.stop_fact_id IS NULL OR c.state IS NULL OR c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR e.stop_fact_id!=c.stop_proof_hash))) LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_instance_homes WHERE instance_id=?1 AND state NOT IN ('CLEANED','CLOSED') LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_instance_histories WHERE instance_id=?1 AND state!='READY' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase!='APPLIED' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 AND state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 AND state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1",
        ] {
            let busy = Statement::prepare(db.as_ptr(), sql)?;
            busy.bind_text(1, instance_id)?;
            if busy.step_row()? { return Err(InstanceManagementError::Conflict); }
        }
        let write = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_profiles SET enabled=0,tombstoned=1,revision=revision+1 WHERE instance_id=?1 AND revision=?2 AND tombstoned=0")?;
        write.bind_text(1, instance_id)?;
        write.bind_i64(2, expected_revision)?;
        write.step_done()?;
        Ok(())
    })
}

pub(crate) enum QualifiedAccountSource {
    CodexAccountRead,
    ClaudeAuthStatus,
    OpenCodeAuthList,
    GrokAuthHeading,
}
impl QualifiedAccountSource {
    fn key(&self) -> &'static str { match self {
        Self::CodexAccountRead => "codex-account-read",
        Self::ClaudeAuthStatus => "claude-auth-status",
        Self::OpenCodeAuthList => "opencode-auth-list",
        Self::GrokAuthHeading => "grok-auth-heading",
    }}
    fn driver(&self) -> &'static str { match self {
        Self::CodexAccountRead => "codex", Self::ClaudeAuthStatus => "claude",
        Self::OpenCodeAuthList => "opencode", Self::GrokAuthHeading => "grok",
    }}
}

/// Produced only by the host's exact qualified login-status route. A missing
/// email remains unknown while an independently present plan can be retained.
/// Neither field is a credential or token; no file is inspected.
pub(crate) struct QualifiedAccount<'a> {
    pub(crate) instance_id: &'a str,
    pub(crate) source: QualifiedAccountSource,
    pub(crate) account_label: Option<&'a str>,
    pub(crate) subscription: Option<&'a str>,
    pub(crate) confirmed_at: &'a str,
}

fn mask_account(label: &str) -> Result<String, InstanceManagementError> {
    if !valid_source(label) || label.len() > 256 { return Err(InstanceManagementError::Invalid("account label")); }
    let (local, domain) = label.split_once('@').ok_or(InstanceManagementError::Invalid("account label"))?;
    if local.is_empty() || domain.is_empty() || domain.contains('@') ||
        !domain.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-')) {
        return Err(InstanceManagementError::Invalid("account label"));
    }
    let first = local.chars().next().ok_or(InstanceManagementError::Invalid("account label"))?;
    Ok(format!("{first}***@{domain}"))
}

pub(crate) fn record_qualified_account(db: &mut VerifiedDatabaseConnection<'_>,
    observation: &QualifiedAccount<'_>) -> Result<(), InstanceManagementError> {
    if !valid_id(observation.instance_id) || !valid_source(observation.confirmed_at) ||
        (observation.account_label.is_none() && observation.subscription.is_none()) ||
        observation.subscription.is_some_and(|value| !valid_source(value)) {
        return Err(InstanceManagementError::Invalid("qualified account"));
    }
    let masked = observation.account_label.map(mask_account).transpose()?;
    transaction(db, |db| {
        let row = Statement::prepare(db.as_ptr(),
            "SELECT driver_id,login_state FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
        row.bind_text(1, observation.instance_id)?;
        if !row.step_row()? || row.column_text(0)? != observation.source.driver() ||
            row.column_text(1)? != "LOGGED_IN" || row.step_row()? {
            return Err(InstanceManagementError::Conflict);
        }
        let write = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_instance_evidence(instance_id,account_masked,subscription,account_confirmed_at,account_source) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(instance_id) DO UPDATE SET account_masked=excluded.account_masked,subscription=excluded.subscription,account_confirmed_at=excluded.account_confirmed_at,account_source=excluded.account_source,available_models_json=NULL,models_source=NULL,models_observed_at=NULL,models_program_digest=NULL")?;
        write.bind_text(1, observation.instance_id)?;
        if let Some(masked)=masked.as_deref() {write.bind_text(2, masked)?;}
        if let Some(subscription) = observation.subscription { write.bind_text(3, subscription)?; }
        write.bind_text(4, observation.confirmed_at)?;
        write.bind_text(5, observation.source.key())?;
        write.step_done()?;
        Ok(())
    })
}

pub(crate) fn read_instance_evidence(db: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<Option<InstanceEvidence>, InstanceManagementError> {
    if !valid_id(instance_id) { return Err(InstanceManagementError::Invalid("instance id")); }
    let row = Statement::prepare(db.as_ptr(), "SELECT CASE WHEN i.login_state='LOGGED_IN' THEN COALESCE(e.account_masked,'') ELSE '' END,CASE WHEN i.login_state='LOGGED_IN' THEN COALESCE(e.subscription,'') ELSE '' END,CASE WHEN i.login_state='LOGGED_IN' THEN COALESCE(e.account_confirmed_at,'') ELSE '' END,CASE WHEN i.login_state='LOGGED_IN' THEN COALESCE(e.account_source,'') ELSE '' END,CASE WHEN e.models_program_digest=i.program_digest AND i.login_state='LOGGED_IN' THEN COALESCE(e.available_models_json,'') ELSE '' END,CASE WHEN e.models_program_digest=i.program_digest AND i.login_state='LOGGED_IN' THEN COALESCE(e.models_source,'') ELSE '' END,CASE WHEN e.models_program_digest=i.program_digest AND i.login_state='LOGGED_IN' THEN COALESCE(e.models_observed_at,'') ELSE '' END,COALESCE(e.detect_error,''),COALESCE(e.detect_error_at,'') FROM main.gogoke_v37_instance_evidence e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE e.instance_id=?1")?;
    row.bind_text(1, instance_id)?;
    if !row.step_row()? { return Ok(None); }
    let result = InstanceEvidence { instance_id: instance_id.to_owned(),
        masked_account: optional_text(&row, 0)?, subscription: optional_text(&row, 1)?,
        account_confirmed_at: optional_text(&row, 2)?, account_source: optional_text(&row, 3)?,
        available_models_json: optional_text(&row, 4)?, models_source: optional_text(&row, 5)?,
        models_observed_at: optional_text(&row, 6)?, detect_error: optional_text(&row, 7)?,
        detect_error_at: optional_text(&row, 8)? };
    if row.step_row()? { return Err(InstanceManagementError::Conflict); }
    Ok(Some(result))
}

fn original_hex_bytes(hex:&str)->Result<Vec<u8>,InstanceManagementError> {
    if hex.len()%2!=0 || hex.len()>2*1024*1024 {return Err(InstanceManagementError::Conflict);}
    hex.as_bytes().chunks_exact(2).map(|pair| {
        let text=std::str::from_utf8(pair).map_err(|_|InstanceManagementError::Conflict)?;
        u8::from_str_radix(text,16).map_err(|_|InstanceManagementError::Conflict)
    }).collect()
}

fn valid_wire_id(value:&str)->bool {
    let bytes=value.as_bytes();
    !bytes.is_empty() && bytes.len()<=128 && bytes[0].is_ascii_alphabetic()
        && bytes[1..].iter().all(|byte|byte.is_ascii_alphanumeric() || matches!(*byte,b'_'|b'-'))
}

fn original_effort(value:&Json)->Result<String,InstanceManagementError> {
    let Json::String(value)=value else {return Err(InstanceManagementError::Conflict)};
    let effort=value.to_well_formed_string().ok_or(InstanceManagementError::Conflict)?;
    if effort.is_empty() || effort.len()>64 || effort.chars().any(char::is_control) {
        return Err(InstanceManagementError::Conflict);
    }
    Ok(effort)
}

fn original_model_efforts_page(response:&[u8],slugs:&[String])
    ->Result<Vec<VerifiedModelEffort>,InstanceManagementError> {
    let raw=std::str::from_utf8(response).map_err(|_|InstanceManagementError::Conflict)?;
    let Json::Object(frame)=Parser::parse(raw).map_err(|_|InstanceManagementError::Conflict)?
        else {return Err(InstanceManagementError::Conflict)};
    let Some(Json::Object(result))=frame.get(&JsonString::from_str("result"))
        else {return Err(InstanceManagementError::Conflict)};
    let Some(Json::Array(data))=result.get(&JsonString::from_str("data"))
        else {return Err(InstanceManagementError::Conflict)};
    let mut seen=BTreeSet::new();
    let mut models=Vec::new();
    for item in data {
        let Json::Object(fields)=item else {return Err(InstanceManagementError::Conflict)};
        let Some(Json::String(slug))=fields.get(&JsonString::from_str("model"))
            else {return Err(InstanceManagementError::Conflict)};
        let slug=slug.to_well_formed_string().ok_or(InstanceManagementError::Conflict)?;
        let Some(Json::Bool(hidden))=fields.get(&JsonString::from_str("hidden"))
            else {return Err(InstanceManagementError::Conflict)};
        if *hidden || !seen.insert(slug.clone()) {continue;}
        let default_effort=fields.get(&JsonString::from_str("defaultReasoningEffort"))
            .map(original_effort).transpose()?;
        let supported_efforts=match fields.get(&JsonString::from_str("supportedReasoningEfforts")) {
            None=>None,
            Some(Json::Array(options)) if options.len()<=32=>{
                let mut values=Vec::new();let mut unique=BTreeSet::new();
                for option in options {
                    let Json::Object(option)=option else {return Err(InstanceManagementError::Conflict)};
                    let effort=original_effort(option.get(&JsonString::from_str("reasoningEffort"))
                        .ok_or(InstanceManagementError::Conflict)?)?;
                    if !unique.insert(effort.clone()) {return Err(InstanceManagementError::Conflict);}
                    values.push(effort);
                }
                Some(values)
            },
            _=>return Err(InstanceManagementError::Conflict),
        };
        if default_effort.as_ref().is_some_and(|default|
            supported_efforts.as_ref().is_some_and(|options|!options.contains(default))) {
            return Err(InstanceManagementError::Conflict);
        }
        models.push(VerifiedModelEffort {model:slug,default_effort,supported_efforts});
    }
    if models.iter().map(|row|&row.model).ne(slugs.iter()) {
        return Err(InstanceManagementError::Conflict);
    }
    Ok(models)
}

/// Re-read only the original H/A pages named by F's existing source marker.
/// This introduces no second model cache and works for observations recorded
/// before effort projection existed. The old slug list remains authoritative.
pub(crate) fn read_verified_model_efforts(db:&VerifiedDatabaseConnection<'_>,instance_id:&str)
    ->Result<Option<VerifiedModelEfforts>,InstanceManagementError> {
    let Some(evidence)=read_instance_evidence(db,instance_id)? else {return Ok(None)};
    let (Some(slugs),Some(source),Some(observed_at))=(evidence.available_models_json,
        evidence.models_source,evidence.models_observed_at) else {return Ok(None)};
    let Some(identity)=source.strip_prefix("codex-model/list:OBSERVED:") else {return Ok(None)};
    let parts:Vec<_>=identity.split(':').collect();
    if parts.len()!=4 || !valid_wire_id(parts[0]) || !valid_wire_id(parts[1])
        || !valid_wire_id(parts[2]) {return Ok(None)};
    let first_cursor=parts[3].parse::<u64>().ok().filter(|value|*value>0)
        .ok_or(InstanceManagementError::Conflict)?;
    let Json::Array(cached)=Parser::parse(&slugs).map_err(|_|InstanceManagementError::Conflict)?
        else {return Err(InstanceManagementError::Conflict)};
    let cached:Vec<String>=cached.iter().map(|item| {
        let Json::String(value)=item else {return Err(InstanceManagementError::Conflict)};
        value.to_well_formed_string().ok_or(InstanceManagementError::Conflict)
    }).collect::<Result<_,_>>()?;
    let rows=Statement::prepare(db.as_ptr(),
        "SELECT s.command_hex,hex(r.raw_bytes),s.source_cursor,s.process_operation_id
           FROM main.gogoke_v37_rpc_steps s
           JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=s.domain_id
             AND e.session_id=s.session_id AND e.generation=s.generation
             AND e.process_operation_id=s.process_operation_id AND e.instance_id=?1
           JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id
             AND i.driver_id='codex' AND i.login_state='LOGGED_IN'
             AND i.program_digest=s.binary_digest
           JOIN main.gogoke_coordination_process_custody c ON c.operation_id=s.process_operation_id
             AND c.ticket=s.ticket AND c.custodian_nonce=s.custodian_nonce
             AND c.pid=s.pid AND c.creation_time_100ns=s.creation_time
             AND c.image_path=s.image_path AND c.binary_digest_sha256=s.binary_digest
             AND c.profile_id=e.instance_id AND c.domain_id=s.domain_id
             AND c.generation=s.generation
           JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
             AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
             AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
             AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
             AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'
          WHERE s.domain_id=?2 AND s.session_id=?3 AND s.source_epoch=?4
            AND CAST(s.source_cursor AS INTEGER)>=?5 AND s.phase='OBSERVED'
            AND s.requires_response=1
            AND instr(lower(s.command_hex),'6d6f64656c2f6c697374')>0
          ORDER BY CAST(s.source_cursor AS INTEGER),s.step_id")?;
    rows.bind_text(1,instance_id)?;rows.bind_text(2,parts[0])?;
    rows.bind_text(3,parts[1])?;rows.bind_text(4,parts[2])?;
    rows.bind_i64(5,i64::try_from(first_cursor).map_err(|_|InstanceManagementError::Conflict)?)?;
    let mut expected_cursor=None;
    let mut prior_source_cursor=0;
    let mut process=None;
    let mut models=Vec::new();
    let mut seen=BTreeSet::new();
    let mut pages=0usize;
    let mut complete=false;
    while rows.step_row()? {
        pages+=1;
        if pages>64 || models.len()>1024 {return Err(InstanceManagementError::Conflict);}
        let command=original_hex_bytes(&rows.column_text(0)?)?;
        let response=original_hex_bytes(&rows.column_text(1)?)?;
        let cursor=rows.column_text(2)?.parse::<u64>()
            .map_err(|_|InstanceManagementError::Conflict)?;
        let current_process=rows.column_text(3)?;
        if cursor<=prior_source_cursor || (prior_source_cursor==0 && cursor!=first_cursor)
            || process.as_ref().is_some_and(|prior|prior!=&current_process) {
            return Err(InstanceManagementError::Conflict);
        }
        prior_source_cursor=cursor;process=Some(current_process);
        let (request_cursor,page,next)=codex_rpc::decode_stored_model_list(&command,&response)
            .map_err(|_|InstanceManagementError::Conflict)?;
        if request_cursor!=expected_cursor {return Err(InstanceManagementError::Conflict);}
        for model in original_model_efforts_page(&response,&page)? {
            if seen.insert(model.model.clone()) {models.push(model);}
        }
        expected_cursor=next;
        if expected_cursor.is_none() {complete=true;break;}
    }
    if !complete || models.iter().map(|model|&model.model).ne(cached.iter()) {
        return Err(InstanceManagementError::Conflict);
    }
    Ok(Some(VerifiedModelEfforts {instance_id:instance_id.into(),
        models_source:source,models_observed_at:observed_at,models}))
}

/// Only H's OBSERVED original model/list RPC pages can update this cache.
/// The supplied step IDs select records; they never supply model names or a
/// response. Every page must be in the same exact process/claim/pin and the
/// opaque cursor chain must terminate before any cache write.
pub(crate) fn record_verified_models_from_original_rpc_source(
    db:&mut VerifiedDatabaseConnection<'_>,instance_id:&str,domain_id:&str,
    session_id:&str,step_ids:&[String],observed_at:&str)
    ->Result<(),InstanceManagementError> {
    if !valid_id(instance_id) || !valid_wire_id(domain_id) || !valid_wire_id(session_id)
        || step_ids.is_empty() || step_ids.len()>64 || observed_at.parse::<u64>().is_err()
        || !step_ids.iter().all(|step|valid_wire_id(step)) {
        return Err(InstanceManagementError::Invalid("model source identity"));
    }
    transaction(db,|db| {
        let mut distinct_steps=BTreeSet::new();
        let mut distinct_models=BTreeSet::new();
        let mut models=Vec::new();
        let mut expected_cursor=None;
        let mut first_source=None;
        let mut prior_source_cursor=None;
        let mut prior_process=None;
        let mut pin_digest=None;
        for (index,step_id) in step_ids.iter().enumerate() {
            if !distinct_steps.insert(step_id) {return Err(InstanceManagementError::Conflict);}
            let row=Statement::prepare(db.as_ptr(),
                "SELECT s.command_hex,hex(r.raw_bytes),s.source_epoch,s.source_cursor,s.process_operation_id,i.program_digest
                   FROM main.gogoke_v37_rpc_steps s
                   JOIN main.gogoke_v37_h_claim h ON h.domain_id=s.domain_id AND h.session_id=s.session_id
                     AND h.instance_id=?1 AND h.process_operation_id=s.process_operation_id
                     AND h.generation=s.generation AND h.state='COMMITTED'
                   JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id
                     AND i.driver_id='codex' AND i.login_state='LOGGED_IN'
                     AND i.program_digest=s.binary_digest
                   JOIN main.gogoke_coordination_process_custody c ON c.operation_id=s.process_operation_id
                     AND c.ticket=s.ticket AND c.custodian_nonce=s.custodian_nonce
                     AND c.pid=s.pid AND c.creation_time_100ns=s.creation_time
                     AND c.image_path=s.image_path AND c.binary_digest_sha256=s.binary_digest
                     AND c.profile_id=h.instance_id AND c.domain_id=s.domain_id
                     AND c.generation=s.generation AND c.state='ACTIVE'
                   JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                     AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                     AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                     AND r.domain_id=s.domain_id AND r.session_id=s.session_id AND r.generation=s.generation
                     AND r.state='NO_EVENT' AND r.no_event_reason='CODEX_RPC_RESPONSE'
                  WHERE s.domain_id=?2 AND s.session_id=?3 AND s.step_id=?4 AND s.phase='OBSERVED'
                    AND s.requires_response=1")?;
            row.bind_text(1,instance_id)?;row.bind_text(2,domain_id)?;
            row.bind_text(3,session_id)?;row.bind_text(4,step_id)?;
            if !row.step_row()? {return Err(InstanceManagementError::Conflict);}
            let command=original_hex_bytes(&row.column_text(0)?)?;
            let response=original_hex_bytes(&row.column_text(1)?)?;
            let epoch=row.column_text(2)?;
            let cursor=row.column_text(3)?;
            let cursor_number=cursor.parse::<u64>().map_err(|_|InstanceManagementError::Conflict)?;
            if cursor_number==0 {return Err(InstanceManagementError::Conflict);}
            let process=row.column_text(4)?;
            let digest=row.column_text(5)?;
            if row.step_row()? {return Err(InstanceManagementError::Conflict);}
            drop(row);
            if prior_process.as_deref().is_some_and(|previous|previous!=process)
                || pin_digest.as_deref().is_some_and(|previous|previous!=digest) {
                return Err(InstanceManagementError::Conflict);
            }
            prior_process=Some(process);pin_digest=Some(digest);
            if index==0 {first_source=Some((epoch.clone(),cursor.clone()));}
            else if first_source.as_ref().is_none_or(|(first_epoch,_)|first_epoch!=&epoch)
                || prior_source_cursor.is_none_or(|previous|previous>=cursor_number) {
                return Err(InstanceManagementError::Conflict);
            }
            prior_source_cursor=Some(cursor_number);
            let (request_cursor,page,next)=codex_rpc::decode_stored_model_list(&command,&response)
                .map_err(|_|InstanceManagementError::Conflict)?;
            if request_cursor!=expected_cursor || (index+1<step_ids.len())!=next.is_some() {
                return Err(InstanceManagementError::Conflict);
            }
            for model in page {
                if model.len()>160 || model.chars().any(char::is_control) || models.len()>=1024 {
                    return Err(InstanceManagementError::Conflict);
                }
                if distinct_models.insert(model.clone()) {models.push(model);}
            }
            expected_cursor=next;
        }
        let (epoch,cursor)=first_source.ok_or(InstanceManagementError::Conflict)?;
        let source=format!("codex-model/list:OBSERVED:{domain_id}:{session_id}:{epoch}:{cursor}");
        let json=Json::Array(models.iter().map(|value|Json::String(JsonString::from_str(value))).collect()).canonical();
        if json.len()>65_536 {return Err(InstanceManagementError::Conflict);}
        let write=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_evidence(instance_id,available_models_json,models_source,models_observed_at,models_program_digest)
             VALUES(?1,?2,?3,?4,?5) ON CONFLICT(instance_id) DO UPDATE SET available_models_json=excluded.available_models_json,
             models_source=excluded.models_source,models_observed_at=excluded.models_observed_at,models_program_digest=excluded.models_program_digest")?;
        write.bind_text(1,instance_id)?;write.bind_text(2,&json)?;write.bind_text(3,&source)?;
        write.bind_text(4,observed_at)?;
        write.bind_text(5,&pin_digest.ok_or(InstanceManagementError::Conflict)?)?;
        write.step_done()?;
        Ok(())
    })
}

#[cfg(all(test, windows))]
#[test]
fn qualified_account_observation_invalidates_prior_models_even_when_mask_collides() {
    use crate::root::RootLock;
    use crate::store::same_open::{create_new,route_b_test_guard};
    let _guard=route_b_test_guard();
    let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-account-model-invalidate-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root=RootLock::acquire(&path).unwrap();
    let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
    db.execute("PRAGMA foreign_keys=ON").unwrap();
    super::initialize_schema(&mut db).unwrap();
    db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    db.execute("INSERT INTO main.gogoke_v37_instance_evidence(instance_id,account_masked,subscription,account_confirmed_at,account_source,available_models_json,models_source,models_observed_at,models_program_digest) VALUES('instanceA','a***@example.test','plus','100','codex-account-read','[\"old-model\"]','codex-model/list:OBSERVED:old','101','sha256:test')").unwrap();
    assert_eq!(read_instance_evidence(&db,"instanceA").unwrap().unwrap().available_models_json.as_deref(),Some("[\"old-model\"]"));
    record_qualified_account(&mut db,&QualifiedAccount {instance_id:"instanceA",
        source:QualifiedAccountSource::CodexAccountRead,
        account_label:Some("another@example.test"),subscription:Some("pro"),confirmed_at:"102"}).unwrap();
    let current=read_instance_evidence(&db,"instanceA").unwrap().unwrap();
    assert_eq!(current.masked_account.as_deref(),Some("a***@example.test"));
    assert_eq!(current.subscription.as_deref(),Some("pro"));
    assert!(current.available_models_json.is_none()&&current.models_source.is_none()
        &&current.models_observed_at.is_none());
    db.close_checked().unwrap();
    drop(root);
    std::fs::remove_dir_all(path).unwrap();
}

#[cfg(all(test, windows))]
#[test]
fn original_model_efforts_follow_the_cached_source_and_current_pin() {
    use crate::root::RootLock;
    use crate::store::same_open::{create_new,route_b_test_guard};
    let _guard=route_b_test_guard();
    let nonce=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let path=std::env::temp_dir().join(format!("gogoke-model-effort-source-{}-{nonce}",std::process::id()));
    std::fs::create_dir(&path).unwrap();
    let root=RootLock::acquire(&path).unwrap();
    let mut db=create_new(&root,&path.join("state.sqlite")).unwrap();
    db.execute("PRAGMA foreign_keys=ON").unwrap();
    super::initialize_schema(&mut db).unwrap();
    db.execute("CREATE TABLE gogoke_v37_rpc_steps(domain_id TEXT,session_id TEXT,step_id TEXT,source_epoch TEXT,source_cursor TEXT,process_operation_id TEXT,ticket TEXT,custodian_nonce TEXT,pid TEXT,creation_time TEXT,image_path TEXT,binary_digest TEXT,profile_id TEXT,generation TEXT,command_hex TEXT,phase TEXT,requires_response INTEGER) STRICT").unwrap();
    db.execute("CREATE TABLE gogoke_v37_h_process_episode(domain_id TEXT,session_id TEXT,generation TEXT,process_operation_id TEXT,instance_id TEXT) STRICT").unwrap();
    db.execute("CREATE TABLE gogoke_coordination_process_custody(operation_id TEXT,ticket TEXT,custodian_nonce TEXT,pid TEXT,creation_time_100ns TEXT,image_path TEXT,binary_digest_sha256 TEXT,profile_id TEXT,domain_id TEXT,generation TEXT,state TEXT) STRICT").unwrap();
    db.execute("CREATE TABLE v37_ledger_raw_source(operation_id TEXT,source_epoch TEXT,source_cursor TEXT,process_ticket TEXT,custodian_nonce TEXT,domain_id TEXT,session_id TEXT,generation TEXT,state TEXT,no_event_reason TEXT,raw_bytes BLOB) STRICT").unwrap();
    db.execute("INSERT INTO gogoke_v37_instances VALUES('instanceA','codex','homeA','identityA','sha256:pin','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
    db.execute("INSERT INTO gogoke_v37_instance_evidence(instance_id,available_models_json,models_source,models_observed_at,models_program_digest) VALUES('instanceA','[\"gpt-6.1-sol\"]','codex-model/list:OBSERVED:global:sessionA:pcn1_demo:10','100','sha256:pin')").unwrap();
    db.execute("INSERT INTO gogoke_v37_h_process_episode VALUES('global','sessionA','1','processA','instanceA')").unwrap();
    db.execute("INSERT INTO gogoke_coordination_process_custody VALUES('processA','ticketA','nonceA','41','42','programA','sha256:pin','instanceA','global','1','STOPPED')").unwrap();
    let command=codex_rpc::Command::ModelList {cursor:None}
        .encode(Some(&codex_rpc::RpcId::client(4).unwrap())).unwrap();
    let response=br#"{"id":4,"result":{"data":[{"model":"gpt-6.1-sol","hidden":false,"defaultReasoningEffort":"low","supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"}]},{"model":"hidden","hidden":true}],"nextCursor":null}}"#;
    let hex=|bytes:&[u8]|bytes.iter().map(|byte|format!("{byte:02x}")).collect::<String>();
    db.execute(&format!("INSERT INTO gogoke_v37_rpc_steps VALUES('global','sessionA','modelsA','pcn1_demo','10','processA','ticketA','nonceA','41','42','programA','sha256:pin','instanceA','1','{}','OBSERVED',1)",hex(&command))).unwrap();
    db.execute(&format!("INSERT INTO v37_ledger_raw_source VALUES('processA','pcn1_demo','10','ticketA','nonceA','global','sessionA','1','NO_EVENT','CODEX_RPC_RESPONSE',X'{}')",hex(response))).unwrap();
    let original=read_verified_model_efforts(&db,"instanceA").unwrap().unwrap();
    assert_eq!(original.models,vec![VerifiedModelEffort {model:"gpt-6.1-sol".into(),
        default_effort:Some("low".into()),supported_efforts:Some(vec!["low".into(),"high".into()])}]);
    db.execute("UPDATE gogoke_v37_instances SET program_digest='sha256:changed' WHERE instance_id='instanceA'").unwrap();
    assert!(read_verified_model_efforts(&db,"instanceA").unwrap().is_none());
    db.execute("UPDATE gogoke_v37_instances SET program_digest='sha256:pin' WHERE instance_id='instanceA'").unwrap();
    let without=br#"{"id":4,"result":{"data":[{"model":"gpt-6.1-sol","hidden":false}],"nextCursor":null}}"#;
    db.execute(&format!("UPDATE v37_ledger_raw_source SET raw_bytes=X'{}'",hex(without))).unwrap();
    let unknown=read_verified_model_efforts(&db,"instanceA").unwrap().unwrap();
    assert_eq!(unknown.models[0].default_effort,None);
    assert_eq!(unknown.models[0].supported_efforts,None);
    db.execute("UPDATE gogoke_v37_instance_evidence SET models_source='codex-model/list:OBSERVED:global:sessionA:pcn1_demo:11' WHERE instance_id='instanceA'").unwrap();
    assert!(matches!(read_verified_model_efforts(&db,"instanceA"),Err(InstanceManagementError::Conflict)));
    db.close_checked().unwrap();drop(root);std::fs::remove_dir_all(path).unwrap();
}
