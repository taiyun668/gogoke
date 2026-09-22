//! Atomic native Owner promotion. No SQL/issuer capability is exported over IPC.
//! This first storage-connected slice supports explicit same-domain, Owner-private
//! promotion. Cross-domain graph wiring and wider sharing remain NOT_QUALIFIED;
//! the source is never looked up by inferring its domain from the destination.
use super::bootstrap::OwnerIssuer;
use super::catalog::current_profile;
use super::model::denied;
use super::promotion::{authorize_owner_promotion, PromotionRequest};
use super::transaction::{self, Result, Transaction};
use super::super::context::{ContextCommand, ContextReceipt};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

const RECEIPT_SCHEMA: &str = include_str!("promotion_receipts.sql");

fn ensure_receipt_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let rows = tx.query(
        "SELECT type,sql FROM sqlite_schema WHERE name='gogoke_context_promotion_authorizations'",
        &[], 2)?;
    if rows.is_empty() {
        tx.write(RECEIPT_SCHEMA.trim(), &[])?;
    } else if rows.len() != 1 || rows[0][0] != "table"
        || rows[0][1].trim() != RECEIPT_SCHEMA.trim() {
        return denied();
    }
    if !tx.query(
        "SELECT name FROM sqlite_schema WHERE type='trigger' AND tbl_name='gogoke_context_promotion_authorizations' LIMIT 1",
        &[], 1)?.is_empty() { return denied(); }
    Ok(())
}

fn matches_target(target: &ContextCommand, request: &PromotionRequest) -> Result<()> {
    // The existing graph stores same-domain edges. Do not silently redirect an
    // explicitly cross-domain source lookup to a matching destination ID.
    if request.source_domain_id != request.destination_domain_id {
        return Err(OrchestrationError::Invalid("cross-domain promotion graph NOT_QUALIFIED"));
    }
    let source_ref = format!("{}@{}", request.source_context_id, request.source_version);
    if target.scope != "GLOBAL" || request.destination_scope != "GLOBAL"
        || target.domain_id != request.destination_domain_id
        || target.visibility != "OWNER_PRIVATE" || !target.read_grant_refs.is_empty()
        || target.derived_from != [source_ref.clone()]
        || !target.supersedes.is_empty() {
        return denied();
    }
    let evidence = target.promotion.as_ref().ok_or(OrchestrationError::AccessDenied)?;
    if evidence.source_version_ref != source_ref
        || evidence.source_grant_ref != request.source_grant.grant_id
        || evidence.target_grant_ref != request.target_grant.grant_id
        || evidence.provenance_refs != request.provenance_refs {
        return denied();
    }
    Ok(())
}

fn request_fingerprint(actor: &OwnerIssuer, request: &PromotionRequest) -> String {
    // Length-delimited components preserve boundaries without a JSON dependency.
    let mut bytes = b"gogoke.native.promotion-commit.v1".to_vec();
    let mut part = |value: &str| {
        bytes.extend_from_slice(value.len().to_string().as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(value.as_bytes());
    };
    for value in [actor.principal_id(), actor.seat_id(), &request.source_domain_id,
        &request.source_context_id, &request.source_version, &request.source_content_hash,
        &request.source_access_policy_revision, &request.destination_domain_id,
        &request.destination_scope, &request.promotion_kind, &request.policy_revision,
        &request.source_grant.grant_id, &request.source_grant.revision,
        &request.source_grant.revocation_head, &request.target_grant.grant_id,
        &request.target_grant.revision, &request.target_grant.revocation_head] {
        part(value);
    }
    part(&request.provenance_refs.len().to_string());
    for value in &request.provenance_refs { part(value); }
    content_hash(&bytes)
}

fn require_current_target(tx: &mut Transaction<'_, '_>, target: &ContextCommand) -> Result<()> {
    let reference = format!("{}@{}", target.context_id, target.version);
    let rows = tx.query(
        "SELECT v.content_hash,v.access_policy_revision,v.scope,s.state,a.visibility,a.read_grant_refs FROM gogoke_context_versions v JOIN gogoke_context_states s ON s.domain_id=v.domain_id AND s.version_ref=? JOIN gogoke_context_access a ON a.domain_id=v.domain_id AND a.version_ref=s.version_ref WHERE v.domain_id=? AND v.context_id=? AND v.version=?",
        &[&reference, &target.domain_id, &target.context_id, &target.version], 6)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1 || row[0] != target.content_hash
        || row[1] != target.access_policy_revision || row[2] != "GLOBAL"
        || row[3] != "ACTIVE" || row[4] != "OWNER_PRIVATE" || !row[5].is_empty() {
        return denied();
    }
    Ok(())
}

/// Internal composition helper; the Transaction type cannot be constructed by a
/// caller. Grant/source revalidation precedes BOTH new writes and durable replay.
pub(super) fn apply_authorized_promotion(
    tx: &mut Transaction<'_, '_>, actor: &OwnerIssuer,
    request: &PromotionRequest, target: ContextCommand,
) -> Result<ContextReceipt> {
    matches_target(&target, request)?;
    let profile = current_profile(tx)?;
    if profile.root_identity != tx.root_identity() { return denied(); }
    actor.check(&profile)?;
    authorize_owner_promotion(tx, actor, request)?;
    ensure_receipt_schema(tx)?;
    let expected = request_fingerprint(actor, request);
    let prior = tx.query(
        "SELECT domain_id,operation_id,context_id,version,context_fingerprint,principal_id,seat_id,source_domain_id,source_context_id,source_version,source_content_hash,source_access_policy_revision,destination_scope,promotion_kind,source_grant_id,source_grant_revision,target_grant_id,target_grant_revision,policy_revision,revocation_head,request_fingerprint FROM gogoke_context_promotion_authorizations WHERE domain_id=? AND operation_id=?",
        &[&target.domain_id, &target.operation_id], 21)?;
    let receipt = tx.apply_context(target.clone())?;
    let evidence_fields: [&str; 21] = [&target.domain_id, &target.operation_id, &target.context_id, &target.version,
                  &receipt.fingerprint, actor.principal_id(), actor.seat_id(),
                  &request.source_domain_id, &request.source_context_id, &request.source_version,
                  &request.source_content_hash, &request.source_access_policy_revision,
                  &request.destination_scope, &request.promotion_kind, &request.source_grant.grant_id,
                  &request.source_grant.revision, &request.target_grant.grant_id,
                  &request.target_grant.revision, &request.policy_revision,
                  &profile.revocation_head, &expected];
    match receipt.disposition {
        "COMMITTED" => {
            if !prior.is_empty() { return Err(OrchestrationError::OperationConflict); }
            tx.write(
                "INSERT INTO gogoke_context_promotion_authorizations(domain_id,operation_id,context_id,version,context_fingerprint,principal_id,seat_id,source_domain_id,source_context_id,source_version,source_content_hash,source_access_policy_revision,destination_scope,promotion_kind,source_grant_id,source_grant_revision,target_grant_id,target_grant_revision,policy_revision,revocation_head,request_fingerprint) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                &evidence_fields)?;
        }
        "RECONCILED" => {
            // A legacy syntactic promotion or a missing authorization receipt
            // cannot be upgraded to an authorized replay just because it exists.
            if prior.len() != 1 || prior[0].iter().map(String::as_str).ne(evidence_fields.iter().copied()) {
                return denied();
            }
        }
        _ => return denied(),
    }
    require_current_target(tx, &target)?;
    Ok(receipt)
}

pub(crate) fn commit_owner_promotion(
    connection: &mut VerifiedDatabaseConnection<'_>, actor: &OwnerIssuer,
    request: &PromotionRequest, target: ContextCommand,
) -> Result<ContextReceipt> {
    transaction::run(connection, |tx| apply_authorized_promotion(tx, actor, request, target))
}
