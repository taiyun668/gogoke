//! Owner-authored instance presentation and source-bound observations.
//! A missing profile or observation stays unknown; registration never invents
//! an enabled state, account, model, or default capacity.

use super::registry::valid_id;
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;

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

/// Produced only by the host's exact qualified login-status route. The value
/// is an account label, never a credential or token. No file is inspected.
pub(crate) struct QualifiedAccount<'a> {
    pub(crate) instance_id: &'a str,
    pub(crate) source: QualifiedAccountSource,
    pub(crate) account_label: &'a str,
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
        observation.subscription.is_some_and(|value| !valid_source(value)) {
        return Err(InstanceManagementError::Invalid("qualified account"));
    }
    let masked = mask_account(observation.account_label)?;
    transaction(db, |db| {
        let row = Statement::prepare(db.as_ptr(),
            "SELECT driver_id,login_state FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
        row.bind_text(1, observation.instance_id)?;
        if !row.step_row()? || row.column_text(0)? != observation.source.driver() ||
            row.column_text(1)? != "LOGGED_IN" || row.step_row()? {
            return Err(InstanceManagementError::Conflict);
        }
        let write = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_instance_evidence(instance_id,account_masked,subscription,account_confirmed_at,account_source) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(instance_id) DO UPDATE SET account_masked=excluded.account_masked,subscription=excluded.subscription,account_confirmed_at=excluded.account_confirmed_at,account_source=excluded.account_source")?;
        write.bind_text(1, observation.instance_id)?;
        write.bind_text(2, &masked)?;
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
    let row = Statement::prepare(db.as_ptr(), "SELECT COALESCE(e.account_masked,''),COALESCE(e.subscription,''),COALESCE(e.account_confirmed_at,''),COALESCE(e.account_source,''),CASE WHEN e.models_program_digest=i.program_digest THEN COALESCE(e.available_models_json,'') ELSE '' END,CASE WHEN e.models_program_digest=i.program_digest THEN COALESCE(e.models_source,'') ELSE '' END,CASE WHEN e.models_program_digest=i.program_digest THEN COALESCE(e.models_observed_at,'') ELSE '' END,COALESCE(e.detect_error,''),COALESCE(e.detect_error_at,'') FROM main.gogoke_v37_instance_evidence e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE e.instance_id=?1")?;
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
