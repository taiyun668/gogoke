//! Internal native Product Authority values, never caller-issued authorization.
use super::super::orchestration::OrchestrationError;

type Result<T> = std::result::Result<T, OrchestrationError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrantRef {
    pub grant_id: String,
    pub revision: String,
    pub revocation_head: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct GrantSpec {
    pub principal_id: String,
    pub seat_id: String,
    pub permission: String,
    pub promotion_kind: String,
    pub source_domain_id: String,
    pub destination_domain_id: String,
    pub destination_scope: String,
    pub delegable_depth: u8,
}

#[derive(Clone, Debug)]
pub(super) struct GrantRecord {
    pub reference: GrantRef,
    pub spec: GrantSpec,
    pub issuer_id: String,
    pub parent: Option<(String, String)>,
    pub policy_revision: String,
}

pub(super) fn denied<T>() -> Result<T> {
    Err(OrchestrationError::AccessDenied)
}

pub(super) fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value
            .bytes()
            .all(|x| x.is_ascii_alphanumeric() || b"._:/-".contains(&x))
    {
        return Err(OrchestrationError::Invalid("authority identity"));
    }
    Ok(())
}

pub(super) fn revision(value: &str) -> Result<u64> {
    if value.is_empty()
        || value.len() > 20
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|x| x.is_ascii_digit())
    {
        return Err(OrchestrationError::Invalid("authority revision"));
    }
    value
        .parse()
        .map_err(|_| OrchestrationError::Invalid("authority revision"))
}

pub(super) fn next_revision(value: &str) -> Result<String> {
    revision(value)?
        .checked_add(1)
        .map(|x| x.to_string())
        .ok_or(OrchestrationError::Invalid("authority revision exhausted"))
}

impl GrantRef {
    pub(super) fn validate(&self) -> Result<()> {
        identifier(&self.grant_id)?;
        revision(&self.revision)?;
        revision(&self.revocation_head)?;
        Ok(())
    }
}

impl GrantSpec {
    pub(super) fn validate(&self) -> Result<()> {
        for value in [
            &self.principal_id,
            &self.seat_id,
            &self.promotion_kind,
            &self.source_domain_id,
            &self.destination_domain_id,
        ] {
            identifier(value)?;
        }
        if !matches!(
            self.permission.as_str(),
            "context.read" | "context.promote.source" | "context.promote.target"
        ) || !matches!(
            self.destination_scope.as_str(),
            "GLOBAL" | "PROJECT" | "SESSION"
        ) || self.delegable_depth > 32
        {
            return Err(OrchestrationError::Invalid("authority grant bounds"));
        }
        Ok(())
    }

    // No wildcards or implicit rights. A child may reduce delegation depth and
    // change the grantee, but may not expand or substitute any permission axis.
    pub(super) fn within(&self, parent: &Self) -> bool {
        parent.delegable_depth > 0
            && self.delegable_depth < parent.delegable_depth
            && self.permission == parent.permission
            && self.promotion_kind == parent.promotion_kind
            && self.source_domain_id == parent.source_domain_id
            && self.destination_domain_id == parent.destination_domain_id
            && self.destination_scope == parent.destination_scope
    }
}
