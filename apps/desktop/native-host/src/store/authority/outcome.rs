//! Native Owner-override outcome append, on the existing product record store.
//! This is not an objective/independent scorer, a new truth store or an IPC grant.
//! Other label producers and service ingress remain separate qualification work.
use super::bootstrap::OwnerIssuer;
use super::catalog::current_profile;
use super::model::{denied, identifier, revision};
use super::decision_replay::validate_public_decision;
use super::transaction::{self, Result, Transaction};
use super::super::atomic::{DomainRecordInput, DomainRecordReceipt};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

#[derive(Clone, Debug)]
pub(crate) struct OutcomeVersionRef {
    pub revision: String,
    pub content_hash: String,
}

/// Comparison inputs, not authority. The actor is the separate private capability.
#[derive(Clone, Debug)]
pub(crate) struct OwnerOutcomeAppend {
    pub domain_id: String,
    pub operation_id: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
    pub policy_revision: String,
    pub revocation_head: String,
    pub decision_version: String,
    pub decision_hash: String,
    /// Explicitly the existing ActionStore operation identity/digest, not a new ID.
    pub action_operation_id: String,
    pub action_digest: String,
    pub previous: Option<OutcomeVersionRef>,
    pub canonical_outcome: Vec<u8>,
}

struct OutcomeIdentity {
    outcome_id: String,
    decision_id: String,
    action_id: String,
    revision: u64,
}

fn require_hash(value: &str) -> Result<()> {
    if value.len() != 71 || !value.starts_with("sha256:")
        || !value.as_bytes()[7..].iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b)) {
        return denied();
    }
    Ok(())
}

// Paths are fixed internal literals, never interpolated from a request.
fn json_text(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<String> {
    let rows = tx.query("SELECT json_type(?,?),json_extract(?,?)", &[json,path,json,path], 2)?;
    if rows.len() != 1 || rows[0][0] != "text" { return denied(); }
    Ok(rows[0][1].clone())
}

fn outcome_identity(tx: &mut Transaction<'_, '_>, bytes: &[u8]) -> Result<OutcomeIdentity> {
    if bytes.is_empty() || bytes.len() > 262_144 { return denied(); }
    let json = std::str::from_utf8(bytes).map_err(|_| OrchestrationError::AccessDenied)?;
    let valid = tx.query("SELECT json_valid(?)", &[json], 1)?;
    if valid.len() != 1 || valid[0][0] != "1" { return denied(); }
    const FIELDS: [&str; 13] = ["outcomeId","decisionId","actionId","revision","labelSource","evidenceRefs",
        "observationWindow","censorStatus","quality","cost","latency","rework","safetyEvents"];
    let fields = tx.query("SELECT key,type FROM json_each(?)", &[json], 2)?;
    let mut seen = std::collections::HashSet::new();
    if fields.len() != FIELDS.len() { return denied(); }
    for field in fields {
        if !FIELDS.contains(&field[0].as_str()) || !seen.insert(field[0].clone()) { return denied(); }
    }
    let outcome_id = json_text(tx,json,"$.outcomeId")?;
    let decision_id = json_text(tx,json,"$.decisionId")?;
    let action_id = json_text(tx,json,"$.actionId")?;
    for id in [&outcome_id,&decision_id,&action_id] { identifier(id)?; }
    let number = revision(&json_text(tx,json,"$.revision")?)?;
    if number == 0 || json_text(tx,json,"$.labelSource")? != "OWNER_OVERRIDE" { return denied(); }
    let status = json_text(tx,json,"$.censorStatus")?;
    if !matches!(status.as_str(),"PENDING"|"OBSERVED"|"CORRECTED"|"CENSORED") { return denied(); }
    let arrays = tx.query("SELECT json_type(?,'$.evidenceRefs'),json_type(?,'$.safetyEvents')", &[json,json], 2)?;
    if arrays.len() != 1 || arrays[0][0] != "array" || arrays[0][1] != "array" { return denied(); }
    let evidence = tx.query("SELECT value,type FROM json_each(?,'$.evidenceRefs')", &[json], 2)?;
    if evidence.is_empty() && matches!(status.as_str(),"OBSERVED"|"CORRECTED") { return denied(); }
    let mut refs = std::collections::HashSet::new();
    for entry in evidence {
        if entry[1] != "text" || !refs.insert(entry[0].clone()) { return denied(); }
        identifier(&entry[0])?;
    }
    Ok(OutcomeIdentity { outcome_id, decision_id, action_id, revision: number })
}

fn read_object(tx: &mut Transaction<'_, '_>, domain: &str, kind: &str, id: &str, version: &str) -> Result<(String,String)> {
    let rows = tx.query(
        "SELECT content_hash,CAST(canonical_json AS TEXT) FROM gogoke_objects WHERE domain_id=? AND object_type=? AND object_id=? AND object_version=?",
        &[domain,kind,id,version], 2)?;
    if rows.len() != 1 { return denied(); }
    let row = &rows[0];
    require_hash(&row[0])?;
    if content_hash(row[1].as_bytes()) != row[0] { return denied(); }
    Ok((row[0].clone(),row[1].clone()))
}

pub(super) fn apply_owner_override_outcome(
    tx: &mut Transaction<'_, '_>, actor: &OwnerIssuer, request: &OwnerOutcomeAppend,
) -> Result<DomainRecordReceipt> {
    for value in [&request.domain_id,&request.operation_id,&request.event_id,&request.receipt_id,&request.action_operation_id] {
        identifier(value)?;
    }
    revision(&request.policy_revision)?; revision(&request.revocation_head)?; revision(&request.decision_version)?;
    require_hash(&request.decision_hash)?; require_hash(&request.action_digest)?;
    let profile = current_profile(tx)?;
    actor.check(&profile)?;
    if profile.root_identity != tx.root_identity() || request.policy_revision != profile.policy_revision
        || request.revocation_head != profile.revocation_head { return denied(); }
    let identity = outcome_identity(tx,&request.canonical_outcome)?;
    // No guessing between product action IDs, reservation IDs and native IDs.
    // This slice requires the Decision to explicitly name its ActionStore operation.
    if identity.action_id != request.action_operation_id { return denied(); }
    let (decision_hash,decision) = read_object(tx,&request.domain_id,"DecisionRecord",&identity.decision_id,&request.decision_version)?;
    validate_public_decision(tx,&decision)?;
    if decision_hash != request.decision_hash || json_text(tx,&decision,"$.decisionId")? != identity.decision_id
        || json_text(tx,&decision,"$.actionId")? != request.action_operation_id
        || json_text(tx,&decision,"$.state")? != "COMMITTED" { return denied(); }
    let action = tx.query("SELECT semantic_digest FROM gogoke_action_reservations WHERE operation_id=?", &[&request.action_operation_id], 1)?;
    if action.len() != 1 || action[0][0] != request.action_digest { return denied(); }

    let previous_hash = match &request.previous {
        None if identity.revision == 1 => None,
        Some(previous) if revision(&previous.revision)?.checked_add(1) == Some(identity.revision) => {
            require_hash(&previous.content_hash)?;
            let (hash,body) = read_object(tx,&request.domain_id,"OutcomeRecord",&identity.outcome_id,&previous.revision)?;
            if hash != previous.content_hash { return denied(); }
            let prior = outcome_identity(tx,body.as_bytes())?;
            if prior.revision.to_string() != previous.revision || prior.outcome_id != identity.outcome_id
                || prior.decision_id != identity.decision_id || prior.action_id != identity.action_id { return denied(); }
            Some(hash)
        }
        _ => return denied(),
    };
    let replay = tx.query("SELECT object_type,object_id,object_version FROM gogoke_receipts WHERE domain_id=? AND operation_id=?",
        &[&request.domain_id,&request.operation_id], 3)?;
    if replay.is_empty() {
        // Full decimal ordering, not a signed SQLite integer cast. Existing stream
        // compare-and-swap below additionally serializes competing appends.
        let head = tx.query("SELECT object_version,content_hash FROM gogoke_objects WHERE domain_id=? AND object_type='OutcomeRecord' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",
            &[&request.domain_id,&identity.outcome_id], 2)?;
        match (&request.previous, head.first()) {
            (None,None) => {}
            (Some(expected),Some(current)) if current[0] == expected.revision && current[1] == expected.content_hash => {}
            _ => return Err(OrchestrationError::OperationConflict),
        }
    } else if replay.len() != 1 || replay[0][0] != "OutcomeRecord" || replay[0][1] != identity.outcome_id
        || replay[0][2] != identity.revision.to_string() {
        return Err(OrchestrationError::OperationConflict);
    }
    // Identifiers and hashes above use the native authority's restricted syntax;
    // object keys are sorted to retain the existing core canonical byte contract.
    let previous_json = previous_hash.map(|hash|format!("\"{hash}\"")).unwrap_or_else(||"null".into());
    let event = format!("{{\"actionId\":\"{}\",\"decisionId\":\"{}\",\"outcomeId\":\"{}\",\"previousHash\":{},\"revision\":\"{}\"}}",
        identity.action_id,identity.decision_id,identity.outcome_id,previous_json,identity.revision);
    let receipt = format!("{{\"actionDigest\":\"{}\",\"decisionHash\":\"{}\",\"decisionVersion\":\"{}\",\"issuerId\":\"{}\",\"policyRevision\":\"{}\",\"principalId\":\"{}\",\"revocationHead\":\"{}\",\"seatId\":\"{}\"}}",
        request.action_digest,request.decision_hash,request.decision_version,profile.issuer_id,profile.policy_revision,
        actor.principal_id(),profile.revocation_head,actor.seat_id());
    tx.apply_domain_record(DomainRecordInput {
        domain_id: request.domain_id.clone(), object_type: "OutcomeRecord".into(), object_id: identity.outcome_id.clone(),
        object_version: identity.revision.to_string(), object_bytes: request.canonical_outcome.clone(), native_identity: None,
        event_id: request.event_id.clone(), stream_id: format!("gogoke.outcome.v1/{}",identity.outcome_id),
        expected_previous_counter: identity.revision.checked_sub(2).map(|number|number.to_string()),
        counter: (identity.revision-1).to_string(), event_type: "OutcomeAppended".into(),
        occurred_at: request.recorded_at.clone(), event_bytes: event.into_bytes(), receipt_id: request.receipt_id.clone(),
        operation_id: request.operation_id.clone(), receipt_type: "OwnerOutcomeAppend".into(),
        recorded_at: request.recorded_at.clone(), receipt_bytes: receipt.into_bytes(),
    })
}

/// Native private Owner capability only; no caller-supplied Boolean, generic IPC,
/// model label, cached authorization or automatic action replay is admitted.
pub(crate) fn append_owner_override_outcome(
    connection: &mut VerifiedDatabaseConnection<'_>, actor: &OwnerIssuer, request: &OwnerOutcomeAppend,
) -> Result<DomainRecordReceipt> {
    transaction::run(connection,|tx|apply_owner_override_outcome(tx,actor,request))
}
