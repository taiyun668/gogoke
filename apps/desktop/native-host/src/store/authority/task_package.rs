//! Durable, typed AuthorizedTaskPackage records on the single Product Authority.
use super::super::atomic::DomainRecordInput;
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::catalog::current_profile;
use super::delegation::{self, AuthorityCeiling, DelegationGrantIdentity, DelegationGrantSnapshot};
use super::material::{self, MaterialVisibility};
use super::model::{denied, identifier, revision};
use super::transaction::{self, Result, Transaction};
use std::collections::BTreeSet;

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const AUTHORITY_STATUS: &str = "PREPARATORY_TRUSTED_MATERIAL_REFS";

const INDEX_SCHEMA: &str = "CREATE TABLE gogoke_authorized_task_packages (domain_id TEXT NOT NULL,operation_id TEXT NOT NULL,package_id TEXT NOT NULL,object_type TEXT NOT NULL CHECK(object_type='AuthorizedTaskPackage'),object_version TEXT NOT NULL CHECK(object_version='1'),package_digest TEXT NOT NULL,event_id TEXT NOT NULL,receipt_id TEXT NOT NULL,recorded_at TEXT NOT NULL,operation_fingerprint TEXT NOT NULL,stream_counter TEXT NOT NULL CHECK(stream_counter='0'),parent_grant_id TEXT NOT NULL,parent_grant_revision TEXT NOT NULL,parent_grant_revocation_head TEXT NOT NULL,parent_policy_revision TEXT NOT NULL,parent_seat_id TEXT NOT NULL,parent_grant_digest TEXT NOT NULL,parent_ceiling_digest TEXT NOT NULL,child_ceiling_digest TEXT NOT NULL,action TEXT NOT NULL,route TEXT NOT NULL,sink TEXT NOT NULL,source_principal_id TEXT NOT NULL,source_project_id TEXT NOT NULL,source_domain_id TEXT NOT NULL,source_role TEXT NOT NULL,target_principal_id TEXT NOT NULL,target_project_id TEXT NOT NULL,target_domain_id TEXT NOT NULL,target_role TEXT NOT NULL,source_session_id TEXT NOT NULL,source_execution_id TEXT NOT NULL,source_generation TEXT NOT NULL,target_session_id TEXT NOT NULL,target_execution_id TEXT NOT NULL,target_generation TEXT NOT NULL,target_binding_kind TEXT NOT NULL,instruction_digest TEXT NOT NULL,material_set_digest TEXT NOT NULL,material_refs_json TEXT NOT NULL,PRIMARY KEY(domain_id,operation_id),UNIQUE(domain_id,package_id),FOREIGN KEY(domain_id,object_type,package_id,object_version) REFERENCES gogoke_objects(domain_id,object_type,object_id,object_version) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskPackagePrincipal {
    pub principal_id: String,
    pub project_id: String,
    pub domain_id: String,
    pub role: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskPackageBinding {
    pub session_id: String,
    pub execution_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedMaterial {
    pub material_id: String,
    pub material_class: String,
    pub visibility: String,
    pub content: String,
    pub content_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TaskMaterialReference {
    pub material_id: String,
    pub revision: String,
}

/// Prepare accepts only the caller's task/package intent. Material facts and
/// all derived package digests are resolved and created inside Product Authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedTaskPackageDraft {
    pub parent_grant_ref: String,
    pub parent_grant_revision: String,
    pub parent_grant_revocation_head: String,
    pub parent_policy_revision: String,
    pub parent_seat_id: String,
    pub child_ceiling: AuthorityCeiling,
    pub action: String,
    pub route: String,
    pub source: TaskPackagePrincipal,
    pub target: TaskPackagePrincipal,
    pub source_binding: TaskPackageBinding,
    pub target_binding: TaskPackageBinding,
    pub target_binding_kind: String,
    pub sink: String,
    pub instruction: String,
}

/// Same field set and JSON spellings as gogoke/policy/types.ts AuthorizedTaskPackage.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedTaskPackage {
    pub package_digest: String,
    pub parent_grant_ref: String,
    pub parent_grant_revision: String,
    pub parent_grant_revocation_head: String,
    pub parent_policy_revision: String,
    pub parent_seat_id: String,
    pub parent_grant_digest: String,
    pub parent_ceiling_digest: String,
    pub child_ceiling: AuthorityCeiling,
    pub child_ceiling_digest: String,
    pub action: String,
    pub route: String,
    pub source: TaskPackagePrincipal,
    pub target: TaskPackagePrincipal,
    pub source_binding: TaskPackageBinding,
    pub target_binding: TaskPackageBinding,
    pub target_binding_kind: String,
    pub sink: String,
    pub instruction: String,
    pub instruction_digest: String,
    pub material_set_digest: String,
    pub materials: Vec<SelectedMaterial>,
}

#[derive(Clone, Debug)]
pub(crate) struct PrepareAuthorizedTaskPackage {
    pub operation_id: String,
    pub domain_id: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
    pub package: AuthorizedTaskPackageDraft,
    pub material_refs: Vec<TaskMaterialReference>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AuthorizedTaskPackageReceipt {
    pub disposition: &'static str,
    pub authority_status: &'static str,
    pub operation_id: String,
    pub package_id: String,
    pub package_digest: String,
    pub canonical_package: Vec<u8>,
}

struct StoredAuthorizedTaskPackage {
    package: AuthorizedTaskPackage,
    material_refs: Vec<TaskMaterialReference>,
    receipt_id: String,
    event_id: String,
    recorded_at: String,
    operation_fingerprint: String,
    canonical_package: Vec<u8>,
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    let rows = tx.query(
        "SELECT type,sql FROM main.sqlite_schema WHERE name='gogoke_authorized_task_packages'",
        &[],
        2,
    )?;
    if rows.is_empty() {
        tx.write(INDEX_SCHEMA, &[])?;
    } else if rows.len() != 1 || rows[0][0] != "table" || rows[0][1] != INDEX_SCHEMA {
        return denied();
    }
    if !tx.query("SELECT 1 FROM temp.sqlite_schema WHERE (type IN ('table','view') AND lower(name) IN ('gogoke_authorized_task_packages','gogoke_objects','gogoke_events','gogoke_receipts','gogoke_stream_heads')) OR (type='trigger' AND lower(tbl_name) IN ('gogoke_authorized_task_packages','gogoke_objects','gogoke_events','gogoke_receipts','gogoke_stream_heads')) LIMIT 1", &[], 1)?.is_empty()
        || !tx.query("SELECT 1 FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name) IN ('gogoke_authorized_task_packages','gogoke_objects','gogoke_events','gogoke_receipts','gogoke_stream_heads') LIMIT 1", &[], 1)?.is_empty()
        || !tx.query("SELECT 1 FROM main.sqlite_schema WHERE type='index' AND lower(tbl_name)='gogoke_authorized_task_packages' AND sql IS NOT NULL LIMIT 1", &[], 1)?.is_empty()
    { return denied(); }
    Ok(())
}

pub(crate) fn initialize_authorized_task_package_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<()> {
    transaction::run(connection, ensure_schema)
}

// Canonical encoding and authority validation are implemented below; keeping
// each JSON encoder dedicated avoids serde/runtime-specific number formatting.
fn quote(value: &str) -> String {
    let mut output = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            ch if (ch as u32) < 0x20 => output.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => output.push(ch),
        }
    }
    output.push('"');
    output
}

pub(super) fn digest_canonical(value: &str) -> String {
    content_hash(value.as_bytes())
}
fn hash(value: &str) -> Result<()> {
    if value.len() != 71
        || !value.starts_with("sha256:")
        || !value.as_bytes()[7..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return denied();
    }
    Ok(())
}

fn canonical_strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| quote(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn ceiling_json(value: &AuthorityCeiling) -> String {
    format!(
        "{{\"allowedActions\":{},\"allowedContinuationResponses\":{},\"allowedMaterialClasses\":{},\"allowedSinks\":{},\"allowedTargetDomainIds\":{},\"allowedTargetPrincipalIds\":{},\"explicitPrivateMaterialIds\":{},\"maxMaterialBytes\":{},\"maxMaterialItems\":{},\"maxResponseBytes\":{}}}",
        canonical_strings(&value.allowed_actions),
        canonical_strings(&value.allowed_continuation_responses),
        canonical_strings(&value.allowed_material_classes),
        canonical_strings(&value.allowed_sinks),
        canonical_strings(&value.allowed_target_domain_ids),
        canonical_strings(&value.allowed_target_principal_ids),
        canonical_strings(&value.explicit_private_material_ids),
        value.max_material_bytes,
        value.max_material_items,
        value.max_response_bytes
    )
}

fn principal_json(value: &TaskPackagePrincipal) -> String {
    format!(
        "{{\"domainId\":{},\"principalId\":{},\"projectId\":{},\"role\":{}}}",
        quote(&value.domain_id),
        quote(&value.principal_id),
        quote(&value.project_id),
        quote(&value.role)
    )
}

fn binding_json(value: &TaskPackageBinding) -> String {
    format!(
        "{{\"executionId\":{},\"generation\":{},\"sessionId\":{}}}",
        quote(&value.execution_id),
        quote(&value.generation),
        quote(&value.session_id)
    )
}

fn material_json(value: &SelectedMaterial) -> String {
    format!(
        "{{\"content\":{},\"contentDigest\":{},\"materialClass\":{},\"materialId\":{},\"visibility\":{}}}",
        quote(&value.content),
        quote(&value.content_digest),
        quote(&value.material_class),
        quote(&value.material_id),
        quote(&value.visibility)
    )
}

fn materials_json(values: &[SelectedMaterial]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(material_json)
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn materials_preimage(values: &[SelectedMaterial]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!(
                "{{\"contentDigest\":{},\"materialClass\":{},\"materialId\":{},\"visibility\":{}}}",
                quote(&value.content_digest),
                quote(&value.material_class),
                quote(&value.material_id),
                quote(&value.visibility)
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn package_preimage(value: &AuthorizedTaskPackage) -> String {
    format!(
        "{{\"action\":{},\"childCeiling\":{},\"childCeilingDigest\":{},\"instruction\":{},\"instructionDigest\":{},\"materialSetDigest\":{},\"materials\":{},\"parentCeilingDigest\":{},\"parentGrantDigest\":{},\"parentGrantRef\":{},\"parentGrantRevision\":{},\"parentGrantRevocationHead\":{},\"parentPolicyRevision\":{},\"parentSeatId\":{},\"route\":{},\"sink\":{},\"source\":{},\"sourceBinding\":{},\"target\":{},\"targetBinding\":{},\"targetBindingKind\":{}}}",
        quote(&value.action),
        ceiling_json(&value.child_ceiling),
        quote(&value.child_ceiling_digest),
        quote(&value.instruction),
        quote(&value.instruction_digest),
        quote(&value.material_set_digest),
        materials_json(&value.materials),
        quote(&value.parent_ceiling_digest),
        quote(&value.parent_grant_digest),
        quote(&value.parent_grant_ref),
        quote(&value.parent_grant_revision),
        quote(&value.parent_grant_revocation_head),
        quote(&value.parent_policy_revision),
        quote(&value.parent_seat_id),
        quote(&value.route),
        quote(&value.sink),
        principal_json(&value.source),
        binding_json(&value.source_binding),
        principal_json(&value.target),
        binding_json(&value.target_binding),
        quote(&value.target_binding_kind)
    )
}

pub(super) fn package_json(value: &AuthorizedTaskPackage) -> Vec<u8> {
    let preimage = package_preimage(value);
    let digest = digest_canonical(&preimage);
    format!("{{\"action\":{},\"childCeiling\":{},\"childCeilingDigest\":{},\"instruction\":{},\"instructionDigest\":{},\"materialSetDigest\":{},\"materials\":{},\"packageDigest\":{},\"parentCeilingDigest\":{},\"parentGrantDigest\":{},\"parentGrantRef\":{},\"parentGrantRevision\":{},\"parentGrantRevocationHead\":{},\"parentPolicyRevision\":{},\"parentSeatId\":{},\"route\":{},\"sink\":{},\"source\":{},\"sourceBinding\":{},\"target\":{},\"targetBinding\":{},\"targetBindingKind\":{}}}",
        quote(&value.action), ceiling_json(&value.child_ceiling), quote(&value.child_ceiling_digest),
        quote(&value.instruction), quote(&value.instruction_digest), quote(&value.material_set_digest),
        materials_json(&value.materials), quote(&digest), quote(&value.parent_ceiling_digest),
        quote(&value.parent_grant_digest), quote(&value.parent_grant_ref), quote(&value.parent_grant_revision),
        quote(&value.parent_grant_revocation_head), quote(&value.parent_policy_revision), quote(&value.parent_seat_id),
        quote(&value.route), quote(&value.sink), principal_json(&value.source), binding_json(&value.source_binding),
        principal_json(&value.target), binding_json(&value.target_binding), quote(&value.target_binding_kind)).into_bytes()
}

fn grant_json(value: &DelegationGrantSnapshot) -> String {
    let parent = value
        .parent
        .as_ref()
        .map(|reference| {
            format!(
                "{{\"grantRef\":{},\"revision\":{}}}",
                quote(&reference.grant_id),
                quote(&reference.revision)
            )
        })
        .unwrap_or_else(|| "null".into());
    format!(
        "{{\"binding\":{},\"ceiling\":{},\"expiresAtEpochMs\":{},\"grantRef\":{},\"issuerId\":{},\"parentGrant\":{},\"policyRevision\":{},\"principal\":{{\"domainId\":{},\"principalId\":{},\"projectId\":{},\"role\":{}}},\"revision\":{},\"revocationHead\":{},\"seatId\":{}}}",
        binding_json(&TaskPackageBinding {
            session_id: value.binding.session_id.clone(),
            execution_id: value.binding.execution_id.clone(),
            generation: value.binding.generation.clone()
        }),
        ceiling_json(&value.ceiling),
        value.expires_at_epoch_ms,
        quote(&value.reference.grant_id),
        quote(&value.issuer_id),
        parent,
        quote(&value.policy_revision),
        quote(&value.principal.domain_id),
        quote(&value.principal.principal_id),
        quote(&value.principal.project_id),
        quote(&value.principal.role),
        quote(&value.reference.revision),
        quote(&value.reference.revocation_head),
        quote(&value.principal.seat_id)
    )
}

fn valid_axes(values: &[String]) -> Result<()> {
    let mut unique = BTreeSet::new();
    for value in values {
        identifier(value)?;
        if !unique.insert(value) {
            return denied();
        }
    }
    Ok(())
}

fn valid_ceiling(value: &AuthorityCeiling) -> Result<()> {
    for axis in [
        &value.allowed_actions,
        &value.allowed_target_principal_ids,
        &value.allowed_target_domain_ids,
        &value.allowed_sinks,
        &value.allowed_material_classes,
        &value.explicit_private_material_ids,
        &value.allowed_continuation_responses,
    ] {
        valid_axes(axis)?;
    }
    if [
        value.max_material_items,
        value.max_material_bytes,
        value.max_response_bytes,
    ]
    .iter()
    .any(|n| *n > MAX_SAFE_INTEGER)
    {
        return denied();
    }
    if value.allowed_actions.iter().any(|v| {
        !matches!(
            v.as_str(),
            "delegate"
                | "return-result"
                | "request-review"
                | "share-material"
                | "answer-continuation"
                | "cancel-continuation"
        )
    }) || value.allowed_sinks.iter().any(|v| {
        !matches!(
            v.as_str(),
            "task-package"
                | "stdin"
                | "rules"
                | "files"
                | "log"
                | "public-stream"
                | "controller"
                | "notification"
                | "export"
                | "cache"
                | "restore"
                | "formal-review"
        )
    }) {
        return denied();
    }
    Ok(())
}

fn within(child: &AuthorityCeiling, parent: &AuthorityCeiling) -> bool {
    child
        .allowed_actions
        .iter()
        .all(|v| parent.allowed_actions.contains(v))
        && child
            .allowed_target_principal_ids
            .iter()
            .all(|v| parent.allowed_target_principal_ids.contains(v))
        && child
            .allowed_target_domain_ids
            .iter()
            .all(|v| parent.allowed_target_domain_ids.contains(v))
        && child
            .allowed_sinks
            .iter()
            .all(|v| parent.allowed_sinks.contains(v))
        && child
            .allowed_material_classes
            .iter()
            .all(|v| parent.allowed_material_classes.contains(v))
        && child
            .explicit_private_material_ids
            .iter()
            .all(|v| parent.explicit_private_material_ids.contains(v))
        && child
            .allowed_continuation_responses
            .iter()
            .all(|v| parent.allowed_continuation_responses.contains(v))
        && child.max_material_items <= parent.max_material_items
        && child.max_material_bytes <= parent.max_material_bytes
        && child.max_response_bytes <= parent.max_response_bytes
}

fn valid_identity_text(value: &str) -> Result<()> {
    if value.is_empty() || value.trim() != value {
        return denied();
    }
    Ok(())
}

fn validate_current_parent(
    tx: &mut Transaction<'_, '_>,
    package: &AuthorizedTaskPackage,
) -> Result<DelegationGrantSnapshot> {
    let current = resolve_current_parent(
        tx,
        &package.parent_grant_ref,
        &package.parent_grant_revision,
        &package.parent_grant_revocation_head,
        &package.parent_policy_revision,
        &package.parent_seat_id,
    )?;
    if package.parent_grant_digest != digest_canonical(&grant_json(&current))
        || package.parent_ceiling_digest != digest_canonical(&ceiling_json(&current.ceiling))
    {
        return denied();
    }
    Ok(current)
}

fn resolve_current_parent(
    tx: &mut Transaction<'_, '_>,
    grant_ref: &str,
    grant_revision: &str,
    revocation_head: &str,
    policy_revision: &str,
    seat_id: &str,
) -> Result<DelegationGrantSnapshot> {
    let profile = current_profile(tx)?;
    let identity = DelegationGrantIdentity {
        grant_id: grant_ref.to_owned(),
        revision: grant_revision.to_owned(),
    };
    let current = delegation::current_in_transaction(tx, &profile, &identity)?;
    if grant_ref != current.reference.grant_id
        || grant_revision != current.reference.revision
        || revocation_head != current.reference.revocation_head
        || policy_revision != current.policy_revision
        || seat_id != current.principal.seat_id
    {
        return denied();
    }
    Ok(current)
}

pub(super) fn material_refs_json(values: &[TaskMaterialReference]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| format!(
                "{{\"materialId\":{},\"revision\":{}}}",
                quote(&value.material_id),
                quote(&value.revision)
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn parse_material_refs(
    tx: &mut Transaction<'_, '_>,
    encoded: &str,
) -> Result<Vec<TaskMaterialReference>> {
    let rows = ordered_json_array(tx, encoded, "$")?;
    let mut refs = Vec::with_capacity(rows.len());
    for (value_type, row) in rows {
        if value_type != "object" {
            return denied();
        }
        exact_keys(tx, &row, None, &["materialId", "revision"])?;
        refs.push(TaskMaterialReference {
            material_id: package_json_type(tx, &row, "$.materialId", "text")?,
            revision: package_json_type(tx, &row, "$.revision", "text")?,
        });
    }
    if material_refs_json(&refs) != encoded {
        return denied();
    }
    Ok(refs)
}

fn resolve_material_refs(
    tx: &mut Transaction<'_, '_>,
    refs: &[TaskMaterialReference],
    draft: &AuthorizedTaskPackageDraft,
    current: &DelegationGrantSnapshot,
) -> Result<Vec<SelectedMaterial>> {
    if refs.len() as u64 > current.ceiling.max_material_items
        || refs.len() as u64 > draft.child_ceiling.max_material_items
    {
        return denied();
    }
    let mut seen = BTreeSet::new();
    let mut total_bytes = 0usize;
    let mut selected = Vec::with_capacity(refs.len());
    for reference in refs {
        identifier(&reference.material_id)?;
        revision(&reference.revision)?;
        if !seen.insert(reference.material_id.as_str()) {
            return denied();
        }
        let stored =
            material::read_current_for_authorized_task_package(tx, &reference.material_id)?;
        let value = &stored.material;
        if stored.revision != reference.revision
            || value.project_id != draft.source.project_id
            || value.project_id != draft.target.project_id
        {
            return denied();
        }
        identifier(&stored.provenance_ref)?;
        let visibility = value.visibility.as_str();
        if !current
            .ceiling
            .allowed_material_classes
            .contains(&value.material_class)
            || !draft
                .child_ceiling
                .allowed_material_classes
                .contains(&value.material_class)
            || (stored.material.visibility == MaterialVisibility::Private
                && (!current
                    .ceiling
                    .explicit_private_material_ids
                    .contains(&value.material_id)
                    || !draft
                        .child_ceiling
                        .explicit_private_material_ids
                        .contains(&value.material_id)))
            || (draft.action == "request-review" && visibility == "private")
        {
            return denied();
        }
        total_bytes = total_bytes
            .checked_add(value.content.as_bytes().len())
            .ok_or(OrchestrationError::AccessDenied)?;
        if total_bytes > current.ceiling.max_material_bytes as usize
            || total_bytes > draft.child_ceiling.max_material_bytes as usize
        {
            return denied();
        }
        selected.push(SelectedMaterial {
            material_id: value.material_id.clone(),
            material_class: value.material_class.clone(),
            visibility: visibility.to_owned(),
            content: value.content.clone(),
            content_digest: digest_canonical(&quote(&value.content)),
        });
    }
    Ok(selected)
}

fn package_from_draft(
    draft: &AuthorizedTaskPackageDraft,
    current: &DelegationGrantSnapshot,
    materials: Vec<SelectedMaterial>,
) -> AuthorizedTaskPackage {
    let value = AuthorizedTaskPackage {
        package_digest: String::new(),
        parent_grant_ref: draft.parent_grant_ref.clone(),
        parent_grant_revision: draft.parent_grant_revision.clone(),
        parent_grant_revocation_head: draft.parent_grant_revocation_head.clone(),
        parent_policy_revision: draft.parent_policy_revision.clone(),
        parent_seat_id: draft.parent_seat_id.clone(),
        parent_grant_digest: String::new(),
        parent_ceiling_digest: String::new(),
        child_ceiling: draft.child_ceiling.clone(),
        child_ceiling_digest: String::new(),
        action: draft.action.clone(),
        route: draft.route.clone(),
        source: draft.source.clone(),
        target: draft.target.clone(),
        source_binding: draft.source_binding.clone(),
        target_binding: draft.target_binding.clone(),
        target_binding_kind: draft.target_binding_kind.clone(),
        sink: draft.sink.clone(),
        instruction: draft.instruction.clone(),
        instruction_digest: String::new(),
        material_set_digest: String::new(),
        materials,
    };
    package_from_current(&value, current)
}

fn draft_from_package(value: &AuthorizedTaskPackage) -> AuthorizedTaskPackageDraft {
    AuthorizedTaskPackageDraft {
        parent_grant_ref: value.parent_grant_ref.clone(),
        parent_grant_revision: value.parent_grant_revision.clone(),
        parent_grant_revocation_head: value.parent_grant_revocation_head.clone(),
        parent_policy_revision: value.parent_policy_revision.clone(),
        parent_seat_id: value.parent_seat_id.clone(),
        child_ceiling: value.child_ceiling.clone(),
        action: value.action.clone(),
        route: value.route.clone(),
        source: value.source.clone(),
        target: value.target.clone(),
        source_binding: value.source_binding.clone(),
        target_binding: value.target_binding.clone(),
        target_binding_kind: value.target_binding_kind.clone(),
        sink: value.sink.clone(),
        instruction: value.instruction.clone(),
    }
}

fn validate_package(
    value: &AuthorizedTaskPackage,
    current: &DelegationGrantSnapshot,
) -> Result<()> {
    for v in [
        &value.parent_grant_ref,
        &value.parent_seat_id,
        &value.action,
        &value.route,
        &value.sink,
        &value.target_binding_kind,
        &value.source.principal_id,
        &value.source.project_id,
        &value.source.domain_id,
        &value.source.role,
        &value.target.principal_id,
        &value.target.project_id,
        &value.target.domain_id,
        &value.target.role,
        &value.source_binding.session_id,
        &value.source_binding.execution_id,
        &value.target_binding.session_id,
        &value.target_binding.execution_id,
    ] {
        valid_identity_text(v)?;
    }
    for identity in [
        &value.source.principal_id,
        &value.source.project_id,
        &value.source.domain_id,
        &value.source_binding.session_id,
        &value.source_binding.execution_id,
        &value.target.principal_id,
        &value.target.project_id,
        &value.target.domain_id,
        &value.target_binding.session_id,
        &value.target_binding.execution_id,
    ] {
        identifier(identity)?;
    }
    identifier(&value.parent_grant_ref)?;
    identifier(&value.parent_seat_id)?;
    for v in [
        &value.parent_grant_revision,
        &value.parent_grant_revocation_head,
        &value.parent_policy_revision,
        &value.source_binding.generation,
        &value.target_binding.generation,
    ] {
        revision(v)?;
    }
    for v in [
        &value.parent_grant_digest,
        &value.parent_ceiling_digest,
        &value.child_ceiling_digest,
        &value.instruction_digest,
        &value.material_set_digest,
    ] {
        hash(v)?;
    }
    valid_identity_text(&value.instruction)?;
    valid_ceiling(&value.child_ceiling)?;
    valid_ceiling(&current.ceiling)?;
    if !within(&value.child_ceiling, &current.ceiling) {
        return denied();
    }
    let source_matches = value.source.principal_id == current.principal.principal_id
        && value.source.project_id == current.principal.project_id
        && value.source.domain_id == current.principal.domain_id
        && value.source.role == current.principal.role
        && value.source_binding.session_id == current.binding.session_id
        && value.source_binding.execution_id == current.binding.execution_id
        && value.source_binding.generation == current.binding.generation;
    if !source_matches
        || value.source.project_id != value.target.project_id
        || !matches!(
            value.source.role.as_str(),
            "controller" | "worker" | "auditor"
        )
        || !matches!(
            value.target.role.as_str(),
            "controller" | "worker" | "auditor"
        )
    {
        return denied();
    }
    let target_allowed = current
        .ceiling
        .allowed_target_principal_ids
        .contains(&value.target.principal_id)
        && current
            .ceiling
            .allowed_target_domain_ids
            .contains(&value.target.domain_id);
    if !target_allowed
        || !current.ceiling.allowed_actions.contains(&value.action)
        || !current.ceiling.allowed_sinks.contains(&value.sink)
        || !value.child_ceiling.allowed_actions.contains(&value.action)
        || !value
            .child_ceiling
            .allowed_target_principal_ids
            .contains(&value.target.principal_id)
        || !value
            .child_ceiling
            .allowed_target_domain_ids
            .contains(&value.target.domain_id)
        || !value.child_ceiling.allowed_sinks.contains(&value.sink)
    {
        return denied();
    }
    let route_ok = match value.action.as_str() {
        "delegate" => {
            value.route == "controller-worker"
                && value.source.role == "controller"
                && value.target.role == "worker"
                && value.sink == "task-package"
        }
        "return-result" => {
            value.route == "worker-controller"
                && value.source.role == "worker"
                && value.target.role == "controller"
                && value.sink == "task-package"
        }
        "request-review" => {
            value.route == "controller-clean-review"
                && value.source.role == "controller"
                && value.target.role == "auditor"
                && value.target_binding_kind == "new-clean"
                && value.sink == "formal-review"
        }
        _ => false,
    };
    if !route_ok || !matches!(value.target_binding_kind.as_str(), "existing" | "new-clean") {
        return denied();
    }
    if value.target_binding_kind == "new-clean" && value.source_binding == value.target_binding {
        return denied();
    }
    let mut material_ids = BTreeSet::new();
    let mut total_bytes = 0usize;
    for material in &value.materials {
        identifier(&material.material_id)?;
        identifier(&material.material_class)?;
        if !material_ids.insert(&material.material_id)
            || !matches!(material.visibility.as_str(), "project" | "private")
        {
            return denied();
        }
        hash(&material.content_digest)?;
        if digest_canonical(&quote(&material.content)) != material.content_digest {
            return denied();
        }
        if !current
            .ceiling
            .allowed_material_classes
            .contains(&material.material_class)
            || !value
                .child_ceiling
                .allowed_material_classes
                .contains(&material.material_class)
            || (material.visibility == "private"
                && (!current
                    .ceiling
                    .explicit_private_material_ids
                    .contains(&material.material_id)
                    || !value
                        .child_ceiling
                        .explicit_private_material_ids
                        .contains(&material.material_id)))
        {
            return denied();
        }
        total_bytes = total_bytes
            .checked_add(material.content.as_bytes().len())
            .ok_or(OrchestrationError::AccessDenied)?;
        if total_bytes > current.ceiling.max_material_bytes as usize
            || total_bytes > value.child_ceiling.max_material_bytes as usize
        {
            return denied();
        }
    }
    if value.materials.len() as u64 > current.ceiling.max_material_items
        || value.materials.len() as u64 > value.child_ceiling.max_material_items
    {
        return denied();
    }
    if value.action == "request-review" && value.materials.iter().any(|m| m.visibility == "private")
    {
        return denied();
    }
    if package_from_current(value, current) != *value {
        return denied();
    }
    Ok(())
}

pub(super) fn package_from_current(
    value: &AuthorizedTaskPackage,
    current: &DelegationGrantSnapshot,
) -> AuthorizedTaskPackage {
    let mut package = value.clone();
    package.parent_grant_digest = digest_canonical(&grant_json(current));
    package.parent_ceiling_digest = digest_canonical(&ceiling_json(&current.ceiling));
    package.child_ceiling_digest = digest_canonical(&ceiling_json(&package.child_ceiling));
    package.instruction_digest = digest_canonical(&quote(&package.instruction));
    package.material_set_digest = digest_canonical(&materials_preimage(&package.materials));
    package.package_digest = digest_canonical(&package_preimage(&package));
    package
}

fn index_values(
    input: &PrepareAuthorizedTaskPackage,
    package: &AuthorizedTaskPackage,
    refs_json: &str,
    operation_fingerprint: &str,
) -> Vec<String> {
    let p = package;
    vec![
        input.domain_id.clone(),
        input.operation_id.clone(),
        p.package_digest.clone(),
        "AuthorizedTaskPackage".into(),
        "1".into(),
        p.package_digest.clone(),
        input.event_id.clone(),
        input.receipt_id.clone(),
        input.recorded_at.clone(),
        operation_fingerprint.to_owned(),
        "0".into(),
        p.parent_grant_ref.clone(),
        p.parent_grant_revision.clone(),
        p.parent_grant_revocation_head.clone(),
        p.parent_policy_revision.clone(),
        p.parent_seat_id.clone(),
        p.parent_grant_digest.clone(),
        p.parent_ceiling_digest.clone(),
        p.child_ceiling_digest.clone(),
        p.action.clone(),
        p.route.clone(),
        p.sink.clone(),
        p.source.principal_id.clone(),
        p.source.project_id.clone(),
        p.source.domain_id.clone(),
        p.source.role.clone(),
        p.target.principal_id.clone(),
        p.target.project_id.clone(),
        p.target.domain_id.clone(),
        p.target.role.clone(),
        p.source_binding.session_id.clone(),
        p.source_binding.execution_id.clone(),
        p.source_binding.generation.clone(),
        p.target_binding.session_id.clone(),
        p.target_binding.execution_id.clone(),
        p.target_binding.generation.clone(),
        p.target_binding_kind.clone(),
        p.instruction_digest.clone(),
        p.material_set_digest.clone(),
        refs_json.to_owned(),
    ]
}

fn insert_index(
    tx: &mut Transaction<'_, '_>,
    input: &PrepareAuthorizedTaskPackage,
    package: &AuthorizedTaskPackage,
    refs_json: &str,
    operation_fingerprint: &str,
) -> Result<()> {
    let fields = index_values(input, package, refs_json, operation_fingerprint);
    let refs = fields.iter().map(String::as_str).collect::<Vec<_>>();
    tx.write("INSERT INTO main.gogoke_authorized_task_packages(domain_id,operation_id,package_id,object_type,object_version,package_digest,event_id,receipt_id,recorded_at,operation_fingerprint,stream_counter,parent_grant_id,parent_grant_revision,parent_grant_revocation_head,parent_policy_revision,parent_seat_id,parent_grant_digest,parent_ceiling_digest,child_ceiling_digest,action,route,sink,source_principal_id,source_project_id,source_domain_id,source_role,target_principal_id,target_project_id,target_domain_id,target_role,source_session_id,source_execution_id,source_generation,target_session_id,target_execution_id,target_generation,target_binding_kind,instruction_digest,material_set_digest,material_refs_json) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)", &refs)
}

fn package_event(
    operation_id: &str,
    package: &AuthorizedTaskPackage,
    material_refs_digest: &str,
) -> Vec<u8> {
    format!(
        "{{\"materialRefsDigest\":{},\"operationId\":{},\"packageDigest\":{},\"parentGrantDigest\":{}}}",
        quote(material_refs_digest),
        quote(operation_id),
        quote(&package.package_digest),
        quote(&package.parent_grant_digest)
    )
    .into_bytes()
}

fn package_receipt(
    operation_id: &str,
    package: &AuthorizedTaskPackage,
    material_refs_digest: &str,
) -> Vec<u8> {
    format!(
        "{{\"authorityStatus\":{},\"materialRefsDigest\":{},\"operationId\":{},\"packageDigest\":{},\"schema\":\"gogoke.authorized-task-package-prepare.v1\"}}",
        quote(AUTHORITY_STATUS),
        quote(material_refs_digest),
        quote(operation_id),
        quote(&package.package_digest)
    )
    .into_bytes()
}

fn record_input(
    domain_id: &str,
    operation_id: &str,
    event_id: &str,
    receipt_id: &str,
    recorded_at: &str,
    package: &AuthorizedTaskPackage,
    material_refs_digest: &str,
) -> DomainRecordInput {
    let package_id = package.package_digest.clone();
    DomainRecordInput {
        domain_id: domain_id.to_owned(),
        object_type: "AuthorizedTaskPackage".into(),
        object_id: package_id.clone(),
        object_version: "1".into(),
        object_bytes: package_json(package),
        native_identity: None,
        event_id: event_id.to_owned(),
        stream_id: format!("gogoke.authorized-task-package.v1/{package_id}"),
        expected_previous_counter: None,
        counter: "0".into(),
        event_type: "AuthorizedTaskPackagePrepared".into(),
        occurred_at: recorded_at.to_owned(),
        event_bytes: package_event(operation_id, package, material_refs_digest),
        receipt_id: receipt_id.to_owned(),
        operation_id: operation_id.to_owned(),
        receipt_type: "AuthorizedTaskPackagePrepared".into(),
        recorded_at: recorded_at.to_owned(),
        receipt_bytes: package_receipt(operation_id, package, material_refs_digest),
    }
}

fn package_json_type(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
    expected: &str,
) -> Result<String> {
    let rows = tx.query(
        "SELECT json_type(?,?),CAST(json_extract(?,?) AS TEXT)",
        &[json, path, json, path],
        2,
    )?;
    if rows.len() != 1 || rows[0][0] != expected {
        return denied();
    }
    Ok(rows[0][1].clone())
}

fn exact_keys(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: Option<&str>,
    expected: &[&str],
) -> Result<()> {
    let rows = match path {
        Some(path) => tx.query("SELECT key FROM json_each(?,?)", &[json, path], 1)?,
        None => tx.query("SELECT key FROM json_each(?)", &[json], 1)?,
    };
    if rows.len() != expected.len() {
        return denied();
    }
    let actual = rows
        .into_iter()
        .map(|r| r[0].clone())
        .collect::<BTreeSet<_>>();
    if expected.iter().any(|key| !actual.contains(*key)) {
        return denied();
    }
    Ok(())
}

fn parse_principal(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<TaskPackagePrincipal> {
    exact_keys(
        tx,
        json,
        Some(path),
        &["domainId", "principalId", "projectId", "role"],
    )?;
    Ok(TaskPackagePrincipal {
        domain_id: package_json_type(tx, json, &format!("{path}.domainId"), "text")?,
        principal_id: package_json_type(tx, json, &format!("{path}.principalId"), "text")?,
        project_id: package_json_type(tx, json, &format!("{path}.projectId"), "text")?,
        role: package_json_type(tx, json, &format!("{path}.role"), "text")?,
    })
}

fn parse_binding(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<TaskPackageBinding> {
    exact_keys(
        tx,
        json,
        Some(path),
        &["executionId", "generation", "sessionId"],
    )?;
    Ok(TaskPackageBinding {
        execution_id: package_json_type(tx, json, &format!("{path}.executionId"), "text")?,
        generation: package_json_type(tx, json, &format!("{path}.generation"), "text")?,
        session_id: package_json_type(tx, json, &format!("{path}.sessionId"), "text")?,
    })
}

fn decode_ordered_json_array(encoded: &str) -> Result<Vec<(String, String)>> {
    let mut values = Vec::new();
    let mut remaining = encoded;
    while !remaining.is_empty() {
        let ordinal_end = remaining
            .find(':')
            .ok_or(OrchestrationError::AccessDenied)?;
        let ordinal_text = &remaining[..ordinal_end];
        let ordinal = ordinal_text
            .parse::<usize>()
            .map_err(|_| OrchestrationError::AccessDenied)?;
        if ordinal != values.len() || ordinal_text != ordinal.to_string() {
            return denied();
        }
        let after_ordinal = &remaining[ordinal_end + 1..];
        let type_end = after_ordinal
            .find(':')
            .ok_or(OrchestrationError::AccessDenied)?;
        let value_type = &after_ordinal[..type_end];
        if value_type.is_empty() || !value_type.bytes().all(|byte| byte.is_ascii_lowercase()) {
            return denied();
        }
        let after_type = &after_ordinal[type_end + 1..];
        let length_end = after_type
            .find(':')
            .ok_or(OrchestrationError::AccessDenied)?;
        let length_text = &after_type[..length_end];
        let length = length_text
            .parse::<usize>()
            .map_err(|_| OrchestrationError::AccessDenied)?;
        if length_text != length.to_string() {
            return denied();
        }
        let payload = &after_type[length_end + 1..];
        if length > payload.len() || !payload.is_char_boundary(length) {
            return denied();
        }
        values.push((value_type.to_owned(), payload[..length].to_owned()));
        remaining = &payload[length..];
    }
    Ok(values)
}

fn ordered_json_array(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<Vec<(String, String)>> {
    if package_json_type(tx, json, path, "array").is_err() {
        return denied();
    }
    // Keep the row cardinality at one regardless of the JSON array size. The
    // ordinal and UTF-8 byte length delimit values even when JSON bodies contain
    // colons, commas, or non-ASCII text.
    let rows = tx.query(
        "SELECT COALESCE(group_concat(CAST(key AS TEXT)||':'||type||':'||length(CAST(value AS BLOB))||':'||CAST(value AS TEXT),''),'') FROM (SELECT key,type,value FROM json_each(?,?) ORDER BY CAST(key AS INTEGER))",
        &[json, path],
        1,
    )?;
    if rows.len() != 1 {
        return denied();
    }
    decode_ordered_json_array(&rows[0][0])
}

fn parse_string_array(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<Vec<String>> {
    let rows = ordered_json_array(tx, json, path)?;
    let mut result = Vec::with_capacity(rows.len());
    for (value_type, value) in rows {
        if value_type != "text" {
            return denied();
        }
        result.push(value);
    }
    valid_axes(&result)?;
    Ok(result)
}

fn parse_number(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<u64> {
    let value = package_json_type(tx, json, path, "integer")?;
    let number = value
        .parse::<u64>()
        .map_err(|_| OrchestrationError::AccessDenied)?;
    if number > MAX_SAFE_INTEGER {
        return denied();
    }
    Ok(number)
}

fn parse_ceiling(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<AuthorityCeiling> {
    exact_keys(
        tx,
        json,
        Some(path),
        &[
            "allowedActions",
            "allowedContinuationResponses",
            "allowedMaterialClasses",
            "allowedSinks",
            "allowedTargetDomainIds",
            "allowedTargetPrincipalIds",
            "explicitPrivateMaterialIds",
            "maxMaterialBytes",
            "maxMaterialItems",
            "maxResponseBytes",
        ],
    )?;
    let value = AuthorityCeiling {
        allowed_actions: parse_string_array(tx, json, &format!("{path}.allowedActions"))?,
        allowed_target_principal_ids: parse_string_array(
            tx,
            json,
            &format!("{path}.allowedTargetPrincipalIds"),
        )?,
        allowed_target_domain_ids: parse_string_array(
            tx,
            json,
            &format!("{path}.allowedTargetDomainIds"),
        )?,
        allowed_sinks: parse_string_array(tx, json, &format!("{path}.allowedSinks"))?,
        allowed_material_classes: parse_string_array(
            tx,
            json,
            &format!("{path}.allowedMaterialClasses"),
        )?,
        explicit_private_material_ids: parse_string_array(
            tx,
            json,
            &format!("{path}.explicitPrivateMaterialIds"),
        )?,
        allowed_continuation_responses: parse_string_array(
            tx,
            json,
            &format!("{path}.allowedContinuationResponses"),
        )?,
        max_material_items: parse_number(tx, json, &format!("{path}.maxMaterialItems"))?,
        max_material_bytes: parse_number(tx, json, &format!("{path}.maxMaterialBytes"))?,
        max_response_bytes: parse_number(tx, json, &format!("{path}.maxResponseBytes"))?,
    };
    valid_ceiling(&value)?;
    Ok(value)
}

fn parse_materials(tx: &mut Transaction<'_, '_>, json: &str) -> Result<Vec<SelectedMaterial>> {
    let rows = ordered_json_array(tx, json, "$.materials")?;
    let mut materials = Vec::with_capacity(rows.len());
    for (value_type, row) in rows {
        if value_type != "object" {
            return denied();
        }
        exact_keys(
            tx,
            &row,
            None,
            &[
                "content",
                "contentDigest",
                "materialClass",
                "materialId",
                "visibility",
            ],
        )?;
        materials.push(SelectedMaterial {
            content: package_json_type(tx, &row, "$.content", "text")?,
            content_digest: package_json_type(tx, &row, "$.contentDigest", "text")?,
            material_class: package_json_type(tx, &row, "$.materialClass", "text")?,
            material_id: package_json_type(tx, &row, "$.materialId", "text")?,
            visibility: package_json_type(tx, &row, "$.visibility", "text")?,
        });
    }
    Ok(materials)
}

fn parse_package(tx: &mut Transaction<'_, '_>, json: &str) -> Result<AuthorizedTaskPackage> {
    exact_keys(
        tx,
        json,
        None,
        &[
            "action",
            "childCeiling",
            "childCeilingDigest",
            "instruction",
            "instructionDigest",
            "materialSetDigest",
            "materials",
            "packageDigest",
            "parentCeilingDigest",
            "parentGrantDigest",
            "parentGrantRef",
            "parentGrantRevision",
            "parentGrantRevocationHead",
            "parentPolicyRevision",
            "parentSeatId",
            "route",
            "sink",
            "source",
            "sourceBinding",
            "target",
            "targetBinding",
            "targetBindingKind",
        ],
    )?;
    Ok(AuthorizedTaskPackage {
        action: package_json_type(tx, json, "$.action", "text")?,
        child_ceiling: parse_ceiling(tx, json, "$.childCeiling")?,
        child_ceiling_digest: package_json_type(tx, json, "$.childCeilingDigest", "text")?,
        instruction: package_json_type(tx, json, "$.instruction", "text")?,
        instruction_digest: package_json_type(tx, json, "$.instructionDigest", "text")?,
        material_set_digest: package_json_type(tx, json, "$.materialSetDigest", "text")?,
        materials: parse_materials(tx, json)?,
        package_digest: package_json_type(tx, json, "$.packageDigest", "text")?,
        parent_ceiling_digest: package_json_type(tx, json, "$.parentCeilingDigest", "text")?,
        parent_grant_digest: package_json_type(tx, json, "$.parentGrantDigest", "text")?,
        parent_grant_ref: package_json_type(tx, json, "$.parentGrantRef", "text")?,
        parent_grant_revision: package_json_type(tx, json, "$.parentGrantRevision", "text")?,
        parent_grant_revocation_head: package_json_type(
            tx,
            json,
            "$.parentGrantRevocationHead",
            "text",
        )?,
        parent_policy_revision: package_json_type(tx, json, "$.parentPolicyRevision", "text")?,
        parent_seat_id: package_json_type(tx, json, "$.parentSeatId", "text")?,
        route: package_json_type(tx, json, "$.route", "text")?,
        sink: package_json_type(tx, json, "$.sink", "text")?,
        source: parse_principal(tx, json, "$.source")?,
        source_binding: parse_binding(tx, json, "$.sourceBinding")?,
        target: parse_principal(tx, json, "$.target")?,
        target_binding: parse_binding(tx, json, "$.targetBinding")?,
        target_binding_kind: package_json_type(tx, json, "$.targetBindingKind", "text")?,
    })
}

fn read_in_transaction(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<Option<StoredAuthorizedTaskPackage>> {
    let rows = tx.query("SELECT i.package_id,o.canonical_json,o.content_hash,r.receipt_id,r.event_id,r.recorded_at,CAST(r.canonical_json AS TEXT),r.content_hash,r.operation_id,r.receipt_type,r.object_type,r.object_id,r.object_version,e.stream_id,e.event_type,CAST(e.canonical_json AS TEXT),e.content_hash,e.object_type,e.object_id,e.object_version,r.operation_fingerprint,e.event_id,e.stream_counter,e.occurred_at,sh.counter,i.material_refs_json FROM main.gogoke_authorized_task_packages i JOIN main.gogoke_objects o ON o.domain_id=i.domain_id AND o.object_type=i.object_type AND o.object_id=i.package_id AND o.object_version=i.object_version JOIN main.gogoke_receipts r ON r.domain_id=i.domain_id AND r.operation_id=i.operation_id AND r.object_type=i.object_type AND r.object_id=i.package_id AND r.object_version=i.object_version JOIN main.gogoke_events e ON e.domain_id=r.domain_id AND e.event_id=r.event_id JOIN main.gogoke_stream_heads sh ON sh.domain_id=e.domain_id AND sh.stream_id=e.stream_id WHERE i.domain_id=? AND i.operation_id=? AND i.object_type='AuthorizedTaskPackage'", &[domain_id,operation_id], 26)?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    let json = row[1].clone();
    let material_refs_digest = digest_canonical(&row[25]);
    let package = parse_package(tx, &json)?;
    if row[0] != package.package_digest
        || row[2] != digest_canonical(&json)
        || package_json(&package) != json.as_bytes()
    {
        return denied();
    }
    let material_refs = parse_material_refs(tx, &row[25])?;
    let current = validate_current_parent(tx, &package)?;
    let draft = draft_from_package(&package);
    let resolved_materials = resolve_material_refs(tx, &material_refs, &draft, &current)?;
    if package.materials != resolved_materials {
        return denied();
    }
    validate_package(&package, &current)?;
    let event_json = row[15].clone();
    let receipt_json = row[6].clone();
    if row[3].is_empty()
        || row[4].is_empty()
        || row[5].is_empty()
        || row[7] != digest_canonical(&receipt_json)
        || row[16] != digest_canonical(&event_json)
        || row[8] != operation_id
        || row[9] != "AuthorizedTaskPackagePrepared"
        || row[10] != "AuthorizedTaskPackage"
        || row[11] != row[0]
        || row[12] != "1"
        || row[13] != format!("gogoke.authorized-task-package.v1/{}", row[0])
        || row[14] != "AuthorizedTaskPackagePrepared"
        || row[17] != "AuthorizedTaskPackage"
        || row[18] != row[0]
        || row[19] != "1"
        || row[21] != row[4]
        || row[22] != "0"
        || row[23] != row[5]
        || row[24] != row[22]
    {
        return denied();
    }
    hash(&row[20])?;
    exact_keys(
        tx,
        &event_json,
        None,
        &[
            "materialRefsDigest",
            "operationId",
            "packageDigest",
            "parentGrantDigest",
        ],
    )?;
    if package_json_type(tx, &event_json, "$.materialRefsDigest", "text")? != material_refs_digest
        || package_json_type(tx, &event_json, "$.operationId", "text")? != operation_id
        || package_json_type(tx, &event_json, "$.packageDigest", "text")? != package.package_digest
        || package_json_type(tx, &event_json, "$.parentGrantDigest", "text")?
            != package.parent_grant_digest
    {
        return denied();
    }
    exact_keys(
        tx,
        &receipt_json,
        None,
        &[
            "authorityStatus",
            "materialRefsDigest",
            "operationId",
            "packageDigest",
            "schema",
        ],
    )?;
    if package_json_type(tx, &receipt_json, "$.authorityStatus", "text")? != AUTHORITY_STATUS
        || package_json_type(tx, &receipt_json, "$.materialRefsDigest", "text")?
            != material_refs_digest
        || package_json_type(tx, &receipt_json, "$.operationId", "text")? != operation_id
        || package_json_type(tx, &receipt_json, "$.packageDigest", "text")?
            != package.package_digest
        || package_json_type(tx, &receipt_json, "$.schema", "text")?
            != "gogoke.authorized-task-package-prepare.v1"
    {
        return denied();
    }
    let expected_event = format!(
        "{{\"materialRefsDigest\":{},\"operationId\":{},\"packageDigest\":{},\"parentGrantDigest\":{}}}",
        quote(&material_refs_digest),
        quote(operation_id),
        quote(&package.package_digest),
        quote(&package.parent_grant_digest)
    );
    let expected_receipt = format!(
        "{{\"authorityStatus\":{},\"materialRefsDigest\":{},\"operationId\":{},\"packageDigest\":{},\"schema\":\"gogoke.authorized-task-package-prepare.v1\"}}",
        quote(AUTHORITY_STATUS),
        quote(&material_refs_digest),
        quote(operation_id),
        quote(&package.package_digest)
    );
    if event_json != expected_event || receipt_json != expected_receipt {
        return denied();
    }
    let index_input = PrepareAuthorizedTaskPackage {
        operation_id: operation_id.to_owned(),
        domain_id: domain_id.to_owned(),
        event_id: row[4].clone(),
        receipt_id: row[3].clone(),
        recorded_at: row[5].clone(),
        package: draft,
        material_refs: material_refs.clone(),
    };
    let expected_index = index_values(&index_input, &package, &row[25], &row[20]);
    let index = tx.query("SELECT domain_id,operation_id,package_id,object_type,object_version,package_digest,event_id,receipt_id,recorded_at,operation_fingerprint,stream_counter,parent_grant_id,parent_grant_revision,parent_grant_revocation_head,parent_policy_revision,parent_seat_id,parent_grant_digest,parent_ceiling_digest,child_ceiling_digest,action,route,sink,source_principal_id,source_project_id,source_domain_id,source_role,target_principal_id,target_project_id,target_domain_id,target_role,source_session_id,source_execution_id,source_generation,target_session_id,target_execution_id,target_generation,target_binding_kind,instruction_digest,material_set_digest,material_refs_json FROM main.gogoke_authorized_task_packages WHERE domain_id=? AND operation_id=?", &[domain_id,operation_id], 40)?;
    if index.len() != 1 || index[0] != expected_index {
        return denied();
    }
    let storage = tx.apply_domain_record(record_input(
        domain_id,
        operation_id,
        &row[4],
        &row[3],
        &row[5],
        &package,
        &material_refs_digest,
    ))?;
    if storage.operation_fingerprint != row[20]
        || storage.event_id != row[4]
        || storage.receipt_id != row[3]
        || storage.object_hash != row[2]
        || storage.event_hash != row[16]
        || storage.receipt_hash != row[7]
    {
        return denied();
    }
    Ok(Some(StoredAuthorizedTaskPackage {
        package,
        material_refs,
        receipt_id: row[3].clone(),
        event_id: row[4].clone(),
        recorded_at: row[5].clone(),
        operation_fingerprint: row[20].clone(),
        canonical_package: json.into_bytes(),
    }))
}

pub(super) fn read_authorized_task_package_in_transaction(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<Option<AuthorizedTaskPackage>> {
    ensure_schema(tx)?;
    Ok(read_in_transaction(tx, domain_id, operation_id)?.map(|stored| stored.package))
}

pub(super) fn prepare_in_transaction(
    tx: &mut Transaction<'_, '_>,
    input: &PrepareAuthorizedTaskPackage,
) -> Result<AuthorizedTaskPackageReceipt> {
    ensure_schema(tx)?;
    let current = resolve_current_parent(
        tx,
        &input.package.parent_grant_ref,
        &input.package.parent_grant_revision,
        &input.package.parent_grant_revocation_head,
        &input.package.parent_policy_revision,
        &input.package.parent_seat_id,
    )?;
    let materials = resolve_material_refs(tx, &input.material_refs, &input.package, &current)?;
    let package = package_from_draft(&input.package, &current, materials);
    validate_package(&package, &current)?;
    let refs_json = material_refs_json(&input.material_refs);
    let refs_digest = digest_canonical(&refs_json);
    if input.domain_id != input.package.source.domain_id
        || input.domain_id != current.principal.domain_id
    {
        return denied();
    }
    if let Some(stored) = read_in_transaction(tx, &input.domain_id, &input.operation_id)? {
        if stored.package != package
            || stored.material_refs != input.material_refs
            || stored.receipt_id != input.receipt_id
            || stored.event_id != input.event_id
            || stored.recorded_at != input.recorded_at
            || stored.canonical_package.as_slice() != package_json(&package).as_slice()
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let current_intent = tx.apply_domain_record(record_input(
            &input.domain_id,
            &input.operation_id,
            &input.event_id,
            &input.receipt_id,
            &input.recorded_at,
            &package,
            &refs_digest,
        ))?;
        if current_intent.operation_fingerprint != stored.operation_fingerprint {
            return Err(OrchestrationError::OperationConflict);
        }
        return Ok(AuthorizedTaskPackageReceipt {
            disposition: "REPLAYED",
            authority_status: AUTHORITY_STATUS,
            operation_id: input.operation_id.clone(),
            package_id: stored.package.package_digest.clone(),
            package_digest: stored.package.package_digest,
            canonical_package: stored.canonical_package,
        });
    }
    if !tx
        .query(
            "SELECT 1 FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=? LIMIT 1",
            &[&input.domain_id, &input.operation_id],
            1,
        )?
        .is_empty()
    {
        return Err(OrchestrationError::OperationConflict);
    }
    let canonical = package_json(&package);
    if package.package_digest != digest_canonical(&package_preimage(&package)) {
        return denied();
    }
    let package_id = package.package_digest.clone();
    identifier(&input.operation_id)?;
    identifier(&input.event_id)?;
    identifier(&input.receipt_id)?;
    identifier(&package_id)?;
    valid_identity_text(&input.recorded_at)?;
    let storage = tx.apply_domain_record(record_input(
        &input.domain_id,
        &input.operation_id,
        &input.event_id,
        &input.receipt_id,
        &input.recorded_at,
        &package,
        &refs_digest,
    ))?;
    insert_index(
        tx,
        input,
        &package,
        &refs_json,
        &storage.operation_fingerprint,
    )?;
    ensure_schema(tx)?;
    let Some(stored) = read_in_transaction(tx, &input.domain_id, &input.operation_id)? else {
        return denied();
    };
    if stored.package != package
        || stored.material_refs != input.material_refs
        || stored.receipt_id != input.receipt_id
        || stored.event_id != input.event_id
        || stored.recorded_at != input.recorded_at
        || stored.operation_fingerprint != storage.operation_fingerprint
        || stored.canonical_package != canonical
    {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok(AuthorizedTaskPackageReceipt {
        disposition: "COMMITTED",
        authority_status: AUTHORITY_STATUS,
        operation_id: input.operation_id.clone(),
        package_id: package_id.clone(),
        package_digest: package_id,
        canonical_package: stored.canonical_package,
    })
}

fn validate_operation(input: &PrepareAuthorizedTaskPackage) -> Result<()> {
    identifier(&input.operation_id)?;
    identifier(&input.domain_id)?;
    identifier(&input.event_id)?;
    identifier(&input.receipt_id)?;
    valid_identity_text(&input.recorded_at)?;
    if input.recorded_at.len() > 128 {
        return denied();
    }
    Ok(())
}

pub(crate) fn prepare_authorized_task_package(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &PrepareAuthorizedTaskPackage,
) -> Result<AuthorizedTaskPackageReceipt> {
    validate_operation(input)?;
    transaction::run(connection, |tx| prepare_in_transaction(tx, input))
}

pub(crate) fn read_authorized_task_package(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    operation_id: &str,
) -> Result<AuthorizedTaskPackageReceipt> {
    identifier(domain_id)?;
    identifier(operation_id)?;
    transaction::run(connection, |tx| {
        ensure_schema(tx)?;
        let Some(stored) = read_in_transaction(tx, domain_id, operation_id)? else {
            return denied();
        };
        Ok(AuthorizedTaskPackageReceipt {
            disposition: "READ",
            authority_status: AUTHORITY_STATUS,
            operation_id: operation_id.to_owned(),
            package_id: stored.package.package_digest.clone(),
            package_digest: stored.package.package_digest,
            canonical_package: stored.canonical_package,
        })
    })
}

// The complete implementation below uses the native current-grant resolver in
// the same immediate transaction as object/event/receipt and typed-index writes.
// A production material authority is not wired in this slice, so receipts are
// explicitly preparatory.
