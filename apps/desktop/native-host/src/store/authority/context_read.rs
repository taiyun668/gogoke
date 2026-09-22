//! Source-qualified Context metadata reads on the existing Product Authority.
//! No new store, lifecycle index, cached permission, file I/O, or IPC actor ingress.
//! A returned snapshot is not an EgressGrant or permission to dispatch a manifest.
use super::bootstrap::{OwnerIssuer, Profile};
use super::catalog::{current_profile, resolve_current};
use super::model::{denied, identifier, revision, GrantRef};
use super::transaction::{self, Result, Transaction};
use super::super::same_open::VerifiedDatabaseConnection;

/// This request identifies an expected immutable source and the exact read right.
/// Values are comparison inputs, never caller-created authorization.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextReadRequest {
    pub source_domain_id: String,
    pub context_id: String,
    pub version: String,
    pub expected_scope: String,
    pub expected_content_hash: String,
    pub expected_access_policy_revision: String,
    pub destination_domain_id: String,
    pub destination_scope: String,
    pub promotion_kind: String,
    pub policy_revision: String,
    pub grant: GrantRef,
}

/// Metadata only: no content bytes and no transferable permission token.
/// The assembly commit and every replay must re-resolve current authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextReadSnapshot {
    pub source_domain_id: String,
    pub context_id: String,
    pub version: String,
    pub scope: String,
    pub kind: String,
    pub content_hash: String,
    pub source_ref: String,
    pub source_hash: String,
    pub source_authority_kind: String,
    pub source_authority_ref: String,
    pub access_policy_revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GranteeContextReadRequest {
    pub principal_id: String,
    pub seat_id: String,
    pub source: ContextReadRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedContextReadSnapshot {
    pub principal_id: String,
    pub seat_id: String,
    pub grant_revision: String,
    pub revocation_head: String,
    pub state: String,
    pub state_revision: String,
    pub source: ContextReadSnapshot,
}

// Each join includes the source domain. An identically named destination Context
// must not stand in for an absent or inaccessible source Context.
const READ_CONTEXT: &str = "SELECT v.scope,v.kind,v.content_hash,v.source_ref,v.source_hash,v.source_authority_kind,v.source_authority_ref,v.access_policy_revision,s.state,a.visibility,a.read_grant_refs FROM gogoke_context_versions v JOIN gogoke_context_states s ON s.domain_id=v.domain_id AND s.version_ref=? JOIN gogoke_context_access a ON a.domain_id=v.domain_id AND a.version_ref=s.version_ref WHERE v.domain_id=? AND v.context_id=? AND v.version=?";

fn valid_scope(value: &str) -> bool {
    matches!(value, "GLOBAL" | "PROJECT" | "SESSION")
}

fn valid_hash(value: &str) -> bool {
    value.len() == 71 && value.starts_with("sha256:")
        && value.as_bytes()[7..].iter().all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(x))
}

// Match the existing native Context value syntax, rather than accidentally
// applying the narrower Grant identity syntax to stored Context source refs.
fn context_text(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
        && value.bytes().all(|x| x.is_ascii_alphanumeric() || b"._:/-@".contains(&x))
}

pub(super) fn read_acl_allows(
    visibility: &str, references: &str, grant_id: &str,
    profile: &Profile, principal_id: &str, seat_id: &str,
) -> Result<bool> {
    let mut seen = std::collections::HashSet::new();
    let mut admitted = false;
    if !references.is_empty() {
        for reference in references.split(',') {
            if identifier(reference).is_err() || !seen.insert(reference) {
                return denied();
            }
            admitted |= reference == grant_id;
        }
    }
    if visibility == "OWNER_PRIVATE" {
        return Ok(principal_id == profile.principal_id && seat_id == profile.seat_id);
    }
    // DOMAIN_GRANTED still binds the concrete Context ACL, not just a matching
    // grant elsewhere in the catalog. No prefix or substring matching.
    if visibility != "DOMAIN_GRANTED" || references.is_empty() {
        return denied();
    }
    Ok(admitted)
}

fn validate_request(request: &ContextReadRequest) -> Result<()> {
    for value in [&request.source_domain_id, &request.destination_domain_id, &request.promotion_kind] {
        identifier(value)?;
    }
    revision(&request.version)?;
    revision(&request.expected_access_policy_revision)?;
    revision(&request.policy_revision)?;
    if !context_text(&request.context_id) || !valid_scope(&request.expected_scope)
        || !valid_scope(&request.destination_scope) || !valid_hash(&request.expected_content_hash) {
        return denied();
    }
    Ok(())
}

fn read_context_for_subject(
    tx: &mut Transaction<'_, '_>, profile: &Profile,
    principal_id: &str, seat_id: &str, request: &ContextReadRequest,
) -> Result<AuthorizedContextReadSnapshot> {
    identifier(principal_id)?;
    identifier(seat_id)?;
    validate_request(request)?;
    if request.policy_revision != profile.policy_revision {
        return denied();
    }
    let grant = resolve_current(tx, profile, &request.grant)?;
    let spec = grant.spec;
    if spec.principal_id != principal_id || spec.seat_id != seat_id
        || spec.permission != "context.read" || spec.promotion_kind != request.promotion_kind
        || spec.source_domain_id != request.source_domain_id
        || spec.destination_domain_id != request.destination_domain_id
        || spec.destination_scope != request.destination_scope {
        return denied();
    }
    let version_ref = format!("{}@{}", request.context_id, request.version);
    let rows = tx.query(READ_CONTEXT,
        &[&version_ref, &request.source_domain_id, &request.context_id, &request.version], 11)?;
    let row = rows.first().ok_or(super::super::orchestration::OrchestrationError::AccessDenied)?;
    let lifecycle = tx.context_state(&request.source_domain_id, &version_ref)?;
    if rows.len() != 1 || row.len() != 11 || row[0] != request.expected_scope
        || row[2] != request.expected_content_hash
        || row[7] != request.expected_access_policy_revision || row[8] != "ACTIVE"
        || lifecycle.state != row[8] || lifecycle.state != "ACTIVE"
        || !read_acl_allows(&row[9], &row[10], &request.grant.grant_id, profile, principal_id, seat_id)? {
        return denied();
    }
    if [1, 3, 5, 6].iter().any(|index| !context_text(&row[*index])) { return denied(); }
    if !valid_hash(&row[4]) { return denied(); }
    let source = ContextReadSnapshot {
        source_domain_id: request.source_domain_id.clone(),
        context_id: request.context_id.clone(), version: request.version.clone(),
        scope: row[0].clone(), kind: row[1].clone(), content_hash: row[2].clone(),
        source_ref: row[3].clone(), source_hash: row[4].clone(),
        source_authority_kind: row[5].clone(), source_authority_ref: row[6].clone(),
        access_policy_revision: row[7].clone(),
    };
    Ok(AuthorizedContextReadSnapshot {
        principal_id: principal_id.to_owned(), seat_id: seat_id.to_owned(),
        grant_revision: request.grant.revision.clone(),
        revocation_head: profile.revocation_head.clone(),
        state: lifecycle.state, state_revision: lifecycle.revision, source,
    })
}

/// Root Owner compatibility wrapper over the same grantee authority core.
pub(super) fn read_owner_context_in_transaction(
    tx: &mut Transaction<'_, '_>, actor: &OwnerIssuer, request: &ContextReadRequest,
) -> Result<ContextReadSnapshot> {
    let profile = current_profile(tx)?;
    actor.check(&profile)?;
    Ok(read_context_for_subject(tx, &profile, actor.principal_id(), actor.seat_id(), request)?.source)
}

pub(super) fn read_grantee_context_in_transaction(
    tx: &mut Transaction<'_, '_>, request: &GranteeContextReadRequest,
) -> Result<AuthorizedContextReadSnapshot> {
    let profile = current_profile(tx)?;
    read_context_for_subject(tx, &profile, &request.principal_id, &request.seat_id, &request.source)
}

/// A fresh transaction on every read, including a retry/replay of the same input.
/// There is deliberately no fallback to a cached grant or stale Context record.
pub(crate) fn read_owner_context(
    connection: &mut VerifiedDatabaseConnection<'_>, actor: &OwnerIssuer,
    request: &ContextReadRequest,
) -> Result<ContextReadSnapshot> {
    transaction::run(connection, |tx| read_owner_context_in_transaction(tx, actor, request))
}

pub(crate) fn read_grantee_context(
    connection: &mut VerifiedDatabaseConnection<'_>, request: &GranteeContextReadRequest,
) -> Result<AuthorizedContextReadSnapshot> {
    transaction::run(connection, |tx| read_grantee_context_in_transaction(tx, request))
}
