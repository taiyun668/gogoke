//! Native promotion admission. This function is valid only inside the commit
//! transaction; it deliberately returns no cacheable/serializable permission.
use std::collections::HashSet;
use super::bootstrap::OwnerIssuer;
use super::catalog::{current_profile, resolve_current};
use super::model::{denied, identifier, revision, GrantRef};
use super::transaction::{Result, Transaction};
use super::super::orchestration::OrchestrationError;

#[derive(Clone, Debug)]
pub(crate) struct PromotionRequest {
    pub source_context_id: String,
    pub source_version: String,
    pub source_domain_id: String,
    pub source_content_hash: String,
    pub source_access_policy_revision: String,
    pub destination_domain_id: String,
    pub destination_scope: String,
    pub promotion_kind: String,
    pub policy_revision: String,
    pub source_grant: GrantRef,
    pub target_grant: GrantRef,
    pub provenance_refs: Vec<String>,
}

fn digest(value: &str) -> Result<()> {
    if value.len() != 71 || !value.starts_with("sha256:")
        || !value.as_bytes()[7..].iter().all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(x)) {
        return Err(OrchestrationError::Invalid("promotion source hash"));
    }
    Ok(())
}

pub(super) fn authorize_owner_promotion(
    tx: &mut Transaction<'_, '_>, actor: &OwnerIssuer, request: &PromotionRequest,
) -> Result<()> {
    for value in [&request.source_context_id, &request.source_domain_id,
        &request.destination_domain_id, &request.promotion_kind] { identifier(value)?; }
    revision(&request.source_version)?;
    revision(&request.source_access_policy_revision)?;
    revision(&request.policy_revision)?;
    digest(&request.source_content_hash)?;
    if request.destination_scope != "GLOBAL" || request.provenance_refs.is_empty()
        || request.provenance_refs.len() > 64 { return denied(); }
    let mut seen = HashSet::new();
    for reference in &request.provenance_refs {
        identifier(reference)?;
        if !seen.insert(reference) { return denied(); }
    }
    let profile = current_profile(tx)?;
    actor.check(&profile)?;
    if request.policy_revision != profile.policy_revision { return denied(); }
    for (reference, permission) in [(&request.source_grant, "context.promote.source"),
        (&request.target_grant, "context.promote.target")] {
        let grant = resolve_current(tx, &profile, reference)?;
        let spec = grant.spec;
        if spec.principal_id != actor.principal_id() || spec.seat_id != actor.seat_id()
            || spec.permission != permission || spec.promotion_kind != request.promotion_kind
            || spec.source_domain_id != request.source_domain_id
            || spec.destination_domain_id != request.destination_domain_id
            || spec.destination_scope != request.destination_scope { return denied(); }
    }
    // The source domain is explicit. Never resolve ctx@version in the target
    // domain or infer cross-domain permission from content hashes or confidence.
    let reference = format!("{}@{}", request.source_context_id, request.source_version);
    let rows = tx.query(
        "SELECT v.content_hash,v.access_policy_revision,v.scope,s.state FROM gogoke_context_versions v JOIN gogoke_context_states s ON s.domain_id=v.domain_id AND s.version_ref=? WHERE v.domain_id=? AND v.context_id=? AND v.version=?",
        &[&reference, &request.source_domain_id, &request.source_context_id, &request.source_version], 4)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1 || row[0] != request.source_content_hash
        || row[1] != request.source_access_policy_revision || row[2] != "PROJECT" || row[3] != "ACTIVE" {
        return denied();
    }
    Ok(())
}
