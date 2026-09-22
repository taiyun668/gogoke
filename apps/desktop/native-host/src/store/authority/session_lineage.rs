//! Durable SessionLineage and ExposureReceipt records on Product Authority.
//! This private preparatory ingress is not a process-state oracle or dispatch path.

use super::super::atomic::{DomainRecordInput, DomainRecordReceipt};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::catalog::current_profile;
use super::model::{denied, identifier, next_revision, revision};
use super::transaction::{self, Result, Transaction};
use std::collections::BTreeSet;

const AUTHORITY_STATUS: &str = "PREPARATORY_TRUSTED_INGRESS_REQUIRED";
const HEAD_SCHEMA: &str = "CREATE TABLE gogoke_session_lineage_heads (domain_id TEXT NOT NULL,session_id TEXT NOT NULL,object_type TEXT NOT NULL CHECK(object_type='SessionLineage'),revision TEXT NOT NULL,content_hash TEXT NOT NULL,native_session_id TEXT NOT NULL,binding_id TEXT NOT NULL,generation TEXT NOT NULL,source_epoch TEXT NOT NULL,lifecycle TEXT NOT NULL CHECK(lifecycle IN ('ACTIVE','ARCHIVED')),PRIMARY KEY(domain_id,session_id),UNIQUE(domain_id,native_session_id,binding_id,generation,source_epoch),FOREIGN KEY(domain_id,object_type,session_id,revision) REFERENCES gogoke_objects(domain_id,object_type,object_id,object_version) ON DELETE RESTRICT ON UPDATE RESTRICT) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeSessionIdentity {
    pub native_session_id: String,
    pub binding_id: String,
    pub generation: String,
    pub source_epoch: String,
    pub domain_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InheritedExposureSummary {
    pub evidence_levels: Vec<String>,
    pub taint_labels: Vec<String>,
    pub unknown_sources: Vec<String>,
    pub source_receipt_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingActionRef {
    pub action_id: String,
    pub operation_id: String,
    pub binding_id: String,
    pub generation: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceObservation {
    pub source_ref: String,
    pub status: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct NativeSourceCoverage {
    pub complete: bool,
    pub observations: Vec<SourceObservation>,
    pub unknown_sources: Vec<String>,
    pub inherited_from_receipt_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExposureReceipt {
    pub receipt_id: String,
    pub manifest_id: String,
    pub binding_id: String,
    pub generation: String,
    pub evidence_level: String,
    pub native_source_coverage: NativeSourceCoverage,
    pub taint_labels: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExposureAssessment {
    pub classification: String,
    pub evidence_level: String,
    pub taint_labels: Vec<String>,
    pub unknown_sources: Vec<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MaterialHandoff {
    pub material_ids: Vec<String>,
    pub source_session_id: String,
    pub native_resume_used: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionLineage {
    pub session_id: String,
    pub binding_id: String,
    pub parent_refs: Vec<String>,
    pub operation_kind: String,
    pub inherited_exposure: InheritedExposureSummary,
    pub source_epoch: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionSnapshot {
    pub domain_id: String,
    pub session_id: String,
    pub revision: String,
    pub lineage: SessionLineage,
    pub native: NativeSessionIdentity,
    pub lifecycle: String,
    pub process_state: String,
    pub exposure: Option<ExposureReceipt>,
    pub exposure_assessment: ExposureAssessment,
    pub pending_actions: Vec<PendingActionRef>,
    pub material_handoff: Option<MaterialHandoff>,
    pub pending_action_disposition: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionLineageOperation {
    NewClean {
        session_id: String,
        native: NativeSessionIdentity,
    },
    Resume {
        session_id: String,
        expected_revision: String,
        native: NativeSessionIdentity,
    },
    NativeFork {
        session_id: String,
        parent_session_id: String,
        expected_parent_revision: String,
        native: NativeSessionIdentity,
    },
    Rebuild {
        session_id: String,
        parent_session_id: String,
        expected_parent_revision: String,
        native: NativeSessionIdentity,
    },
    Handoff {
        session_id: String,
        parent_session_id: String,
        expected_parent_revision: String,
        native: NativeSessionIdentity,
        material_ids: Vec<String>,
    },
    Archive {
        session_id: String,
        expected_revision: String,
    },
    RetainPendingAction {
        session_id: String,
        expected_revision: String,
        action: PendingActionRef,
    },
    AppendExposureReceipt {
        session_id: String,
        expected_revision: String,
        exposure: ExposureReceipt,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionLineageCommand {
    pub operation_id: String,
    pub domain_id: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
    pub operation: SessionLineageOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionLineageReceipt {
    pub disposition: &'static str,
    pub authority_status: &'static str,
    pub operation_id: String,
    pub snapshot: SessionSnapshot,
    pub storage: DomainRecordReceipt,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredExposureReceipt {
    pub domain_id: String,
    pub session_id: String,
    pub session_revision: String,
    pub exposure: ExposureReceipt,
}

fn ensure_schema(tx: &mut Transaction<'_, '_>) -> Result<()> {
    tx.validate_product_core_schema()?;
    let rows = tx.query(
        "SELECT type,sql FROM main.sqlite_schema WHERE name='gogoke_session_lineage_heads'",
        &[],
        2,
    )?;
    if rows.is_empty() {
        tx.write(HEAD_SCHEMA, &[])?;
    } else if rows.len() != 1 || rows[0][0] != "table" || rows[0][1] != HEAD_SCHEMA {
        return denied();
    }
    if !tx.query(
        "SELECT 1 FROM temp.sqlite_schema WHERE (type IN ('table','view') AND lower(name) IN ('gogoke_session_lineage_heads','gogoke_objects','gogoke_events','gogoke_receipts','gogoke_stream_heads')) OR (type='trigger' AND lower(tbl_name) IN ('gogoke_session_lineage_heads','gogoke_objects','gogoke_events','gogoke_receipts','gogoke_stream_heads')) LIMIT 1",
        &[],
        1,
    )?.is_empty()
        || !tx.query(
            "SELECT 1 FROM main.sqlite_schema WHERE type='trigger' AND lower(tbl_name) IN ('gogoke_session_lineage_heads','gogoke_objects','gogoke_events','gogoke_receipts','gogoke_stream_heads') LIMIT 1",
            &[],
            1,
        )?.is_empty()
    {
        return denied();
    }
    Ok(())
}

pub(crate) fn initialize_session_lineage_schema(
    connection: &mut VerifiedDatabaseConnection<'_>,
) -> Result<()> {
    transaction::run(connection, ensure_schema)
}

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

fn strings_json(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| quote(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn native_json(native: &NativeSessionIdentity) -> String {
    format!(
        "{{\"bindingId\":{},\"domainId\":{},\"generation\":{},\"nativeSessionId\":{},\"sourceEpoch\":{}}}",
        quote(&native.binding_id), quote(&native.domain_id), quote(&native.generation),
        quote(&native.native_session_id), quote(&native.source_epoch),
    )
}

fn pending_json(values: &[PendingActionRef]) -> String {
    format!(
        "[{}]",
        values.iter().map(|value| format!(
            "{{\"actionId\":{},\"bindingId\":{},\"generation\":{},\"operationId\":{},\"state\":\"PENDING\"}}",
            quote(&value.action_id), quote(&value.binding_id), quote(&value.generation), quote(&value.operation_id),
        )).collect::<Vec<_>>().join(",")
    )
}

fn exposure_json(value: &ExposureReceipt) -> String {
    let observations = format!(
        "[{}]",
        value
            .native_source_coverage
            .observations
            .iter()
            .map(|observation| format!(
                "{{\"sourceRef\":{},\"status\":{}}}",
                quote(&observation.source_ref),
                quote(&observation.status),
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let inherited = value
        .native_source_coverage
        .inherited_from_receipt_id
        .as_ref()
        .map(|id| format!("\"inheritedFromReceiptId\":{},", quote(id)))
        .unwrap_or_default();
    format!(
        "{{\"bindingId\":{},\"evidenceLevel\":{},\"evidenceRefs\":{},\"generation\":{},\"manifestId\":{},\"nativeSourceCoverage\":{{\"complete\":{},{}\"observations\":{},\"unknownSources\":{}}},\"receiptId\":{},\"taintLabels\":{}}}",
        quote(&value.binding_id), quote(&value.evidence_level), strings_json(&value.evidence_refs),
        quote(&value.generation), quote(&value.manifest_id), value.native_source_coverage.complete,
        inherited, observations, strings_json(&value.native_source_coverage.unknown_sources),
        quote(&value.receipt_id), strings_json(&value.taint_labels),
    )
}

fn inherited_json(value: &InheritedExposureSummary) -> String {
    format!(
        "{{\"evidenceLevels\":{},\"sourceReceiptRefs\":{},\"taintLabels\":{},\"unknownSources\":{}}}",
        strings_json(&value.evidence_levels), strings_json(&value.source_receipt_refs),
        strings_json(&value.taint_labels), strings_json(&value.unknown_sources),
    )
}

fn assessment_json(value: &ExposureAssessment) -> String {
    format!(
        "{{\"classification\":{},\"evidenceLevel\":{},\"reason\":{},\"taintLabels\":{},\"unknownSources\":{}}}",
        quote(&value.classification), quote(&value.evidence_level), quote(&value.reason),
        strings_json(&value.taint_labels), strings_json(&value.unknown_sources),
    )
}

fn handoff_json(value: &Option<MaterialHandoff>) -> String {
    match value {
        None => "null".into(),
        Some(value) => format!(
            "{{\"materialIds\":{},\"nativeResumeUsed\":false,\"sourceSessionId\":{}}}",
            strings_json(&value.material_ids),
            quote(&value.source_session_id),
        ),
    }
}

fn snapshot_preimage(value: &SessionSnapshot) -> String {
    format!(
        "{{\"domainId\":{},\"exposure\":{},\"exposureAssessment\":{},\"lifecycle\":{},\"lineage\":{{\"bindingId\":{},\"inheritedExposure\":{},\"operationKind\":{},\"parentRefs\":{},\"sessionId\":{},\"sourceEpoch\":{}}},\"materialHandoff\":{},\"native\":{},\"pendingActionDisposition\":{},\"pendingActions\":{},\"processState\":{},\"revision\":{},\"sessionId\":{}}}",
        quote(&value.domain_id), value.exposure.as_ref().map(exposure_json).unwrap_or_else(|| "null".into()),
        assessment_json(&value.exposure_assessment), quote(&value.lifecycle),
        quote(&value.lineage.binding_id), inherited_json(&value.lineage.inherited_exposure),
        quote(&value.lineage.operation_kind), strings_json(&value.lineage.parent_refs),
        quote(&value.lineage.session_id), quote(&value.lineage.source_epoch),
        handoff_json(&value.material_handoff), native_json(&value.native),
        quote(&value.pending_action_disposition), pending_json(&value.pending_actions),
        quote(&value.process_state), quote(&value.revision), quote(&value.session_id),
    )
}

fn snapshot_json(value: &SessionSnapshot) -> Vec<u8> {
    let preimage = snapshot_preimage(value);
    let hash = content_hash(preimage.as_bytes());
    format!(
        "{{\"contentHash\":{},\"domainId\":{},\"exposure\":{},\"exposureAssessment\":{},\"lifecycle\":{},\"lineage\":{{\"bindingId\":{},\"inheritedExposure\":{},\"operationKind\":{},\"parentRefs\":{},\"sessionId\":{},\"sourceEpoch\":{}}},\"materialHandoff\":{},\"native\":{},\"pendingActionDisposition\":{},\"pendingActions\":{},\"processState\":{},\"revision\":{},\"sessionId\":{}}}",
        quote(&hash), quote(&value.domain_id), value.exposure.as_ref().map(exposure_json).unwrap_or_else(|| "null".into()),
        assessment_json(&value.exposure_assessment), quote(&value.lifecycle),
        quote(&value.lineage.binding_id), inherited_json(&value.lineage.inherited_exposure),
        quote(&value.lineage.operation_kind), strings_json(&value.lineage.parent_refs),
        quote(&value.lineage.session_id), quote(&value.lineage.source_epoch),
        handoff_json(&value.material_handoff), native_json(&value.native),
        quote(&value.pending_action_disposition), pending_json(&value.pending_actions),
        quote(&value.process_state), quote(&value.revision), quote(&value.session_id),
    ).into_bytes()
}

fn canonical_u64(value: &str) -> Result<()> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| OrchestrationError::AccessDenied)?;
    if value != parsed.to_string() {
        return denied();
    }
    Ok(())
}

fn unique_ids(values: &[String]) -> Result<Vec<String>> {
    let mut seen = BTreeSet::new();
    for value in values {
        identifier(value)?;
        if !seen.insert(value.as_str()) {
            return denied();
        }
    }
    Ok(values.to_vec())
}

fn validate_native(value: &NativeSessionIdentity, expected_domain: &str) -> Result<()> {
    for identity in [
        &value.native_session_id,
        &value.binding_id,
        &value.domain_id,
    ] {
        identifier(identity)?;
    }
    canonical_u64(&value.generation)?;
    canonical_u64(&value.source_epoch)?;
    if value.domain_id != expected_domain {
        return denied();
    }
    Ok(())
}

fn validate_action(value: &PendingActionRef, native: &NativeSessionIdentity) -> Result<()> {
    identifier(&value.action_id)?;
    identifier(&value.operation_id)?;
    identifier(&value.binding_id)?;
    canonical_u64(&value.generation)?;
    if value.binding_id != native.binding_id || value.generation != native.generation {
        return denied();
    }
    Ok(())
}

fn validate_exposure(value: &ExposureReceipt, native: &NativeSessionIdentity) -> Result<()> {
    for identity in [&value.receipt_id, &value.manifest_id, &value.binding_id] {
        identifier(identity)?;
    }
    canonical_u64(&value.generation)?;
    if value.binding_id != native.binding_id
        || value.generation != native.generation
        || !matches!(
            value.evidence_level.as_str(),
            "HOST_PREPARED"
                | "HOST_DELIVERED"
                | "NATIVE_ACKED"
                | "INHERITED"
                | "POSSIBLE"
                | "UNKNOWN"
        )
    {
        return denied();
    }
    unique_ids(&value.taint_labels)?;
    unique_ids(&value.evidence_refs)?;
    unique_ids(&value.native_source_coverage.unknown_sources)?;
    if let Some(receipt_id) = &value.native_source_coverage.inherited_from_receipt_id {
        identifier(receipt_id)?;
    }
    let mut sources = BTreeSet::new();
    for observation in &value.native_source_coverage.observations {
        identifier(&observation.source_ref)?;
        if !sources.insert(observation.source_ref.as_str())
            || !matches!(
                observation.status.as_str(),
                "COMPLETE" | "PARTIAL" | "NOT_OBSERVED" | "UNKNOWN"
            )
        {
            return denied();
        }
    }
    let complete = value
        .native_source_coverage
        .inherited_from_receipt_id
        .is_none()
        && !value.native_source_coverage.observations.is_empty()
        && value
            .native_source_coverage
            .observations
            .iter()
            .all(|item| item.status == "COMPLETE")
        && value.native_source_coverage.unknown_sources.is_empty();
    if value.native_source_coverage.complete != complete {
        return denied();
    }
    if value.evidence_level == "INHERITED"
        && value
            .native_source_coverage
            .inherited_from_receipt_id
            .is_none()
    {
        return denied();
    }
    Ok(())
}

fn dedupe(values: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

fn exposure_assessment(
    exposure: Option<&ExposureReceipt>,
    inherited: &InheritedExposureSummary,
) -> ExposureAssessment {
    let (base_class, evidence_level, mut taint, mut unknown, base_reason) = match exposure {
        None => ("UNKNOWN", "NONE", Vec::new(), Vec::new(), "NO_RECEIPT"),
        Some(receipt) => {
            let mut unknown = receipt.native_source_coverage.unknown_sources.clone();
            unknown.extend(
                receipt
                    .native_source_coverage
                    .observations
                    .iter()
                    .filter(|observation| {
                        observation.status == "UNKNOWN" || observation.status == "NOT_OBSERVED"
                    })
                    .map(|observation| observation.source_ref.clone()),
            );
            let class_and_reason = if !receipt.taint_labels.is_empty() {
                (
                    "TAINTED",
                    if receipt.evidence_level == "INHERITED" {
                        "INHERITED_TAINT"
                    } else {
                        "EXPLICIT_TAINT"
                    },
                )
            } else if receipt.evidence_level == "HOST_PREPARED"
                || receipt.evidence_level == "HOST_DELIVERED"
            {
                ("UNKNOWN", "HOST_DELIVERY_NOT_OBSERVATION")
            } else if receipt.evidence_level != "NATIVE_ACKED"
                || !receipt.native_source_coverage.complete
            {
                ("UNKNOWN", "NATIVE_COVERAGE_INCOMPLETE")
            } else if !unknown.is_empty() {
                ("UNKNOWN", "NATIVE_COVERAGE_UNKNOWN")
            } else {
                ("CLEAN", "NATIVE_COVERAGE_COMPLETE")
            };
            (
                class_and_reason.0,
                receipt.evidence_level.as_str(),
                receipt.taint_labels.clone(),
                unknown,
                class_and_reason.1,
            )
        }
    };
    taint.extend(inherited.taint_labels.clone());
    unknown.extend(inherited.unknown_sources.clone());
    let taint = dedupe(taint);
    let unknown = dedupe(unknown);
    let (classification, reason) = if !taint.is_empty() {
        (
            "TAINTED",
            if exposure
                .map(|value| !value.taint_labels.is_empty())
                .unwrap_or(false)
                && inherited.taint_labels.is_empty()
            {
                base_reason
            } else {
                "INHERITED_TAINT"
            },
        )
    } else if base_class == "UNKNOWN"
        || inherited.unknown_sources.len() > 0
        || inherited
            .evidence_levels
            .iter()
            .any(|level| level != "NATIVE_ACKED")
    {
        (
            "UNKNOWN",
            if base_class == "UNKNOWN" {
                base_reason
            } else if !unknown.is_empty() {
                "NATIVE_COVERAGE_UNKNOWN"
            } else {
                "NATIVE_COVERAGE_INCOMPLETE"
            },
        )
    } else {
        (base_class, base_reason)
    };
    ExposureAssessment {
        classification: classification.into(),
        evidence_level: evidence_level.into(),
        taint_labels: taint,
        unknown_sources: unknown,
        reason: reason.into(),
    }
}

fn inherited_summary(
    parent: &InheritedExposureSummary,
    exposure: Option<&ExposureReceipt>,
) -> InheritedExposureSummary {
    let assessment = exposure_assessment(
        exposure,
        &InheritedExposureSummary {
            evidence_levels: Vec::new(),
            taint_labels: Vec::new(),
            unknown_sources: Vec::new(),
            source_receipt_refs: Vec::new(),
        },
    );
    let mut levels = parent.evidence_levels.clone();
    if let Some(receipt) = exposure {
        levels.push(receipt.evidence_level.clone());
    }
    if assessment.classification == "UNKNOWN" {
        levels.push("UNKNOWN".into());
    }
    let mut labels = parent.taint_labels.clone();
    let mut unknown = parent.unknown_sources.clone();
    let mut refs = parent.source_receipt_refs.clone();
    if let Some(receipt) = exposure {
        labels.extend(receipt.taint_labels.clone());
        unknown.extend(assessment.unknown_sources.clone());
        if !receipt.native_source_coverage.complete {
            unknown.extend(
                receipt
                    .native_source_coverage
                    .observations
                    .iter()
                    .filter(|item| item.status != "COMPLETE")
                    .map(|item| item.source_ref.clone()),
            );
        }
        refs.push(receipt.receipt_id.clone());
        refs.extend(receipt.evidence_refs.clone());
    }
    InheritedExposureSummary {
        evidence_levels: dedupe(levels),
        taint_labels: dedupe(labels),
        unknown_sources: dedupe(unknown),
        source_receipt_refs: dedupe(refs),
    }
}

fn request_operation_json(operation: &SessionLineageOperation) -> String {
    match operation {
        SessionLineageOperation::NewClean { session_id, native } => format!(
            "{{\"native\":{},\"operation\":\"NEW_CLEAN\",\"sessionId\":{}}}", native_json(native), quote(session_id)),
        SessionLineageOperation::Resume { session_id, expected_revision, native } => format!(
            "{{\"expectedRevision\":{},\"native\":{},\"operation\":\"RESUME\",\"sessionId\":{}}}", quote(expected_revision), native_json(native), quote(session_id)),
        SessionLineageOperation::NativeFork { session_id, parent_session_id, expected_parent_revision, native } => format!(
            "{{\"expectedParentRevision\":{},\"native\":{},\"operation\":\"NATIVE_FORK\",\"parentSessionId\":{},\"sessionId\":{}}}", quote(expected_parent_revision), native_json(native), quote(parent_session_id), quote(session_id)),
        SessionLineageOperation::Rebuild { session_id, parent_session_id, expected_parent_revision, native } => format!(
            "{{\"expectedParentRevision\":{},\"native\":{},\"operation\":\"REBUILD\",\"parentSessionId\":{},\"sessionId\":{}}}", quote(expected_parent_revision), native_json(native), quote(parent_session_id), quote(session_id)),
        SessionLineageOperation::Handoff { session_id, parent_session_id, expected_parent_revision, native, material_ids } => format!(
            "{{\"expectedParentRevision\":{},\"materialIds\":{},\"native\":{},\"operation\":\"HANDOFF\",\"parentSessionId\":{},\"sessionId\":{}}}", quote(expected_parent_revision), strings_json(material_ids), native_json(native), quote(parent_session_id), quote(session_id)),
        SessionLineageOperation::Archive { session_id, expected_revision } => format!(
            "{{\"expectedRevision\":{},\"operation\":\"ARCHIVE\",\"sessionId\":{}}}", quote(expected_revision), quote(session_id)),
        SessionLineageOperation::RetainPendingAction { session_id, expected_revision, action } => format!(
            "{{\"action\":{},\"expectedRevision\":{},\"operation\":\"RETAIN_PENDING_ACTION\",\"sessionId\":{}}}", pending_json(std::slice::from_ref(action)).trim_matches(&['[', ']'][..]), quote(expected_revision), quote(session_id)),
        SessionLineageOperation::AppendExposureReceipt { session_id, expected_revision, exposure } => format!(
            "{{\"expectedRevision\":{},\"exposure\":{},\"operation\":\"APPEND_EXPOSURE_RECEIPT\",\"sessionId\":{}}}", quote(expected_revision), exposure_json(exposure), quote(session_id)),
    }
}

fn command_digest(command: &SessionLineageCommand) -> String {
    let body = format!(
        "{{\"domainId\":{},\"eventId\":{},\"operation\":{},\"operationId\":{},\"receiptId\":{},\"recordedAt\":{}}}",
        quote(&command.domain_id), quote(&command.event_id), request_operation_json(&command.operation),
        quote(&command.operation_id), quote(&command.receipt_id), quote(&command.recorded_at),
    );
    content_hash(body.as_bytes())
}

fn exact_keys(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: Option<&str>,
    keys: &[&str],
) -> Result<()> {
    let rows = match path {
        Some(path) => tx.query("SELECT key FROM json_each(?,?)", &[json, path], 1)?,
        None => tx.query("SELECT key FROM json_each(?)", &[json], 1)?,
    };
    if rows.len() != keys.len() {
        return denied();
    }
    let actual = rows
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<BTreeSet<_>>();
    if keys.iter().any(|key| !actual.contains(*key)) {
        return denied();
    }
    Ok(())
}

fn exact_keys_optional(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: Option<&str>,
    required: &[&str],
    optional: &[&str],
) -> Result<()> {
    let rows = match path {
        Some(path) => tx.query("SELECT key FROM json_each(?,?)", &[json, path], 1)?,
        None => tx.query("SELECT key FROM json_each(?)", &[json], 1)?,
    };
    if rows.len() < required.len() || rows.len() > required.len() + optional.len() {
        return denied();
    }
    let actual = rows
        .into_iter()
        .map(|row| row[0].clone())
        .collect::<BTreeSet<_>>();
    if required.iter().any(|key| !actual.contains(*key))
        || actual
            .iter()
            .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return denied();
    }
    Ok(())
}

fn json_type(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<String> {
    let rows = tx.query("SELECT json_type(?,?)", &[json, path], 1)?;
    if rows.len() != 1 {
        return denied();
    }
    Ok(rows[0][0].clone())
}

fn json_text(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<String> {
    let rows = tx.query(
        "SELECT json_type(?,?),json_extract(?,?)",
        &[json, path, json, path],
        2,
    )?;
    if rows.len() != 1 || rows[0][0] != "text" {
        return denied();
    }
    Ok(rows[0][1].clone())
}

fn json_bool(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<bool> {
    match json_type(tx, json, path)?.as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => denied(),
    }
}

fn array_rows(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<Vec<(String, String)>> {
    if json_type(tx, json, path)? != "array" {
        return denied();
    }
    let rows = tx.query(
        "SELECT type,CAST(value AS TEXT) FROM json_each(?,?) ORDER BY CAST(key AS INTEGER)",
        &[json, path],
        2,
    )?;
    Ok(rows
        .into_iter()
        .map(|row| (row[0].clone(), row[1].clone()))
        .collect())
}

fn parse_string_array(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<Vec<String>> {
    let rows = array_rows(tx, json, path)?;
    if rows.iter().any(|(kind, _)| kind != "text") {
        return denied();
    }
    let values = rows.into_iter().map(|(_, value)| value).collect::<Vec<_>>();
    unique_ids(&values)
}

fn parse_native(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<NativeSessionIdentity> {
    exact_keys(
        tx,
        json,
        Some(path),
        &[
            "bindingId",
            "domainId",
            "generation",
            "nativeSessionId",
            "sourceEpoch",
        ],
    )?;
    Ok(NativeSessionIdentity {
        binding_id: json_text(tx, json, &format!("{path}.bindingId"))?,
        domain_id: json_text(tx, json, &format!("{path}.domainId"))?,
        generation: json_text(tx, json, &format!("{path}.generation"))?,
        native_session_id: json_text(tx, json, &format!("{path}.nativeSessionId"))?,
        source_epoch: json_text(tx, json, &format!("{path}.sourceEpoch"))?,
    })
}

fn parse_inherited(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<InheritedExposureSummary> {
    exact_keys(
        tx,
        json,
        Some(path),
        &[
            "evidenceLevels",
            "sourceReceiptRefs",
            "taintLabels",
            "unknownSources",
        ],
    )?;
    let value = InheritedExposureSummary {
        evidence_levels: parse_string_array(tx, json, &format!("{path}.evidenceLevels"))?,
        source_receipt_refs: parse_string_array(tx, json, &format!("{path}.sourceReceiptRefs"))?,
        taint_labels: parse_string_array(tx, json, &format!("{path}.taintLabels"))?,
        unknown_sources: parse_string_array(tx, json, &format!("{path}.unknownSources"))?,
    };
    if value.evidence_levels.iter().any(|level| {
        !matches!(
            level.as_str(),
            "HOST_PREPARED"
                | "HOST_DELIVERED"
                | "NATIVE_ACKED"
                | "INHERITED"
                | "POSSIBLE"
                | "UNKNOWN"
        )
    }) {
        return denied();
    }
    Ok(value)
}

fn parse_exposure(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<Option<ExposureReceipt>> {
    match json_type(tx, json, path)?.as_str() {
        "null" => Ok(None),
        "object" => {
            exact_keys(
                tx,
                json,
                Some(path),
                &[
                    "bindingId",
                    "evidenceLevel",
                    "evidenceRefs",
                    "generation",
                    "manifestId",
                    "nativeSourceCoverage",
                    "receiptId",
                    "taintLabels",
                ],
            )?;
            let coverage_path = format!("{path}.nativeSourceCoverage");
            exact_keys_optional(
                tx,
                json,
                Some(&coverage_path),
                &["complete", "observations", "unknownSources"],
                &["inheritedFromReceiptId"],
            )?;
            let mut observations = Vec::new();
            for (kind, item) in array_rows(tx, json, &format!("{coverage_path}.observations"))? {
                if kind != "object" {
                    return denied();
                }
                exact_keys(tx, &item, None, &["sourceRef", "status"])?;
                observations.push(SourceObservation {
                    source_ref: json_text(tx, &item, "$.sourceRef")?,
                    status: json_text(tx, &item, "$.status")?,
                });
            }
            let inherited_path = format!("{coverage_path}.inheritedFromReceiptId");
            let inherited_rows = tx.query(
                "SELECT 1 FROM json_each(?,?) WHERE key='inheritedFromReceiptId'",
                &[json, &coverage_path],
                1,
            )?;
            let inherited_from_receipt_id = if inherited_rows.is_empty() {
                None
            } else {
                Some(json_text(tx, json, &inherited_path)?)
            };
            Ok(Some(ExposureReceipt {
                binding_id: json_text(tx, json, &format!("{path}.bindingId"))?,
                evidence_level: json_text(tx, json, &format!("{path}.evidenceLevel"))?,
                evidence_refs: parse_string_array(tx, json, &format!("{path}.evidenceRefs"))?,
                generation: json_text(tx, json, &format!("{path}.generation"))?,
                manifest_id: json_text(tx, json, &format!("{path}.manifestId"))?,
                native_source_coverage: NativeSourceCoverage {
                    complete: json_bool(tx, json, &format!("{coverage_path}.complete"))?,
                    observations,
                    unknown_sources: parse_string_array(
                        tx,
                        json,
                        &format!("{coverage_path}.unknownSources"),
                    )?,
                    inherited_from_receipt_id,
                },
                receipt_id: json_text(tx, json, &format!("{path}.receiptId"))?,
                taint_labels: parse_string_array(tx, json, &format!("{path}.taintLabels"))?,
            }))
        }
        _ => denied(),
    }
}

fn parse_pending(
    tx: &mut Transaction<'_, '_>,
    json: &str,
    path: &str,
) -> Result<Vec<PendingActionRef>> {
    let mut actions = Vec::new();
    for (kind, item) in array_rows(tx, json, path)? {
        if kind != "object" {
            return denied();
        }
        exact_keys(
            tx,
            &item,
            None,
            &[
                "actionId",
                "bindingId",
                "generation",
                "operationId",
                "state",
            ],
        )?;
        if json_text(tx, &item, "$.state")? != "PENDING" {
            return denied();
        }
        actions.push(PendingActionRef {
            action_id: json_text(tx, &item, "$.actionId")?,
            binding_id: json_text(tx, &item, "$.bindingId")?,
            generation: json_text(tx, &item, "$.generation")?,
            operation_id: json_text(tx, &item, "$.operationId")?,
        });
    }
    Ok(actions)
}

fn parse_assessment(tx: &mut Transaction<'_, '_>, json: &str) -> Result<ExposureAssessment> {
    exact_keys(
        tx,
        json,
        Some("$.exposureAssessment"),
        &[
            "classification",
            "evidenceLevel",
            "reason",
            "taintLabels",
            "unknownSources",
        ],
    )?;
    Ok(ExposureAssessment {
        classification: json_text(tx, json, "$.exposureAssessment.classification")?,
        evidence_level: json_text(tx, json, "$.exposureAssessment.evidenceLevel")?,
        reason: json_text(tx, json, "$.exposureAssessment.reason")?,
        taint_labels: parse_string_array(tx, json, "$.exposureAssessment.taintLabels")?,
        unknown_sources: parse_string_array(tx, json, "$.exposureAssessment.unknownSources")?,
    })
}

fn parse_handoff(tx: &mut Transaction<'_, '_>, json: &str) -> Result<Option<MaterialHandoff>> {
    match json_type(tx, json, "$.materialHandoff")?.as_str() {
        "null" => Ok(None),
        "object" => {
            exact_keys(
                tx,
                json,
                Some("$.materialHandoff"),
                &["materialIds", "nativeResumeUsed", "sourceSessionId"],
            )?;
            if json_bool(tx, json, "$.materialHandoff.nativeResumeUsed")? {
                return denied();
            }
            Ok(Some(MaterialHandoff {
                material_ids: parse_string_array(tx, json, "$.materialHandoff.materialIds")?,
                source_session_id: json_text(tx, json, "$.materialHandoff.sourceSessionId")?,
                native_resume_used: false,
            }))
        }
        _ => denied(),
    }
}

fn parse_snapshot(tx: &mut Transaction<'_, '_>, json: &str) -> Result<SessionSnapshot> {
    let valid = tx.query("SELECT json_valid(?)", &[json], 1)?;
    if valid.len() != 1 || valid[0][0] != "1" {
        return denied();
    }
    exact_keys(
        tx,
        json,
        None,
        &[
            "contentHash",
            "domainId",
            "exposure",
            "exposureAssessment",
            "lifecycle",
            "lineage",
            "materialHandoff",
            "native",
            "pendingActionDisposition",
            "pendingActions",
            "processState",
            "revision",
            "sessionId",
        ],
    )?;
    exact_keys(
        tx,
        json,
        Some("$.lineage"),
        &[
            "bindingId",
            "inheritedExposure",
            "operationKind",
            "parentRefs",
            "sessionId",
            "sourceEpoch",
        ],
    )?;
    Ok(SessionSnapshot {
        content_hash: json_text(tx, json, "$.contentHash")?,
        domain_id: json_text(tx, json, "$.domainId")?,
        exposure: parse_exposure(tx, json, "$.exposure")?,
        exposure_assessment: parse_assessment(tx, json)?,
        lifecycle: json_text(tx, json, "$.lifecycle")?,
        lineage: SessionLineage {
            binding_id: json_text(tx, json, "$.lineage.bindingId")?,
            inherited_exposure: parse_inherited(tx, json, "$.lineage.inheritedExposure")?,
            operation_kind: json_text(tx, json, "$.lineage.operationKind")?,
            parent_refs: parse_string_array(tx, json, "$.lineage.parentRefs")?,
            session_id: json_text(tx, json, "$.lineage.sessionId")?,
            source_epoch: json_text(tx, json, "$.lineage.sourceEpoch")?,
        },
        material_handoff: parse_handoff(tx, json)?,
        native: parse_native(tx, json, "$.native")?,
        pending_action_disposition: json_text(tx, json, "$.pendingActionDisposition")?,
        pending_actions: parse_pending(tx, json, "$.pendingActions")?,
        process_state: json_text(tx, json, "$.processState")?,
        revision: json_text(tx, json, "$.revision")?,
        session_id: json_text(tx, json, "$.sessionId")?,
    })
}

fn operation_name(operation: &SessionLineageOperation) -> &'static str {
    match operation {
        SessionLineageOperation::NewClean { .. } => "NEW_CLEAN",
        SessionLineageOperation::Resume { .. } => "RESUME",
        SessionLineageOperation::NativeFork { .. } => "NATIVE_FORK",
        SessionLineageOperation::Rebuild { .. } => "REBUILD",
        SessionLineageOperation::Handoff { .. } => "HANDOFF",
        SessionLineageOperation::Archive { .. } => "ARCHIVE",
        SessionLineageOperation::RetainPendingAction { .. } => "RETAIN_PENDING_ACTION",
        SessionLineageOperation::AppendExposureReceipt { .. } => "APPEND_EXPOSURE_RECEIPT",
    }
}

fn operation_session_id(operation: &SessionLineageOperation) -> &str {
    match operation {
        SessionLineageOperation::NewClean { session_id, .. }
        | SessionLineageOperation::Resume { session_id, .. }
        | SessionLineageOperation::NativeFork { session_id, .. }
        | SessionLineageOperation::Rebuild { session_id, .. }
        | SessionLineageOperation::Handoff { session_id, .. }
        | SessionLineageOperation::Archive { session_id, .. }
        | SessionLineageOperation::RetainPendingAction { session_id, .. }
        | SessionLineageOperation::AppendExposureReceipt { session_id, .. } => session_id,
    }
}

fn validate_command(command: &SessionLineageCommand) -> Result<()> {
    for value in [
        &command.operation_id,
        &command.domain_id,
        &command.event_id,
        &command.receipt_id,
    ] {
        identifier(value)?;
    }
    identifier(operation_session_id(&command.operation))?;
    identifier(&command.recorded_at)?;
    if command.recorded_at.len() > 128 {
        return denied();
    }
    match &command.operation {
        SessionLineageOperation::NewClean { native, .. }
        | SessionLineageOperation::Resume { native, .. }
        | SessionLineageOperation::NativeFork { native, .. }
        | SessionLineageOperation::Rebuild { native, .. }
        | SessionLineageOperation::Handoff { native, .. } => {
            validate_native(native, &command.domain_id)?
        }
        _ => {}
    }
    match &command.operation {
        SessionLineageOperation::NewClean { .. } => {}
        SessionLineageOperation::Resume {
            expected_revision, ..
        }
        | SessionLineageOperation::Archive {
            expected_revision, ..
        }
        | SessionLineageOperation::RetainPendingAction {
            expected_revision, ..
        }
        | SessionLineageOperation::AppendExposureReceipt {
            expected_revision, ..
        } => {
            if revision(expected_revision)?.to_string() != *expected_revision {
                return denied();
            }
        }
        SessionLineageOperation::NativeFork {
            parent_session_id,
            expected_parent_revision,
            ..
        }
        | SessionLineageOperation::Rebuild {
            parent_session_id,
            expected_parent_revision,
            ..
        }
        | SessionLineageOperation::Handoff {
            parent_session_id,
            expected_parent_revision,
            ..
        } => {
            identifier(parent_session_id)?;
            if revision(expected_parent_revision)?.to_string() != *expected_parent_revision
                || parent_session_id == operation_session_id(&command.operation)
            {
                return denied();
            }
        }
    }
    if let SessionLineageOperation::Handoff { material_ids, .. } = &command.operation {
        if material_ids.is_empty() {
            return denied();
        }
        unique_ids(material_ids)?;
    }
    if let SessionLineageOperation::RetainPendingAction { action, .. } = &command.operation {
        for value in [&action.action_id, &action.operation_id, &action.binding_id] {
            identifier(value)?;
        }
        canonical_u64(&action.generation)?;
    }
    if let SessionLineageOperation::AppendExposureReceipt { exposure, .. } = &command.operation {
        for value in [
            &exposure.receipt_id,
            &exposure.manifest_id,
            &exposure.binding_id,
        ] {
            identifier(value)?;
        }
        canonical_u64(&exposure.generation)?;
        if !matches!(
            exposure.evidence_level.as_str(),
            "HOST_PREPARED"
                | "HOST_DELIVERED"
                | "NATIVE_ACKED"
                | "INHERITED"
                | "POSSIBLE"
                | "UNKNOWN"
        ) {
            return denied();
        }
        unique_ids(&exposure.taint_labels)?;
        unique_ids(&exposure.evidence_refs)?;
        unique_ids(&exposure.native_source_coverage.unknown_sources)?;
        let mut refs = BTreeSet::new();
        for item in &exposure.native_source_coverage.observations {
            identifier(&item.source_ref)?;
            if !refs.insert(item.source_ref.as_str())
                || !matches!(
                    item.status.as_str(),
                    "COMPLETE" | "PARTIAL" | "NOT_OBSERVED" | "UNKNOWN"
                )
            {
                return denied();
            }
        }
        let derived = exposure
            .native_source_coverage
            .inherited_from_receipt_id
            .is_none()
            && !exposure.native_source_coverage.observations.is_empty()
            && exposure
                .native_source_coverage
                .observations
                .iter()
                .all(|item| item.status == "COMPLETE")
            && exposure.native_source_coverage.unknown_sources.is_empty();
        if exposure.native_source_coverage.complete != derived {
            return denied();
        }
        if let Some(inherited) = &exposure.native_source_coverage.inherited_from_receipt_id {
            identifier(inherited)?;
        }
        if exposure.evidence_level == "INHERITED"
            && exposure
                .native_source_coverage
                .inherited_from_receipt_id
                .is_none()
        {
            return denied();
        }
    }
    Ok(())
}

fn operation_event(
    command: &SessionLineageCommand,
    snapshot: &SessionSnapshot,
    digest: &str,
) -> Vec<u8> {
    format!(
        "{{\"contentHash\":{},\"operation\":{},\"operationId\":{},\"requestDigest\":{},\"sessionId\":{},\"sessionRevision\":{},\"type\":\"SessionLineageCommitted\"}}",
        quote(&snapshot.content_hash), quote(operation_name(&command.operation)), quote(&command.operation_id),
        quote(digest), quote(&snapshot.session_id), quote(&snapshot.revision),
    ).into_bytes()
}

fn operation_receipt(
    command: &SessionLineageCommand,
    snapshot: &SessionSnapshot,
    digest: &str,
) -> Vec<u8> {
    format!(
        "{{\"authorityStatus\":{},\"contentHash\":{},\"domainId\":{},\"operationId\":{},\"requestDigest\":{},\"revision\":{},\"sessionId\":{},\"type\":\"SessionLineageCommitted\"}}",
        quote(AUTHORITY_STATUS), quote(&snapshot.content_hash), quote(&command.domain_id),
        quote(&command.operation_id), quote(digest), quote(&snapshot.revision), quote(&snapshot.session_id),
    ).into_bytes()
}

fn record_input(
    command: &SessionLineageCommand,
    snapshot: &SessionSnapshot,
    digest: &str,
) -> Result<DomainRecordInput> {
    let version = revision(&snapshot.revision)?;
    if version == 0 || snapshot.revision != version.to_string() {
        return denied();
    }
    Ok(DomainRecordInput {
        domain_id: command.domain_id.clone(),
        object_type: "SessionLineage".into(),
        object_id: snapshot.session_id.clone(),
        object_version: snapshot.revision.clone(),
        object_bytes: snapshot_json(snapshot),
        native_identity: None,
        event_id: command.event_id.clone(),
        stream_id: format!("gogoke.session-lineage.v1/{}", snapshot.session_id),
        expected_previous_counter: version.checked_sub(2).map(|value| value.to_string()),
        counter: version.saturating_sub(1).to_string(),
        event_type: "SessionLineageCommitted".into(),
        occurred_at: command.recorded_at.clone(),
        event_bytes: operation_event(command, snapshot, digest),
        receipt_id: command.receipt_id.clone(),
        operation_id: command.operation_id.clone(),
        receipt_type: "SessionLineageCommitted".into(),
        recorded_at: command.recorded_at.clone(),
        receipt_bytes: operation_receipt(command, snapshot, digest),
    })
}

fn event_json(
    operation: &str,
    operation_id: &str,
    request_digest: &str,
    snapshot: &SessionSnapshot,
) -> String {
    format!(
        "{{\"contentHash\":{},\"operation\":{},\"operationId\":{},\"requestDigest\":{},\"sessionId\":{},\"sessionRevision\":{},\"type\":\"SessionLineageCommitted\"}}",
        quote(&snapshot.content_hash), quote(operation), quote(operation_id), quote(request_digest),
        quote(&snapshot.session_id), quote(&snapshot.revision),
    )
}

fn receipt_json(
    domain_id: &str,
    operation_id: &str,
    request_digest: &str,
    snapshot: &SessionSnapshot,
) -> String {
    format!(
        "{{\"authorityStatus\":{},\"contentHash\":{},\"domainId\":{},\"operationId\":{},\"requestDigest\":{},\"revision\":{},\"sessionId\":{},\"type\":\"SessionLineageCommitted\"}}",
        quote(AUTHORITY_STATUS), quote(&snapshot.content_hash), quote(domain_id), quote(operation_id),
        quote(request_digest), quote(&snapshot.revision), quote(&snapshot.session_id),
    )
}

fn validate_snapshot(snapshot: &SessionSnapshot) -> Result<()> {
    identifier(&snapshot.domain_id)?;
    identifier(&snapshot.session_id)?;
    if snapshot.revision != revision(&snapshot.revision)?.to_string()
        || !matches!(snapshot.lifecycle.as_str(), "ACTIVE" | "ARCHIVED")
        || !matches!(
            snapshot.process_state.as_str(),
            "RUNNING" | "IDLE" | "UNKNOWN"
        )
        || !matches!(
            snapshot.pending_action_disposition.as_str(),
            "NONE" | "RETAINED_ON_PARENT" | "RETAINED_NOT_REPLAYED"
        )
        || !matches!(
            snapshot.lineage.operation_kind.as_str(),
            "NEW_CLEAN" | "RESUME" | "NATIVE_FORK" | "REBUILD" | "HANDOFF" | "ARCHIVE"
        )
    {
        return denied();
    }
    validate_native(&snapshot.native, &snapshot.domain_id)?;
    if snapshot.lineage.session_id != snapshot.session_id
        || snapshot.lineage.binding_id != snapshot.native.binding_id
        || snapshot.lineage.source_epoch != snapshot.native.source_epoch
    {
        return denied();
    }
    unique_ids(&snapshot.lineage.parent_refs)?;
    unique_ids(&snapshot.lineage.inherited_exposure.evidence_levels)?;
    unique_ids(&snapshot.lineage.inherited_exposure.taint_labels)?;
    unique_ids(&snapshot.lineage.inherited_exposure.unknown_sources)?;
    unique_ids(&snapshot.lineage.inherited_exposure.source_receipt_refs)?;
    if snapshot
        .lineage
        .inherited_exposure
        .evidence_levels
        .iter()
        .any(|level| {
            !matches!(
                level.as_str(),
                "HOST_PREPARED"
                    | "HOST_DELIVERED"
                    | "NATIVE_ACKED"
                    | "INHERITED"
                    | "POSSIBLE"
                    | "UNKNOWN"
            )
        })
    {
        return denied();
    }
    let mut action_ids = BTreeSet::new();
    for action in &snapshot.pending_actions {
        validate_action(action, &snapshot.native)?;
        if !action_ids.insert(action.action_id.as_str()) {
            return denied();
        }
    }
    if let Some(exposure) = &snapshot.exposure {
        validate_exposure(exposure, &snapshot.native)?;
    }
    if let Some(handoff) = &snapshot.material_handoff {
        identifier(&handoff.source_session_id)?;
        if handoff.native_resume_used || handoff.material_ids.is_empty() {
            return denied();
        }
        unique_ids(&handoff.material_ids)?;
    }
    let expected_assessment = exposure_assessment(
        snapshot.exposure.as_ref(),
        &snapshot.lineage.inherited_exposure,
    );
    if snapshot.exposure_assessment != expected_assessment {
        return denied();
    }
    if snapshot.content_hash != content_hash(snapshot_preimage(snapshot).as_bytes()) {
        return denied();
    }
    Ok(())
}

fn decode_object_snapshot(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    session_id: &str,
    object_version: &str,
) -> Result<SessionSnapshot> {
    let rows = tx.query(
        "SELECT CAST(canonical_json AS TEXT),content_hash FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND object_id=? AND object_version=?",
        &[domain_id, session_id, object_version],
        2,
    )?;
    if rows.len() != 1 || content_hash(rows[0][0].as_bytes()) != rows[0][1] {
        return denied();
    }
    let snapshot = parse_snapshot(tx, &rows[0][0])?;
    validate_snapshot(&snapshot)?;
    if snapshot.domain_id != domain_id
        || snapshot.session_id != session_id
        || snapshot.revision != object_version
        || snapshot_json(&snapshot) != rows[0][0].as_bytes()
    {
        return denied();
    }
    Ok(snapshot)
}

fn count_value(tx: &mut Transaction<'_, '_>, sql: &str, args: &[&str]) -> Result<u64> {
    let rows = tx.query(sql, args, 1)?;
    if rows.len() != 1 {
        return denied();
    }
    rows[0][0]
        .parse::<u64>()
        .map_err(|_| OrchestrationError::AccessDenied)
}

fn hash_valid(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn validate_history_record(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    session_id: &str,
    version_number: u64,
) -> Result<()> {
    let object_version = version_number.to_string();
    let counter = (version_number - 1).to_string();
    let stream_id = format!("gogoke.session-lineage.v1/{session_id}");
    let snapshot = decode_object_snapshot(tx, domain_id, session_id, &object_version)?;
    let rows = tx.query(
        "SELECT e.event_id,e.event_type,e.object_type,e.object_id,e.object_version,CAST(e.canonical_json AS TEXT),e.content_hash,r.receipt_id,r.operation_id,r.receipt_type,r.object_type,r.object_id,r.object_version,CAST(r.canonical_json AS TEXT),r.content_hash,r.operation_fingerprint,r.recorded_at,e.occurred_at,e.stream_counter FROM main.gogoke_events e JOIN main.gogoke_receipts r ON r.domain_id=e.domain_id AND r.event_id=e.event_id WHERE e.domain_id=? AND e.stream_id=? AND e.stream_counter=? AND e.object_version=?",
        &[domain_id, &stream_id, &counter, &object_version],
        19,
    )?;
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    if row[1] != "SessionLineageCommitted"
        || row[2] != "SessionLineage"
        || row[3] != session_id
        || row[4] != object_version
        || row[9] != "SessionLineageCommitted"
        || row[10] != "SessionLineage"
        || row[11] != session_id
        || row[12] != object_version
        || row[16] != row[17]
        || row[18] != counter
        || !hash_valid(&row[6])
        || content_hash(row[5].as_bytes()) != row[6]
        || !hash_valid(&row[14])
        || content_hash(row[13].as_bytes()) != row[14]
        || !hash_valid(&row[15])
    {
        return denied();
    }
    exact_keys(
        tx,
        &row[5],
        None,
        &[
            "contentHash",
            "operation",
            "operationId",
            "requestDigest",
            "sessionId",
            "sessionRevision",
            "type",
        ],
    )?;
    exact_keys(
        tx,
        &row[13],
        None,
        &[
            "authorityStatus",
            "contentHash",
            "domainId",
            "operationId",
            "requestDigest",
            "revision",
            "sessionId",
            "type",
        ],
    )?;
    let operation = json_text(tx, &row[5], "$.operation")?;
    let request_digest = json_text(tx, &row[5], "$.requestDigest")?;
    if !matches!(
        operation.as_str(),
        "NEW_CLEAN"
            | "RESUME"
            | "NATIVE_FORK"
            | "REBUILD"
            | "HANDOFF"
            | "ARCHIVE"
            | "RETAIN_PENDING_ACTION"
            | "APPEND_EXPOSURE_RECEIPT"
    ) || json_text(tx, &row[5], "$.operationId")? != row[8]
        || json_text(tx, &row[13], "$.operationId")? != row[8]
        || json_text(tx, &row[13], "$.requestDigest")? != request_digest
        || json_text(tx, &row[5], "$.sessionId")? != session_id
        || json_text(tx, &row[5], "$.sessionRevision")? != object_version
        || json_text(tx, &row[5], "$.contentHash")? != snapshot.content_hash
        || json_text(tx, &row[13], "$.authorityStatus")? != AUTHORITY_STATUS
        || json_text(tx, &row[13], "$.domainId")? != domain_id
        || json_text(tx, &row[13], "$.contentHash")? != snapshot.content_hash
        || json_text(tx, &row[13], "$.revision")? != object_version
        || json_text(tx, &row[13], "$.sessionId")? != session_id
        || !hash_valid(&request_digest)
    {
        return denied();
    }
    let expected_event = event_json(&operation, &row[8], &request_digest, &snapshot);
    let expected_receipt = receipt_json(domain_id, &row[8], &request_digest, &snapshot);
    if row[5] != expected_event || row[13] != expected_receipt {
        return denied();
    }
    let replay = tx.apply_domain_record(DomainRecordInput {
        domain_id: domain_id.to_owned(),
        object_type: "SessionLineage".into(),
        object_id: session_id.to_owned(),
        object_version: object_version.clone(),
        object_bytes: snapshot_json(&snapshot),
        native_identity: None,
        event_id: row[0].clone(),
        stream_id,
        expected_previous_counter: version_number.checked_sub(2).map(|value| value.to_string()),
        counter,
        event_type: "SessionLineageCommitted".into(),
        occurred_at: row[17].clone(),
        event_bytes: expected_event.into_bytes(),
        receipt_id: row[7].clone(),
        operation_id: row[8].clone(),
        receipt_type: "SessionLineageCommitted".into(),
        recorded_at: row[16].clone(),
        receipt_bytes: expected_receipt.into_bytes(),
    })?;
    if replay.operation_fingerprint != row[15]
        || replay.event_id != row[0]
        || replay.receipt_id != row[7]
        || replay.object_hash != content_hash(&snapshot_json(&snapshot))
        || replay.event_hash != row[6]
        || replay.receipt_hash != row[14]
    {
        return denied();
    }
    Ok(())
}

fn load_current(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    session_id: &str,
) -> Result<SessionSnapshot> {
    identifier(domain_id)?;
    identifier(session_id)?;
    ensure_schema(tx)?;
    let heads = tx.query(
        "SELECT revision,content_hash,native_session_id,binding_id,generation,source_epoch,lifecycle,object_type FROM main.gogoke_session_lineage_heads WHERE domain_id=? AND session_id=?",
        &[domain_id, session_id],
        8,
    )?;
    if heads.len() != 1 {
        return denied();
    }
    let head = &heads[0];
    let version_number = revision(&head[0])?;
    if version_number == 0 || head[0] != version_number.to_string() || head[7] != "SessionLineage" {
        return denied();
    }
    let snapshot = decode_object_snapshot(tx, domain_id, session_id, &head[0])?;
    if snapshot.content_hash != head[1]
        || snapshot.native.native_session_id != head[2]
        || snapshot.native.binding_id != head[3]
        || snapshot.native.generation != head[4]
        || snapshot.native.source_epoch != head[5]
        || snapshot.lifecycle != head[6]
    {
        return denied();
    }
    let latest = tx.query(
        "SELECT object_version FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",
        &[domain_id, session_id],
        1,
    )?;
    if latest.len() != 1 || latest[0][0] != head[0]
        || count_value(tx, "SELECT count(*) FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND object_id=?", &[domain_id, session_id])? != version_number
    {
        return denied();
    }
    let stream_id = format!("gogoke.session-lineage.v1/{session_id}");
    let expected_counter = (version_number - 1).to_string();
    let stream_heads = tx.query(
        "SELECT counter FROM main.gogoke_stream_heads WHERE domain_id=? AND stream_id=?",
        &[domain_id, &stream_id],
        1,
    )?;
    if stream_heads.len() != 1
        || stream_heads[0][0] != expected_counter
        || count_value(tx, "SELECT count(*) FROM main.gogoke_events WHERE domain_id=? AND stream_id=?", &[domain_id, &stream_id])? != version_number
        || count_value(tx, "SELECT count(*) FROM main.gogoke_receipts WHERE domain_id=? AND object_type='SessionLineage' AND object_id=?", &[domain_id, session_id])? != version_number
    {
        return denied();
    }
    for version in 1..=version_number {
        validate_history_record(tx, domain_id, session_id, version)?;
    }
    Ok(snapshot)
}

pub(crate) fn read_session_lineage(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
) -> Result<SessionSnapshot> {
    transaction::run(connection, |tx| {
        let _profile = current_profile(tx)?;
        load_current(tx, domain_id, session_id)
    })
}

pub(super) fn read_session_lineage_in_transaction(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    session_id: &str,
) -> Result<SessionSnapshot> {
    identifier(domain_id)?;
    identifier(session_id)?;
    let _profile = current_profile(tx)?;
    ensure_schema(tx)?;
    load_current(tx, domain_id, session_id)
}

pub(crate) fn read_exposure_receipt(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    receipt_id: &str,
) -> Result<StoredExposureReceipt> {
    identifier(domain_id)?;
    identifier(receipt_id)?;
    transaction::run(connection, |tx| {
        let _profile = current_profile(tx)?;
        ensure_schema(tx)?;
        let owners = tx.query(
            "SELECT count(DISTINCT object_id) FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND json_extract(CAST(canonical_json AS TEXT),'$.exposure.receiptId')=?",
            &[domain_id, receipt_id],
            1,
        )?;
        if owners.len() != 1 || owners[0][0] != "1" {
            return denied();
        }
        let first = tx.query(
            "SELECT object_id,object_version FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND json_extract(CAST(canonical_json AS TEXT),'$.exposure.receiptId')=? ORDER BY length(object_version),object_version LIMIT 1",
            &[domain_id, receipt_id],
            2,
        )?;
        if first.len() != 1 {
            return denied();
        }
        let session_id = first[0][0].clone();
        let session_revision = first[0][1].clone();
        // Validate the whole immutable lineage chain, not only the selected object.
        let current = load_current(tx, domain_id, &session_id)?;
        let first_snapshot = decode_object_snapshot(tx, domain_id, &session_id, &session_revision)?;
        let Some(exposure) = first_snapshot.exposure else {
            return denied();
        };
        if exposure.receipt_id != receipt_id {
            return denied();
        }
        let mut occurrences = 0u64;
        for version in 1..=revision(&current.revision)? {
            let snapshot =
                decode_object_snapshot(tx, domain_id, &session_id, &version.to_string())?;
            if let Some(candidate) = snapshot.exposure {
                if candidate.receipt_id == receipt_id {
                    occurrences += 1;
                    if candidate != exposure {
                        return denied();
                    }
                }
            }
        }
        if occurrences == 0 {
            return denied();
        }
        Ok(StoredExposureReceipt {
            domain_id: domain_id.to_owned(),
            session_id,
            session_revision,
            exposure,
        })
    })
}

fn empty_inherited() -> InheritedExposureSummary {
    InheritedExposureSummary {
        evidence_levels: Vec::new(),
        taint_labels: Vec::new(),
        unknown_sources: Vec::new(),
        source_receipt_refs: Vec::new(),
    }
}

fn make_snapshot(
    domain_id: &str,
    session_id: &str,
    revision_value: String,
    native: NativeSessionIdentity,
    operation_kind: &str,
    parents: Vec<String>,
    inherited: InheritedExposureSummary,
    lifecycle: &str,
    process_state: &str,
    exposure: Option<ExposureReceipt>,
    pending_actions: Vec<PendingActionRef>,
    material_handoff: Option<MaterialHandoff>,
    pending_action_disposition: &str,
) -> SessionSnapshot {
    let mut snapshot = SessionSnapshot {
        domain_id: domain_id.into(),
        session_id: session_id.into(),
        revision: revision_value,
        lineage: SessionLineage {
            session_id: session_id.into(),
            binding_id: native.binding_id.clone(),
            parent_refs: parents,
            operation_kind: operation_kind.into(),
            inherited_exposure: inherited,
            source_epoch: native.source_epoch.clone(),
        },
        native,
        lifecycle: lifecycle.into(),
        process_state: process_state.into(),
        exposure,
        exposure_assessment: ExposureAssessment {
            classification: String::new(),
            evidence_level: String::new(),
            taint_labels: Vec::new(),
            unknown_sources: Vec::new(),
            reason: String::new(),
        },
        pending_actions,
        material_handoff,
        pending_action_disposition: pending_action_disposition.into(),
        content_hash: String::new(),
    };
    seal_snapshot(&mut snapshot);
    snapshot
}

fn seal_snapshot(snapshot: &mut SessionSnapshot) {
    snapshot.exposure_assessment = exposure_assessment(
        snapshot.exposure.as_ref(),
        &snapshot.lineage.inherited_exposure,
    );
    snapshot.content_hash = content_hash(snapshot_preimage(snapshot).as_bytes());
}

fn ensure_session_absent(
    tx: &mut Transaction<'_, '_>,
    domain_id: &str,
    session_id: &str,
) -> Result<()> {
    let stream_id = format!("gogoke.session-lineage.v1/{session_id}");
    if !tx.query("SELECT 1 FROM main.gogoke_session_lineage_heads WHERE domain_id=? AND session_id=? LIMIT 1", &[domain_id, session_id], 1)?.is_empty()
        || !tx.query("SELECT 1 FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND object_id=? LIMIT 1", &[domain_id, session_id], 1)?.is_empty()
        || !tx.query("SELECT 1 FROM main.gogoke_stream_heads WHERE domain_id=? AND stream_id=? LIMIT 1", &[domain_id, &stream_id], 1)?.is_empty()
        || !tx.query("SELECT 1 FROM main.gogoke_events WHERE domain_id=? AND stream_id=? LIMIT 1", &[domain_id, &stream_id], 1)?.is_empty()
    {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok(())
}

fn ensure_native_identity_available(
    tx: &mut Transaction<'_, '_>,
    native: &NativeSessionIdentity,
    except_session: Option<&str>,
) -> Result<()> {
    let rows = tx.query(
        "SELECT session_id FROM main.gogoke_session_lineage_heads WHERE domain_id=? AND native_session_id=? AND binding_id=? AND generation=? AND source_epoch=?",
        &[&native.domain_id, &native.native_session_id, &native.binding_id, &native.generation, &native.source_epoch],
        1,
    )?;
    if rows
        .iter()
        .any(|row| Some(row[0].as_str()) != except_session)
    {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok(())
}

fn commit_snapshot(
    tx: &mut Transaction<'_, '_>,
    command: &SessionLineageCommand,
    snapshot: SessionSnapshot,
    digest: &str,
    previous: Option<&SessionSnapshot>,
) -> Result<SessionLineageReceipt> {
    validate_snapshot(&snapshot)?;
    let input = record_input(command, &snapshot, digest)?;
    let storage = tx.apply_domain_record(input)?;
    if let Some(previous) = previous {
        tx.write(
            "UPDATE main.gogoke_session_lineage_heads SET revision=?,content_hash=?,native_session_id=?,binding_id=?,generation=?,source_epoch=?,lifecycle=? WHERE domain_id=? AND session_id=? AND revision=? AND content_hash=?",
            &[&snapshot.revision, &snapshot.content_hash, &snapshot.native.native_session_id,
              &snapshot.native.binding_id, &snapshot.native.generation, &snapshot.native.source_epoch,
              &snapshot.lifecycle, &snapshot.domain_id, &snapshot.session_id,
              &previous.revision, &previous.content_hash],
        )?;
    } else {
        tx.write(
            "INSERT INTO main.gogoke_session_lineage_heads(domain_id,session_id,object_type,revision,content_hash,native_session_id,binding_id,generation,source_epoch,lifecycle) VALUES(?,?,'SessionLineage',?,?,?,?,?,?,?)",
            &[&snapshot.domain_id, &snapshot.session_id, &snapshot.revision, &snapshot.content_hash,
              &snapshot.native.native_session_id, &snapshot.native.binding_id, &snapshot.native.generation,
              &snapshot.native.source_epoch, &snapshot.lifecycle],
        )?;
    }
    let current = load_current(tx, &snapshot.domain_id, &snapshot.session_id)?;
    if current != snapshot {
        return denied();
    }
    Ok(SessionLineageReceipt {
        disposition: if previous.is_some() {
            "COMMITTED"
        } else {
            "COMMITTED"
        },
        authority_status: AUTHORITY_STATUS,
        operation_id: command.operation_id.clone(),
        snapshot,
        storage,
    })
}

fn find_operation_replay(
    tx: &mut Transaction<'_, '_>,
    command: &SessionLineageCommand,
    digest: &str,
) -> Result<Option<SessionLineageReceipt>> {
    let rows = tx.query(
        "SELECT r.receipt_id,r.event_id,r.recorded_at,CAST(r.canonical_json AS TEXT),r.content_hash,r.receipt_type,r.object_type,r.object_id,r.object_version,r.operation_fingerprint,e.stream_id,e.event_type,CAST(e.canonical_json AS TEXT),e.content_hash,e.object_type,e.object_id,e.object_version,e.stream_counter,e.occurred_at FROM main.gogoke_receipts r JOIN main.gogoke_events e ON e.domain_id=r.domain_id AND e.event_id=r.event_id WHERE r.domain_id=? AND r.operation_id=?",
        &[&command.domain_id, &command.operation_id],
        19,
    )?;
    if rows.is_empty() {
        return Ok(None);
    }
    if rows.len() != 1 {
        return denied();
    }
    let row = &rows[0];
    if row[5] != "SessionLineageCommitted"
        || row[6] != "SessionLineage"
        || row[11] != "SessionLineageCommitted"
        || row[14] != "SessionLineage"
        || row[7] != operation_session_id(&command.operation)
        || row[15] != row[7]
        || row[8] != row[16]
        || row[0] != command.receipt_id
        || row[1] != command.event_id
        || row[2] != command.recorded_at
        || row[10] != format!("gogoke.session-lineage.v1/{}", row[7])
        || row[17] != revision(&row[8])?.saturating_sub(1).to_string()
        || !hash_valid(&row[4])
        || content_hash(row[3].as_bytes()) != row[4]
        || !hash_valid(&row[13])
        || content_hash(row[12].as_bytes()) != row[13]
        || !hash_valid(&row[9])
        || row[18] != row[2]
    {
        return Err(OrchestrationError::OperationConflict);
    }
    let snapshot = decode_object_snapshot(tx, &command.domain_id, &row[7], &row[8])?;
    exact_keys(
        tx,
        &row[3],
        None,
        &[
            "authorityStatus",
            "contentHash",
            "domainId",
            "operationId",
            "requestDigest",
            "revision",
            "sessionId",
            "type",
        ],
    )?;
    exact_keys(
        tx,
        &row[12],
        None,
        &[
            "contentHash",
            "operation",
            "operationId",
            "requestDigest",
            "sessionId",
            "sessionRevision",
            "type",
        ],
    )?;
    let event = event_json(
        operation_name(&command.operation),
        &command.operation_id,
        digest,
        &snapshot,
    );
    let receipt = receipt_json(&command.domain_id, &command.operation_id, digest, &snapshot);
    if row[3] != receipt
        || row[12] != event
        || json_text(tx, &row[3], "$.authorityStatus")? != AUTHORITY_STATUS
        || json_text(tx, &row[3], "$.requestDigest")? != digest
        || json_text(tx, &row[12], "$.requestDigest")? != digest
    {
        return Err(OrchestrationError::OperationConflict);
    }
    let current = load_current(tx, &command.domain_id, &snapshot.session_id)?;
    if revision(&current.revision)? < revision(&snapshot.revision)? {
        return denied();
    }
    let storage = tx.apply_domain_record(record_input(command, &snapshot, digest)?)?;
    Ok(Some(SessionLineageReceipt {
        disposition: "REPLAYED",
        authority_status: AUTHORITY_STATUS,
        operation_id: command.operation_id.clone(),
        snapshot,
        storage,
    }))
}

fn expected_revision(current: &SessionSnapshot, requested: &str) -> Result<()> {
    if current.revision != requested {
        return Err(OrchestrationError::OperationConflict);
    }
    Ok(())
}

fn build_operation_snapshot(
    tx: &mut Transaction<'_, '_>,
    command: &SessionLineageCommand,
) -> Result<(SessionSnapshot, Option<SessionSnapshot>)> {
    let domain_id = command.domain_id.as_str();
    match &command.operation {
        SessionLineageOperation::NewClean { session_id, native } => {
            ensure_session_absent(tx, domain_id, session_id)?;
            ensure_native_identity_available(tx, native, None)?;
            let snapshot = make_snapshot(
                domain_id,
                session_id,
                "1".into(),
                native.clone(),
                "NEW_CLEAN",
                Vec::new(),
                empty_inherited(),
                "ACTIVE",
                "RUNNING",
                None,
                Vec::new(),
                None,
                "NONE",
            );
            Ok((snapshot, None))
        }
        SessionLineageOperation::Resume {
            session_id,
            expected_revision: expected,
            native,
        } => {
            let current = load_current(tx, domain_id, session_id)?;
            expected_revision(&current, expected)?;
            if current.lifecycle == "ARCHIVED" || current.native != *native {
                return denied();
            }
            let revision_value = next_revision(&current.revision)?;
            let disposition = if current.pending_actions.is_empty() {
                "NONE"
            } else {
                "RETAINED_NOT_REPLAYED"
            };
            let mut next = current.clone();
            next.revision = revision_value;
            next.lineage.operation_kind = "RESUME".into();
            next.pending_action_disposition = disposition.into();
            seal_snapshot(&mut next);
            Ok((next, Some(current)))
        }
        SessionLineageOperation::NativeFork {
            session_id,
            parent_session_id,
            expected_parent_revision,
            native,
        }
        | SessionLineageOperation::Rebuild {
            session_id,
            parent_session_id,
            expected_parent_revision,
            native,
        } => {
            let parent = load_current(tx, domain_id, parent_session_id)?;
            expected_revision(&parent, expected_parent_revision)?;
            ensure_session_absent(tx, domain_id, session_id)?;
            if native.native_session_id == parent.native.native_session_id
                || native.binding_id == parent.native.binding_id
            {
                return denied();
            }
            ensure_native_identity_available(tx, native, None)?;
            let (operation_kind, snapshot) = match &command.operation {
                SessionLineageOperation::NativeFork { .. } => ("NATIVE_FORK", "NATIVE_FORK"),
                _ => ("REBUILD", "REBUILD"),
            };
            let pending_disposition = if parent.pending_actions.is_empty() {
                "NONE"
            } else {
                "RETAINED_ON_PARENT"
            };
            let next = make_snapshot(
                domain_id,
                session_id,
                "1".into(),
                native.clone(),
                snapshot,
                vec![parent.session_id.clone()],
                inherited_summary(&parent.lineage.inherited_exposure, parent.exposure.as_ref()),
                "ACTIVE",
                "RUNNING",
                None,
                Vec::new(),
                None,
                pending_disposition,
            );
            debug_assert_eq!(operation_kind, next.lineage.operation_kind);
            Ok((next, None))
        }
        SessionLineageOperation::Handoff {
            session_id,
            parent_session_id,
            expected_parent_revision,
            native,
            material_ids,
        } => {
            let parent = load_current(tx, domain_id, parent_session_id)?;
            expected_revision(&parent, expected_parent_revision)?;
            ensure_session_absent(tx, domain_id, session_id)?;
            if native.native_session_id == parent.native.native_session_id
                || native.binding_id == parent.native.binding_id
            {
                return denied();
            }
            ensure_native_identity_available(tx, native, None)?;
            let pending_disposition = if parent.pending_actions.is_empty() {
                "NONE"
            } else {
                "RETAINED_ON_PARENT"
            };
            let next = make_snapshot(
                domain_id,
                session_id,
                "1".into(),
                native.clone(),
                "HANDOFF",
                vec![parent.session_id.clone()],
                inherited_summary(&parent.lineage.inherited_exposure, parent.exposure.as_ref()),
                "ACTIVE",
                "RUNNING",
                None,
                Vec::new(),
                Some(MaterialHandoff {
                    material_ids: material_ids.clone(),
                    source_session_id: parent.session_id.clone(),
                    native_resume_used: false,
                }),
                pending_disposition,
            );
            Ok((next, None))
        }
        SessionLineageOperation::Archive {
            session_id,
            expected_revision: expected,
        } => {
            let current = load_current(tx, domain_id, session_id)?;
            expected_revision(&current, expected)?;
            if current.lifecycle == "ARCHIVED" {
                return Err(OrchestrationError::OperationConflict);
            }
            let mut next = current.clone();
            next.revision = next_revision(&current.revision)?;
            next.lifecycle = "ARCHIVED".into();
            next.lineage.operation_kind = "ARCHIVE".into();
            // ARCHIVE changes only the lifecycle; processState remains the recorded observation.
            seal_snapshot(&mut next);
            Ok((next, Some(current)))
        }
        SessionLineageOperation::RetainPendingAction {
            session_id,
            expected_revision: expected,
            action,
        } => {
            let current = load_current(tx, domain_id, session_id)?;
            expected_revision(&current, expected)?;
            validate_action(action, &current.native)?;
            if current
                .pending_actions
                .iter()
                .any(|existing| existing.action_id == action.action_id)
            {
                return Err(OrchestrationError::OperationConflict);
            }
            let mut next = current.clone();
            next.revision = next_revision(&current.revision)?;
            next.pending_actions.push(action.clone());
            if next.lineage.operation_kind == "RESUME" {
                next.pending_action_disposition = "RETAINED_NOT_REPLAYED".into();
            }
            seal_snapshot(&mut next);
            Ok((next, Some(current)))
        }
        SessionLineageOperation::AppendExposureReceipt {
            session_id,
            expected_revision: expected,
            exposure,
        } => {
            let current = load_current(tx, domain_id, session_id)?;
            expected_revision(&current, expected)?;
            validate_exposure(exposure, &current.native)?;
            let duplicate = tx.query(
                "SELECT 1 FROM main.gogoke_objects WHERE domain_id=? AND object_type='SessionLineage' AND json_extract(CAST(canonical_json AS TEXT),'$.exposure.receiptId')=? LIMIT 1",
                &[domain_id, &exposure.receipt_id],
                1,
            )?;
            if !duplicate.is_empty() {
                return Err(OrchestrationError::OperationConflict);
            }
            let mut next = current.clone();
            next.revision = next_revision(&current.revision)?;
            if current.exposure.is_some() {
                next.lineage.inherited_exposure = inherited_summary(
                    &current.lineage.inherited_exposure,
                    current.exposure.as_ref(),
                );
            }
            next.exposure = Some(exposure.clone());
            seal_snapshot(&mut next);
            Ok((next, Some(current)))
        }
    }
}

fn apply_session_lineage_in_transaction(
    tx: &mut Transaction<'_, '_>,
    command: &SessionLineageCommand,
) -> Result<SessionLineageReceipt> {
    ensure_schema(tx)?;
    let _profile = current_profile(tx)?;
    let digest = command_digest(command);
    if let Some(replay) = find_operation_replay(tx, command, &digest)? {
        return Ok(replay);
    }
    let (snapshot, previous) = build_operation_snapshot(tx, command)?;
    commit_snapshot(tx, command, snapshot, &digest, previous.as_ref())
}

pub(crate) fn apply_session_lineage_command(
    connection: &mut VerifiedDatabaseConnection<'_>,
    command: &SessionLineageCommand,
) -> Result<SessionLineageReceipt> {
    validate_command(command)?;
    transaction::run(connection, |tx| {
        apply_session_lineage_in_transaction(tx, command)
    })
}
