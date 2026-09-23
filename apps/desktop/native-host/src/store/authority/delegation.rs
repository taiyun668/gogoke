//! Durable Controller delegation grants in the single Product Authority.
//! Context grants have a separate typed payload and never resolve here.
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::orchestration::OrchestrationError;
use super::super::digest::content_hash;
use super::super::same_open::VerifiedDatabaseConnection;
use super::bootstrap::{self, OwnerIssuer, Profile};
use super::catalog::verify_grant_payload_kind;
use super::catalog::{current_profile, seat_issuer};
use super::model::{denied, identifier, next_revision, revision, GrantRef};
use super::transaction::{self, Result, Transaction};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const R2_V2_MANIFEST_BLOB: &str = "17afbba28795338561530927595fda93fd8a2a11";
const CEILING_AXES: [&str; 7] = [
    "allowed_actions",
    "allowed_target_principal_ids",
    "allowed_target_domain_ids",
    "allowed_sinks",
    "allowed_material_classes",
    "explicit_private_material_ids",
    "allowed_continuation_responses",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DelegationGrantIdentity {
    pub grant_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DelegationPrincipal {
    pub principal_id: String,
    pub project_id: String,
    pub domain_id: String,
    pub role: String,
    pub seat_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DelegationBinding {
    pub session_id: String,
    pub execution_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorityCeiling {
    pub allowed_actions: Vec<String>,
    pub allowed_target_principal_ids: Vec<String>,
    pub allowed_target_domain_ids: Vec<String>,
    pub allowed_sinks: Vec<String>,
    pub allowed_material_classes: Vec<String>,
    pub explicit_private_material_ids: Vec<String>,
    pub allowed_continuation_responses: Vec<String>,
    pub max_material_items: u64,
    pub max_material_bytes: u64,
    pub max_response_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DelegationGrantInput {
    pub principal: DelegationPrincipal,
    pub binding: DelegationBinding,
    pub expires_at_epoch_ms: u64,
    pub ceiling: AuthorityCeiling,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DelegationGrantSnapshot {
    /// grant_id is the canonical grantRef; current revision/revocation head are
    /// native Product Authority values returned from durable state.
    pub reference: GrantRef,
    pub issuer_id: String,
    pub parent: Option<GrantRef>,
    pub policy_revision: String,
    pub principal: DelegationPrincipal,
    pub binding: DelegationBinding,
    pub expires_at_epoch_ms: u64,
    pub ceiling: AuthorityCeiling,
}

impl DelegationGrantIdentity {
    fn validate(&self) -> Result<()> {
        identifier(&self.grant_id)?;
        revision(&self.revision)?;
        Ok(())
    }
}

impl DelegationGrantInput {
    fn validate(&self) -> Result<()> {
        for value in [
            &self.principal.principal_id,
            &self.principal.project_id,
            &self.principal.domain_id,
            &self.principal.role,
            &self.principal.seat_id,
            &self.binding.session_id,
            &self.binding.execution_id,
            &self.binding.generation,
        ] {
            identifier(value)?;
        }
        if self.expires_at_epoch_ms > MAX_SAFE_INTEGER {
            return Err(OrchestrationError::Invalid("delegation expiry bounds"));
        }
        self.ceiling.validate()
    }
}

impl AuthorityCeiling {
    fn axes(&self) -> [&[String]; 7] {
        [
            &self.allowed_actions,
            &self.allowed_target_principal_ids,
            &self.allowed_target_domain_ids,
            &self.allowed_sinks,
            &self.allowed_material_classes,
            &self.explicit_private_material_ids,
            &self.allowed_continuation_responses,
        ]
    }

    fn validate(&self) -> Result<()> {
        for values in self.axes() {
            for value in values {
                identifier(value)?;
            }
        }
        for value in [
            self.max_material_items,
            self.max_material_bytes,
            self.max_response_bytes,
        ] {
            if value > MAX_SAFE_INTEGER {
                return Err(OrchestrationError::Invalid("delegation ceiling bounds"));
            }
        }
        Ok(())
    }

    fn within(&self, parent: &Self) -> bool {
        self.axes()
            .iter()
            .zip(parent.axes())
            .all(|(child, allowed)| child.iter().all(|item| allowed.contains(item)))
            && self.max_material_items <= parent.max_material_items
            && self.max_material_bytes <= parent.max_material_bytes
            && self.max_response_bytes <= parent.max_response_bytes
    }
}

fn current_epoch_ms() -> Result<u64> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OrchestrationError::Invalid("delegation clock"))?
        .as_millis();
    u64::try_from(millis).map_err(|_| OrchestrationError::Invalid("delegation clock"))
}

fn validate_owner(owner: &OwnerIssuer, profile: &Profile) -> Result<()> {
    owner.check(profile)
}

fn read_payload(
    tx: &mut Transaction<'_, '_>,
    grant_id: &str,
    revision_value: &str,
) -> Result<DelegationGrantInput> {
    let rows = tx.query(
        "SELECT p.principal_id,p.project_id,p.domain_id,p.role,p.seat_id,p.session_id,p.execution_id,p.generation,p.expires_at_epoch_ms,p.max_material_items,p.max_material_bytes,p.max_response_bytes FROM main.gogoke_authority_delegation_grant_payloads p JOIN main.gogoke_authority_grants g ON g.grant_id=p.grant_id AND g.revision=p.revision WHERE p.grant_id=? AND p.revision=? AND g.grant_kind='DELEGATION'",
        &[grant_id, revision_value], 12)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1 {
        return denied();
    }
    let mut axis_values: Vec<Vec<String>> = Vec::with_capacity(CEILING_AXES.len());
    for axis in CEILING_AXES {
        let rows = tx.query(
            "SELECT COALESCE(group_concat(CAST(ordinal AS TEXT)||':'||length(value)||':'||value,''),'') FROM (SELECT ordinal,value FROM main.gogoke_authority_delegation_ceiling_entries WHERE grant_id=? AND revision=? AND axis=? ORDER BY ordinal)",
            &[grant_id, revision_value, axis], 1)?;
        let encoded = rows.first().ok_or(OrchestrationError::AccessDenied)?[0].as_str();
        axis_values.push(decode_axis(encoded)?);
    }
    let number = |index: usize| -> Result<u64> {
        row[index]
            .parse()
            .map_err(|_| OrchestrationError::AccessDenied)
    };
    let input = DelegationGrantInput {
        principal: DelegationPrincipal {
            principal_id: row[0].clone(),
            project_id: row[1].clone(),
            domain_id: row[2].clone(),
            role: row[3].clone(),
            seat_id: row[4].clone(),
        },
        binding: DelegationBinding {
            session_id: row[5].clone(),
            execution_id: row[6].clone(),
            generation: row[7].clone(),
        },
        expires_at_epoch_ms: number(8)?,
        ceiling: AuthorityCeiling {
            allowed_actions: std::mem::take(&mut axis_values[0]),
            allowed_target_principal_ids: std::mem::take(&mut axis_values[1]),
            allowed_target_domain_ids: std::mem::take(&mut axis_values[2]),
            allowed_sinks: std::mem::take(&mut axis_values[3]),
            allowed_material_classes: std::mem::take(&mut axis_values[4]),
            explicit_private_material_ids: std::mem::take(&mut axis_values[5]),
            allowed_continuation_responses: std::mem::take(&mut axis_values[6]),
            max_material_items: number(9)?,
            max_material_bytes: number(10)?,
            max_response_bytes: number(11)?,
        },
    };
    input.validate()?;
    Ok(input)
}

fn decode_axis(encoded: &str) -> Result<Vec<String>> {
    let mut values = Vec::new();
    let mut remaining = encoded;
    while !remaining.is_empty() {
        let ordinal_separator = remaining
            .find(':')
            .ok_or(OrchestrationError::AccessDenied)?;
        let ordinal: usize = remaining[..ordinal_separator]
            .parse()
            .map_err(|_| OrchestrationError::AccessDenied)?;
        if ordinal != values.len() {
            return denied();
        }
        let after_ordinal = &remaining[ordinal_separator + 1..];
        let length_separator = after_ordinal
            .find(':')
            .ok_or(OrchestrationError::AccessDenied)?;
        let length: usize = after_ordinal[..length_separator]
            .parse()
            .map_err(|_| OrchestrationError::AccessDenied)?;
        let rest = &after_ordinal[length_separator + 1..];
        if length == 0 || length > rest.len() || !rest.is_char_boundary(length) {
            return denied();
        }
        values.push(rest[..length].to_owned());
        remaining = &rest[length..];
    }
    Ok(values)
}

fn insert_grant(
    tx: &mut Transaction<'_, '_>,
    profile: &Profile,
    identity: &DelegationGrantIdentity,
    input: &DelegationGrantInput,
    issuer_id: &str,
    parent: Option<&DelegationGrantIdentity>,
) -> Result<()> {
    let (parent_id, parent_revision) = parent
        .map(|parent| (parent.grant_id.as_str(), parent.revision.as_str()))
        .unwrap_or(("", ""));
    tx.write(
        "INSERT INTO main.gogoke_authority_grants(grant_id,revision,grant_kind,issuer_id,parent_grant_id,parent_revision,policy_revision,issued_revocation_head) VALUES(?,?,'DELEGATION',?,NULLIF(?,''),NULLIF(?,''),?,?)",
        &[&identity.grant_id, &identity.revision, issuer_id, parent_id, parent_revision,
          &profile.policy_revision, &profile.revocation_head])?;
    tx.write(
        "INSERT INTO main.gogoke_authority_delegation_grant_payloads(grant_id,revision,principal_id,project_id,domain_id,role,seat_id,session_id,execution_id,generation,expires_at_epoch_ms,max_material_items,max_material_bytes,max_response_bytes) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        &[&identity.grant_id, &identity.revision, &input.principal.principal_id,
          &input.principal.project_id, &input.principal.domain_id, &input.principal.role,
          &input.principal.seat_id, &input.binding.session_id, &input.binding.execution_id,
          &input.binding.generation, &input.expires_at_epoch_ms.to_string(),
          &input.ceiling.max_material_items.to_string(), &input.ceiling.max_material_bytes.to_string(),
          &input.ceiling.max_response_bytes.to_string()])?;
    for (axis, values) in CEILING_AXES.into_iter().zip(input.ceiling.axes()) {
        for (ordinal, value) in values.iter().enumerate() {
            tx.write(
                "INSERT INTO main.gogoke_authority_delegation_ceiling_entries(grant_id,revision,axis,ordinal,value) VALUES(?,?,?,?,?)",
                &[&identity.grant_id, &identity.revision, axis, &ordinal.to_string(), value])?;
        }
    }
    Ok(())
}

fn load_core(
    tx: &mut Transaction<'_, '_>,
    identity: &DelegationGrantIdentity,
    profile: &Profile,
) -> Result<DelegationGrantSnapshot> {
    identity.validate()?;
    let rows = tx.query(
        "SELECT g.grant_kind,g.issuer_id,COALESCE(g.parent_grant_id,''),COALESCE(g.parent_revision,''),g.policy_revision,g.issued_revocation_head,h.revision,h.revoked FROM main.gogoke_authority_grants g JOIN main.gogoke_authority_grant_heads h ON h.grant_id=g.grant_id AND h.revision=g.revision WHERE g.grant_id=? AND g.revision=?",
        &[&identity.grant_id, &identity.revision], 8)?;
    let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
    if rows.len() != 1 || !matches!(row[0].as_str(), "CONTEXT" | "DELEGATION") {
        return denied();
    }
    verify_grant_payload_kind(tx, &identity.grant_id, &identity.revision, &row[0])?;
    if row[0] != "DELEGATION"
        || row[7] != "0"
        || row[4] != profile.policy_revision
        || revision(&row[5])? > revision(&profile.revocation_head)?
        || row[6] != identity.revision
    {
        return denied();
    }
    let parent = match (row[2].is_empty(), row[3].is_empty()) {
        (true, true) => None,
        (false, false) => {
            identifier(&row[2])?;
            revision(&row[3])?;
            Some(GrantRef {
                grant_id: row[2].clone(),
                revision: row[3].clone(),
                revocation_head: profile.revocation_head.clone(),
            })
        }
        _ => return denied(),
    };
    let input = read_payload(tx, &identity.grant_id, &identity.revision)?;
    let reference = GrantRef {
        grant_id: identity.grant_id.clone(),
        revision: identity.revision.clone(),
        revocation_head: profile.revocation_head.clone(),
    };
    Ok(DelegationGrantSnapshot {
        reference,
        issuer_id: row[1].clone(),
        parent,
        policy_revision: row[4].clone(),
        principal: input.principal,
        binding: input.binding,
        expires_at_epoch_ms: input.expires_at_epoch_ms,
        ceiling: input.ceiling,
    })
}

pub(super) fn current_in_transaction(
    tx: &mut Transaction<'_, '_>,
    profile: &Profile,
    identity: &DelegationGrantIdentity,
) -> Result<DelegationGrantSnapshot> {
    let leaf = load_core(tx, identity, profile)?;
    if leaf.expires_at_epoch_ms <= current_epoch_ms()? {
        return denied();
    }
    let mut current = leaf.clone();
    let mut visited = HashSet::new();
    for _ in 0..=32 {
        if !visited.insert((
            current.reference.grant_id.clone(),
            current.reference.revision.clone(),
        )) {
            return denied();
        }
        let Some(parent_reference) = &current.parent else {
            return if current.issuer_id == profile.issuer_id {
                Ok(leaf)
            } else {
                denied()
            };
        };
        let parent_identity = DelegationGrantIdentity {
            grant_id: parent_reference.grant_id.clone(),
            revision: parent_reference.revision.clone(),
        };
        let parent = load_core(tx, &parent_identity, profile)?;
        if parent.expires_at_epoch_ms <= current_epoch_ms()?
            || current.issuer_id
                != seat_issuer(&parent.principal.principal_id, &parent.principal.seat_id)
            || current.principal.project_id != parent.principal.project_id
            || current.principal.domain_id != parent.principal.domain_id
            || current.expires_at_epoch_ms > parent.expires_at_epoch_ms
            || !current.ceiling.within(&parent.ceiling)
        {
            return denied();
        }
        current = parent;
    }
    denied()
}

pub(crate) fn issue_owner_delegation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    input: DelegationGrantInput,
) -> Result<DelegationGrantSnapshot> {
    input.validate()?;
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        validate_owner(owner, &profile)?;
        let identity = DelegationGrantIdentity {
            grant_id: bootstrap::random_id("grant")?,
            revision: "1".into(),
        };
        insert_grant(tx, &profile, &identity, &input, &profile.issuer_id, None)?;
        tx.write("INSERT INTO main.gogoke_authority_grant_heads(grant_id,revision,revoked) VALUES(?,?,0)",
            &[&identity.grant_id, &identity.revision])?;
        tx.write(
            "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES('ISSUE',?,?,?,?,?)",
            &[&profile.issuer_id, &identity.grant_id, &identity.revision, &profile.policy_revision, &profile.revocation_head])?;
        current_in_transaction(tx, &profile, &identity)
    })
}

/// The public R2-02 fixture has one fixed, non-private grant envelope. The
/// operation identity determines its grant ID so a lost reply cannot issue a
/// second grant. This remains private native composition, never an IPC grant
/// constructor; the caller still needs a separately verified Owner ingress.
pub(crate) fn issue_r2_test_owner_delegation_once(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    operation_id: &str,
    input: DelegationGrantInput,
) -> Result<DelegationGrantSnapshot> {
    identifier(operation_id)?;
    input.validate()?;
    let expected = AuthorityCeiling {
        allowed_actions: vec!["delegate".into()],
        allowed_target_principal_ids: vec!["principal-r2-02-worker".into()],
        allowed_target_domain_ids: vec!["domain-r2-02-test".into()],
        allowed_sinks: vec!["task-package".into()],
        allowed_material_classes: vec![],
        explicit_private_material_ids: vec![],
        allowed_continuation_responses: vec![],
        max_material_items: 0,
        max_material_bytes: 0,
        max_response_bytes: 32 * 1024,
    };
    let now = current_epoch_ms()?;
    if input.principal.principal_id != owner.principal_id()
        || input.principal.seat_id != owner.seat_id()
        || input.principal.project_id != "project-r2-02-test"
        || input.principal.domain_id != "domain-r2-02-test"
        || input.principal.role != "controller"
        || input.binding.session_id != "session-r2-02-source"
        || input.binding.execution_id != "execution-r2-02-source"
        || input.binding.generation != "1"
        || input.ceiling != expected
        || input.expires_at_epoch_ms <= now
        || input.expires_at_epoch_ms > now + 3_600_000
    {
        return denied();
    }
    let digest = content_hash(format!("r2-02-test-grant:{R2_V2_MANIFEST_BLOB}:{operation_id}").as_bytes());
    let identity = DelegationGrantIdentity {
        grant_id: format!("grant:r2-02:{}", &digest[7..]),
        revision: "1".into(),
    };
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        validate_owner(owner, &profile)?;
        let heads = tx.query(
            "SELECT revision,revoked FROM main.gogoke_authority_grant_heads WHERE grant_id=?",
            &[&identity.grant_id], 2,
        )?;
        if !heads.is_empty() {
            if heads.len() != 1 || heads[0][0] != "1" || heads[0][1] != "0" {
                return denied();
            }
            let existing = current_in_transaction(tx, &profile, &identity)?;
            if existing.parent.is_some() || existing.principal != input.principal
                || existing.binding != input.binding || existing.ceiling != input.ceiling {
                return Err(OrchestrationError::OperationConflict);
            }
            return Ok(existing);
        }
        insert_grant(tx, &profile, &identity, &input, &profile.issuer_id, None)?;
        tx.write("INSERT INTO main.gogoke_authority_grant_heads(grant_id,revision,revoked) VALUES(?,?,0)",
            &[&identity.grant_id, &identity.revision])?;
        tx.write(
            "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES('ISSUE',?,?,?,?,?)",
            &[&profile.issuer_id, &identity.grant_id, &identity.revision, &profile.policy_revision, &profile.revocation_head])?;
        current_in_transaction(tx, &profile, &identity)
    })
}

/// This in-process entry is currently tied to the authenticated bootstrap Owner.
/// A non-owner seat caller must only be wired through an independently admitted
/// Product Authority caller context; a grant reference is not a bearer identity.
pub(crate) fn delegate_owner_delegation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    parent_identity: &DelegationGrantIdentity,
    input: DelegationGrantInput,
) -> Result<DelegationGrantSnapshot> {
    parent_identity.validate()?;
    input.validate()?;
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        validate_owner(owner, &profile)?;
        let parent = current_in_transaction(tx, &profile, parent_identity)?;
        if parent.principal.principal_id != owner.principal_id()
            || parent.principal.seat_id != owner.seat_id()
            || input.principal.project_id != parent.principal.project_id
            || input.principal.domain_id != parent.principal.domain_id
            || input.expires_at_epoch_ms > parent.expires_at_epoch_ms
            || !input.ceiling.within(&parent.ceiling)
        {
            return denied();
        }
        let identity = DelegationGrantIdentity {
            grant_id: bootstrap::random_id("grant")?,
            revision: "1".into(),
        };
        let issuer_id = seat_issuer(&parent.principal.principal_id, &parent.principal.seat_id);
        insert_grant(
            tx,
            &profile,
            &identity,
            &input,
            &issuer_id,
            Some(parent_identity),
        )?;
        tx.write("INSERT INTO main.gogoke_authority_grant_heads(grant_id,revision,revoked) VALUES(?,?,0)",
            &[&identity.grant_id, &identity.revision])?;
        tx.write(
            "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES('ISSUE',?,?,?,?,?)",
            &[&issuer_id, &identity.grant_id, &identity.revision, &profile.policy_revision, &profile.revocation_head])?;
        current_in_transaction(tx, &profile, &identity)
    })
}

pub(crate) fn read_current_delegation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    grant_id: &str,
) -> Result<DelegationGrantSnapshot> {
    identifier(grant_id)?;
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        let rows = tx.query(
            "SELECT revision FROM main.gogoke_authority_grant_heads WHERE grant_id=?",
            &[grant_id],
            1,
        )?;
        let row = rows.first().ok_or(OrchestrationError::AccessDenied)?;
        if rows.len() != 1 {
            return denied();
        }
        let identity = DelegationGrantIdentity {
            grant_id: grant_id.to_owned(),
            revision: row[0].clone(),
        };
        identity.validate()?;
        current_in_transaction(tx, &profile, &identity)
    })
}

pub(crate) fn revise_owner_delegation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    expected: &DelegationGrantIdentity,
    input: DelegationGrantInput,
) -> Result<DelegationGrantSnapshot> {
    expected.validate()?;
    input.validate()?;
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        validate_owner(owner, &profile)?;
        let old = current_in_transaction(tx, &profile, expected)?;
        if old.parent.is_some() || old.issuer_id != profile.issuer_id {
            return denied();
        }
        let identity = DelegationGrantIdentity {
            grant_id: expected.grant_id.clone(),
            revision: next_revision(&expected.revision)?,
        };
        insert_grant(tx, &profile, &identity, &input, &profile.issuer_id, None)?;
        tx.write("UPDATE main.gogoke_authority_grant_heads SET revision=? WHERE grant_id=? AND revision=? AND revoked=0",
            &[&identity.revision, &identity.grant_id, &expected.revision])?;
        if tx.query("SELECT changes()", &[], 1)?[0][0] != "1" {
            return denied();
        }
        tx.write(
            "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES('REVISE',?,?,?,?,?)",
            &[&profile.issuer_id, &identity.grant_id, &identity.revision, &profile.policy_revision, &profile.revocation_head])?;
        current_in_transaction(tx, &profile, &identity)
    })
}

pub(crate) fn revoke_owner_delegation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    expected: &DelegationGrantIdentity,
) -> Result<String> {
    expected.validate()?;
    transaction::run(connection, |tx| {
        let profile = current_profile(tx)?;
        validate_owner(owner, &profile)?;
        current_in_transaction(tx, &profile, expected)?;
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
        tx.write(
            "INSERT INTO main.gogoke_authority_events(event_kind,issuer_id,grant_id,grant_revision,policy_revision,revocation_head) VALUES('REVOKE',?,?,?,?,?)",
            &[&profile.issuer_id, &expected.grant_id, &expected.revision, &profile.policy_revision, &next])?;
        Ok(next)
    })
}
