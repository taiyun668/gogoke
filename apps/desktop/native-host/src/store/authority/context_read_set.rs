//! A consistent native source set, not an AssemblyBasis or cached authorization.
//! Current grants and source metadata are read in one existing authority transaction.
use std::collections::HashSet;
use super::bootstrap::OwnerIssuer;
use super::catalog::current_profile;
use super::context_read::{
    read_grantee_context_in_transaction, read_owner_context_in_transaction,
    AuthorizedContextReadSnapshot, ContextReadRequest, ContextReadSnapshot, GranteeContextReadRequest,
};
use super::model::denied;
use super::transaction::{self, Result, Transaction};
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;

const MAX_CONTEXT_READ_SET: usize = 64;

/// Metadata and comparison coordinates only. No content, grant references or
/// transferable permission are returned. Commit/dispatch must authorize again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ContextReadSet {
    pub policy_revision: String,
    pub revocation_head: String,
    pub destination_domain_id: String,
    pub destination_scope: String,
    pub promotion_kind: String,
    pub sources: Vec<ContextReadSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedContextReadSet {
    pub principal_id: String,
    pub seat_id: String,
    pub policy_revision: String,
    pub revocation_head: String,
    pub destination_domain_id: String,
    pub destination_scope: String,
    pub promotion_kind: String,
    pub sources: Vec<AuthorizedContextReadSnapshot>,
}

pub(super) fn read_owner_context_set_in_transaction(
    tx: &mut Transaction<'_, '_>, owner: &OwnerIssuer, requests: &[ContextReadRequest],
) -> Result<ContextReadSet> {
    if requests.is_empty() || requests.len() > MAX_CONTEXT_READ_SET {
        return Err(OrchestrationError::Invalid("context source set bound"));
    }
    let profile = current_profile(tx)?;
    owner.check(&profile)?;
    let first = &requests[0];
    let mut seen = HashSet::new();
    // Reject an inconsistent set before loading ANY Context metadata. Do not
    // quietly select one of multiple versions or truncate an oversized set.
    for request in requests {
        if request.policy_revision != profile.policy_revision
            || request.grant.revocation_head != profile.revocation_head
            || request.destination_domain_id != first.destination_domain_id
            || request.destination_scope != first.destination_scope
            || request.promotion_kind != first.promotion_kind
            || !seen.insert((request.source_domain_id.as_str(), request.context_id.as_str())) {
            return denied();
        }
    }
    let mut sources = Vec::with_capacity(requests.len());
    for request in requests {
        // Reuse the real current Catalog and Context ACL checks for every item.
        // An error returns no partial set. No callback/await/second connection.
        sources.push(read_owner_context_in_transaction(tx, owner, request)?);
    }
    Ok(ContextReadSet {
        policy_revision: profile.policy_revision,
        revocation_head: profile.revocation_head,
        destination_domain_id: first.destination_domain_id.clone(),
        destination_scope: first.destination_scope.clone(),
        promotion_kind: first.promotion_kind.clone(),
        sources,
    })
}

/// Replays are fresh reads. A previous ContextReadSet is never accepted as proof.
pub(crate) fn read_owner_context_set(
    connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer,
    requests: &[ContextReadRequest],
) -> Result<ContextReadSet> {
    transaction::run(connection, |tx| read_owner_context_set_in_transaction(tx, owner, requests))
}

pub(super) fn read_grantee_context_set_in_transaction(
    tx: &mut Transaction<'_, '_>, requests: &[GranteeContextReadRequest],
) -> Result<AuthorizedContextReadSet> {
    if requests.is_empty() || requests.len() > MAX_CONTEXT_READ_SET {
        return Err(OrchestrationError::Invalid("context source set bound"));
    }
    let profile = current_profile(tx)?;
    let first = &requests[0];
    let principal_id = &first.principal_id;
    let seat_id = &first.seat_id;
    let first_source = &first.source;
    let mut seen = HashSet::new();
    for request in requests {
        let source = &request.source;
        if request.principal_id != principal_id.as_str() || request.seat_id != seat_id.as_str()
            || source.policy_revision != profile.policy_revision
            || source.grant.revocation_head != profile.revocation_head
            || source.destination_domain_id != first_source.destination_domain_id
            || source.destination_scope != first_source.destination_scope
            || source.promotion_kind != first_source.promotion_kind
            || !seen.insert((source.source_domain_id.as_str(), source.context_id.as_str())) {
            return denied();
        }
    }
    let mut sources = Vec::with_capacity(requests.len());
    for request in requests {
        sources.push(read_grantee_context_in_transaction(tx, request)?);
    }
    Ok(AuthorizedContextReadSet {
        principal_id: principal_id.clone(), seat_id: seat_id.clone(),
        policy_revision: profile.policy_revision, revocation_head: profile.revocation_head,
        destination_domain_id: first_source.destination_domain_id.clone(),
        destination_scope: first_source.destination_scope.clone(),
        promotion_kind: first_source.promotion_kind.clone(), sources,
    })
}

pub(crate) fn read_grantee_context_set(
    connection: &mut VerifiedDatabaseConnection<'_>, requests: &[GranteeContextReadRequest],
) -> Result<AuthorizedContextReadSet> {
    transaction::run(connection, |tx| read_grantee_context_set_in_transaction(tx, requests))
}
