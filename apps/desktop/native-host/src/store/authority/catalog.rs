//! Authoritative grant revisions and lineage on the existing native SQLite domain.
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::bootstrap::{self, OwnerIssuer, ProductIdentitySnapshot, Profile};
use super::model::{denied, identifier, next_revision, revision, GrantRecord, GrantRef, GrantSpec};
use super::transaction::{self, Result, Transaction};
use std::collections::HashSet;

pub(super) fn seat_issuer(principal: &str, seat: &str) -> String {
    content_hash(format!("gogoke.authority.seat.v1\0{principal}\0{seat}").as_bytes())
}

pub(super) fn current_profile(tx: &mut Transaction<'_, '_>) -> Result<Profile> {
    bootstrap::verify_schema(tx)?;
    bootstrap::profile(tx)
}

pub(super) fn verify_grant_payload_kind(
    tx: &mut Transaction<'_, '_>,
    grant_id: &str,
    grant_revision: &str,
    kind: &str,
) -> Result<()> {
    let rows = tx.query(
        "SELECT (SELECT count(*) FROM main.gogoke_authority_context_grant_payloads WHERE grant_id=? AND revision=?),(SELECT count(*) FROM main.gogoke_authority_delegation_grant_payloads WHERE grant_id=? AND revision=?)",
        &[grant_id, grant_revision, grant_id, grant_revision], 2)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1
        || !matches!(
            (kind, row[0].as_str(), row[1].as_str()),
            ("CONTEXT", "1", "0") | ("DELEGATION", "0", "1")
        )
    {
        return denied();
    }
    Ok(())
}

fn load_current(
    tx: &mut Transaction<'_, '_>,
    reference: &GrantRef,
    profile: &Profile,
) -> Result<GrantRecord> {
    verify_grant_payload_kind(tx, &reference.grant_id, &reference.revision, "CONTEXT")?;
    let rows = tx.query(
        "SELECT p.principal_id,p.seat_id,p.permission,p.promotion_kind,p.source_domain_id,p.destination_domain_id,p.destination_scope,g.issuer_id,COALESCE(g.parent_grant_id,''),COALESCE(g.parent_revision,''),g.policy_revision,p.delegable_depth,g.issued_revocation_head FROM main.gogoke_authority_grants g JOIN main.gogoke_authority_grant_heads h ON h.grant_id=g.grant_id AND h.revision=g.revision JOIN main.gogoke_authority_context_grant_payloads p ON p.grant_id=g.grant_id AND p.revision=g.revision WHERE g.grant_id=? AND g.revision=? AND h.revoked=0 AND g.grant_kind='CONTEXT'",
        &[&reference.grant_id, &reference.revision], 13)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1
        || row[10] != profile.policy_revision
        || revision(&row[12])? > revision(&profile.revocation_head)?
    {
        return denied();
    }
    let spec = GrantSpec {
        principal_id: row[0].clone(),
        seat_id: row[1].clone(),
        permission: row[2].clone(),
        promotion_kind: row[3].clone(),
        source_domain_id: row[4].clone(),
        destination_domain_id: row[5].clone(),
        destination_scope: row[6].clone(),
        delegable_depth: row[11]
            .parse()
            .map_err(|_| OrchestrationError::AccessDenied)?,
    };
    spec.validate()?;
    let parent = match (row[8].is_empty(), row[9].is_empty()) {
        (true, true) => None,
        (false, false) => {
            identifier(&row[8])?;
            revision(&row[9])?;
            Some((row[8].clone(), row[9].clone()))
        }
        _ => return denied(),
    };
    Ok(GrantRecord {
        reference: reference.clone(),
        spec,
        issuer_id: row[7].clone(),
        parent,
        policy_revision: row[10].clone(),
    })
}

// Must remain inside the caller's active authoritative transaction. A parsed
// reference or a positive cached answer is not a transferable authorization.
pub(super) fn resolve_current(
    tx: &mut Transaction<'_, '_>,
    profile: &Profile,
    reference: &GrantRef,
) -> Result<GrantRecord> {
    reference.validate()?;
    if reference.revocation_head != profile.revocation_head {
        return denied();
    }
    let leaf = load_current(tx, reference, profile)?;
    let mut current = leaf.clone();
    let mut seen = HashSet::new();
    for _ in 0..=32 {
        if !seen.insert((
            current.reference.grant_id.clone(),
            current.reference.revision.clone(),
        )) {
            return denied();
        }
        let Some((parent_id, parent_revision)) = &current.parent else {
            return if current.issuer_id == profile.issuer_id {
                Ok(leaf)
            } else {
                denied()
            };
        };
        let parent_ref = GrantRef {
            grant_id: parent_id.clone(),
            revision: parent_revision.clone(),
            revocation_head: profile.revocation_head.clone(),
        };
        let parent = load_current(tx, &parent_ref, profile)?;
        if current.policy_revision != parent.policy_revision
            || !current.spec.within(&parent.spec)
            || current.issuer_id != seat_issuer(&parent.spec.principal_id, &parent.spec.seat_id)
        {
            return denied();
        }
        current = parent;
    }
    denied()
}

fn owner_profile(
    tx: &mut Transaction<'_, '_>,
    owner: &OwnerIssuer,
    policy: &str,
    revocation: &str,
) -> Result<Profile> {
    revision(policy)?;
    revision(revocation)?;
    let profile = current_profile(tx)?;
    owner.check(&profile)?;
    if policy != profile.policy_revision || revocation != profile.revocation_head {
        return denied();
    }
    Ok(profile)
}

fn append_record(
    tx: &mut Transaction<'_, '_>,
    profile: &Profile,
    record: &GrantRecord,
) -> Result<()> {
    let (parent_id, parent_revision) = record
        .parent
        .as_ref()
        .map(|(id, revision)| (id.as_str(), revision.as_str()))
        .unwrap_or(("", ""));
    tx.write(
        "INSERT INTO main.gogoke_authority_grants(grant_id,revision,grant_kind,issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head) VALUES(?,?,'CONTEXT',?,NULLIF(?,''),NULLIF(?,''),?,?)",
        &[&record.reference.grant_id, &record.reference.revision, &record.issuer_id,
          parent_id, parent_revision, &record.policy_revision, &profile.revocation_head])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_context_grant_payloads(grant_id,revision,principal_id,seat_id,permission,promotion_kind,source_domain_id,destination_domain_id,destination_scope,delegable_depth) VALUES(?,?,?,?,?,?,?,?,?,?)",
        &[&record.reference.grant_id, &record.reference.revision, &record.spec.principal_id,
          &record.spec.seat_id, &record.spec.permission, &record.spec.promotion_kind,
          &record.spec.source_domain_id, &record.spec.destination_domain_id,
          &record.spec.destination_scope, &record.spec.delegable_depth.to_string()])
}

fn audit(
    tx: &mut Transaction<'_, '_>,
    kind: &str,
    issuer: &str,
    reference: &GrantRef,
    policy: &str,
) -> Result<()> {
    tx.write(
        "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES(?,?,?,?,?,?)",
        &[kind, issuer, &reference.grant_id, &reference.revision, policy, &reference.revocation_head])
}

fn create_record(
    tx: &mut Transaction<'_, '_>,
    profile: &Profile,
    spec: GrantSpec,
    issuer_id: String,
    parent: Option<(String, String)>,
) -> Result<GrantRef> {
    let reference = GrantRef {
        grant_id: bootstrap::random_id("grant")?,
        revision: "1".into(),
        revocation_head: profile.revocation_head.clone(),
    };
    let record = GrantRecord {
        reference: reference.clone(),
        spec,
        issuer_id,
        parent,
        policy_revision: profile.policy_revision.clone(),
    };
    append_record(tx, profile, &record)?;
    tx.write(
        "INSERT INTO main.gogoke_authority_grant_heads(grant_id,revision,revoked) VALUES(?,?,0)",
        &[&reference.grant_id, &reference.revision],
    )?;
    audit(
        tx,
        "ISSUE",
        &record.issuer_id,
        &reference,
        &profile.policy_revision,
    )?;
    Ok(reference)
}

pub(crate) fn issue_owner_grant(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    policy: &str,
    revocation: &str,
    spec: GrantSpec,
) -> Result<GrantRef> {
    spec.validate()?;
    transaction::run(connection, |tx| {
        let profile = owner_profile(tx, owner, policy, revocation)?;
        create_record(tx, &profile, spec, profile.issuer_id.clone(), None)
    })
}

pub(crate) fn r2_public_context_grant_id() -> String {
    let digest = content_hash(format!("r2-02-public-context-grant:{}", super::PUBLIC_R2_MANIFEST_BLOB).as_bytes());
    format!("grant:r2-02-context:{}", &digest[7..])
}

/// One Owner-issued context.read grant for the public R2 fixture only. The
/// deterministic identity makes a lost reply replay the same bounded grant.
pub(crate) fn issue_r2_public_context_grant_once(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    admitted: &ProductIdentitySnapshot,
) -> Result<GrantRef> {
    let spec = GrantSpec {
        principal_id: "principal-r2-02-worker".into(),
        seat_id: "seat-r2-02-worker".into(),
        permission: "context.read".into(),
        promotion_kind: "PROJECT_ONLY".into(),
        source_domain_id: "domain-r2-02-source".into(),
        destination_domain_id: "domain-r2-02-test".into(),
        destination_scope: "PROJECT".into(),
        delegable_depth: 0,
    };
    let grant_id = r2_public_context_grant_id();
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        owner.check(&profile)?;
        if profile.profile_id != admitted.profile_id
            || profile.root_identity != admitted.root_identity
            || profile.principal_id != admitted.principal_id
            || profile.seat_id != admitted.seat_id
            || profile.policy_revision != admitted.policy_revision
            || profile.revocation_head != admitted.revocation_head {
            return denied();
        }
        let reference = GrantRef {
            grant_id: grant_id.clone(), revision: "1".into(),
            revocation_head: profile.revocation_head.clone(),
        };
        let heads = tx.query(
            "SELECT revision,revoked FROM main.gogoke_authority_grant_heads WHERE grant_id=?",
            &[&grant_id], 2,
        )?;
        if !heads.is_empty() {
            if heads.len() != 1 || heads[0][0] != "1" || heads[0][1] != "0" {
                return denied();
            }
            let current = resolve_current(tx, &profile, &reference)?;
            if current.parent.is_some() || current.spec != spec || current.issuer_id != profile.issuer_id {
                return Err(OrchestrationError::OperationConflict);
            }
            return Ok(reference);
        }
        let record = GrantRecord {
            reference: reference.clone(), spec,
            issuer_id: profile.issuer_id.clone(), parent: None,
            policy_revision: profile.policy_revision.clone(),
        };
        append_record(tx, &profile, &record)?;
        tx.write("INSERT INTO main.gogoke_authority_grant_heads(grant_id,revision,revoked) VALUES(?,?,0)",
            &[&reference.grant_id, &reference.revision])?;
        audit(tx, "ISSUE", &record.issuer_id, &reference, &profile.policy_revision)?;
        resolve_current(tx, &profile, &reference)?;
        Ok(reference)
    })
}

// Currently reachable only by the native bootstrap Owner actor. This is not a
// mechanism for a model to impersonate an arbitrary grant's grantee. Non-Owner
// authenticated seat ingress remains unconnected, rather than accepting IDs.
pub(crate) fn delegate_owner_grant(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    policy: &str,
    parent_ref: &GrantRef,
    spec: GrantSpec,
) -> Result<GrantRef> {
    spec.validate()?;
    transaction::run(connection, |tx| {
        let profile = owner_profile(tx, owner, policy, &parent_ref.revocation_head)?;
        let parent = resolve_current(tx, &profile, parent_ref)?;
        if parent.spec.principal_id != owner.principal_id()
            || parent.spec.seat_id != owner.seat_id()
            || !spec.within(&parent.spec)
        {
            return denied();
        }
        create_record(
            tx,
            &profile,
            spec,
            seat_issuer(owner.principal_id(), owner.seat_id()),
            Some((parent_ref.grant_id.clone(), parent_ref.revision.clone())),
        )
    })
}

pub(crate) fn revise_owner_grant(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    policy: &str,
    expected: &GrantRef,
    spec: GrantSpec,
) -> Result<GrantRef> {
    spec.validate()?;
    transaction::run(connection, |tx| {
        let profile = owner_profile(tx, owner, policy, &expected.revocation_head)?;
        let old = resolve_current(tx, &profile, expected)?;
        // A revision cannot turn a delegated grant into a new root grant.
        if old.parent.is_some() || old.issuer_id != profile.issuer_id {
            return denied();
        }
        let reference = GrantRef {
            grant_id: expected.grant_id.clone(),
            revision: next_revision(&expected.revision)?,
            revocation_head: profile.revocation_head.clone(),
        };
        let record = GrantRecord {
            reference: reference.clone(),
            spec,
            issuer_id: profile.issuer_id.clone(),
            parent: None,
            policy_revision: profile.policy_revision.clone(),
        };
        append_record(tx, &profile, &record)?;
        tx.write("UPDATE main.gogoke_authority_grant_heads SET revision=? WHERE grant_id=? AND revision=? AND revoked=0",
            &[&reference.revision, &expected.grant_id, &expected.revision])?;
        if tx.query("SELECT changes()", &[], 1)?[0][0] != "1" {
            return denied();
        }
        audit(
            tx,
            "REVISE",
            &profile.issuer_id,
            &reference,
            &profile.policy_revision,
        )?;
        Ok(reference)
    })
}

pub(crate) fn revoke_owner_grant(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    policy: &str,
    expected: &GrantRef,
) -> Result<String> {
    transaction::run(connection, |tx| {
        let profile = owner_profile(tx, owner, policy, &expected.revocation_head)?;
        resolve_current(tx, &profile, expected)?;
        let next = next_revision(&profile.revocation_head)?;
        tx.write("UPDATE main.gogoke_authority_grant_heads SET revoked=1 WHERE grant_id=? AND revision=? AND revoked=0",
            &[&expected.grant_id, &expected.revision])?;
        if tx.query("SELECT changes()", &[], 1)?[0][0] != "1" {
            return denied();
        }
        tx.write("UPDATE main.gogoke_authority_profile SET revocation_head=? WHERE singleton=1 AND revocation_head=?",
            &[&next, &profile.revocation_head])?;
        if tx.query("SELECT changes()", &[], 1)?[0][0] != "1" {
            return denied();
        }
        let revoked = GrantRef {
            revocation_head: next.clone(),
            ..expected.clone()
        };
        audit(
            tx,
            "REVOKE",
            &profile.issuer_id,
            &revoked,
            &profile.policy_revision,
        )?;
        Ok(next)
    })
}
