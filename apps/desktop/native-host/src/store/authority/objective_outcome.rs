//! Append-only OBJECTIVE outcomes derived from Product Authority records.
//!
//! This ingress contains no actor-supplied label. It accepts only canonical,
//! same-domain records whose typed authority receipts can be resolved here.
//! Owner-authored corrections remain in `outcome.rs` and retain their private
//! Owner capability.
use super::super::atomic::{
    canonical_object_without_string_field, DomainRecordInput, DomainRecordReceipt,
};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::decision_replay::read_in_transaction;
use super::model::{denied, identifier, revision};
use super::task_package::read_authorized_task_package_in_transaction;
use super::transaction::{self, Result, Transaction};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectiveEvidenceRef {
    pub object_type: String,
    pub object_id: String,
    pub object_version: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectiveVersionRef {
    pub revision: String,
    pub content_hash: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ObjectiveObservationWindow {
    pub starts_at: String,
    pub ends_at: String,
    pub status: String,
}

/// Inputs are references only. The objective label, project identity and
/// completion state are derived from native records inside the transaction.
#[derive(Clone, Debug)]
pub(crate) struct AppendObjectiveOutcome {
    pub domain_id: String,
    pub outcome_id: String,
    pub revision: String,
    pub operation_id: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
    pub manifest_id: String,
    pub manifest_version: String,
    pub manifest_hash: String,
    pub decision_id: String,
    pub decision_version: String,
    pub decision_hash: String,
    pub action_operation_id: String,
    pub action_completion_ref: String,
    pub result_refs: Vec<ObjectiveEvidenceRef>,
    pub evidence_refs: Vec<ObjectiveEvidenceRef>,
    pub observation: ObjectiveObservationWindow,
    pub previous: Option<ObjectiveVersionRef>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObjectiveOutcomeVersion {
    pub domain_id: String,
    pub outcome_id: String,
    pub revision: String,
    pub content_hash: String,
    pub canonical_outcome: Vec<u8>,
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
            c if (c as u32) < 0x20 => output.push_str(&format!("\\u{:04x}", c as u32)),
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

fn canonical_object(mut fields: Vec<(String, String)>) -> String {
    fields.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    format!(
        "{{{}}}",
        fields
            .into_iter()
            .map(|(key, value)| format!("{}:{value}", quote(&key)))
            .collect::<Vec<_>>()
            .join(",")
    )
}

pub(super) fn canonical_refs(refs: &[ObjectiveEvidenceRef]) -> Result<String> {
    if refs.len() > 64 {
        return denied();
    }
    let mut checked = refs.to_vec();
    for item in &checked {
        if !matches!(
            item.object_type.as_str(),
            "ActionCompletion" | "ContextManifest" | "DecisionRecord"
        ) {
            return denied();
        }
        identifier(&item.object_id)?;
        revision(&item.object_version)?;
        hash(&item.content_hash)?;
    }
    checked.sort_by(|a, b| {
        (&a.object_type, &a.object_id, &a.object_version).cmp(&(
            &b.object_type,
            &b.object_id,
            &b.object_version,
        ))
    });
    if checked.windows(2).any(|pair| {
        pair[0].object_type == pair[1].object_type
            && pair[0].object_id == pair[1].object_id
            && pair[0].object_version == pair[1].object_version
    }) {
        return denied();
    }
    Ok(format!(
        "[{}]",
        checked
            .iter()
            .map(|item| canonical_object(vec![
                ("contentHash".into(), quote(&item.content_hash)),
                ("objectId".into(), quote(&item.object_id)),
                ("objectType".into(), quote(&item.object_type)),
                ("objectVersion".into(), quote(&item.object_version)),
            ]))
            .collect::<Vec<_>>()
            .join(",")
    ))
}

fn json_text(tx: &mut Transaction<'_, '_>, json: &str, path: &str) -> Result<String> {
    let rows = tx.query(
        "SELECT json_type(?,?),json_extract(?,?)",
        &[json, path, json, path],
        2,
    )?;
    if rows.len() != 1 || rows[0][0] != "text" || rows[0][1].is_empty() {
        return denied();
    }
    Ok(rows[0][1].clone())
}

pub(super) fn validate_record_ref(
    tx: &mut Transaction<'_, '_>,
    domain: &str,
    item: &ObjectiveEvidenceRef,
) -> Result<()> {
    let rows=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),r.receipt_type,r.object_id,r.object_version,r.content_hash,CAST(r.canonical_json AS TEXT),r.operation_id,r.receipt_id FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version WHERE o.domain_id=? AND o.object_type=? AND o.object_id=? AND o.object_version=?",&[domain,&item.object_type,&item.object_id,&item.object_version],9)?;
    if rows.len() != 1
        || rows[0][0] != item.content_hash
        || rows[0][5].is_empty()
        || content_hash(rows[0][1].as_bytes()) != rows[0][0]
        || content_hash(rows[0][6].as_bytes()) != rows[0][5]
        || rows[0][3] != item.object_id
        || rows[0][4] != item.object_version
    {
        return denied();
    }
    let allowed = match item.object_type.as_str() {
        "ActionCompletion" => rows[0][2] == "ActionCompletionRecorded",
        "ContextManifest" => rows[0][2] == "ContextManifestCommitted",
        "DecisionRecord" => rows[0][2] == "DecisionApplied",
        _ => false,
    };
    if !allowed {
        return denied();
    }
    match item.object_type.as_str() {
        "ActionCompletion" => {
            let (state, record_hash, _) = completion(tx, domain, &item.object_id, &rows[0][8])?;
            if !matches!(state.as_str(), "COMPLETED" | "REJECTED")
                || record_hash != item.content_hash
            {
                return denied();
            }
        }
        "ContextManifest" => {
            let embedded = json_text(tx, &rows[0][1], "$.manifestHash")?;
            if validate_manifest(tx, domain, &item.object_id, &item.object_version, &embedded)?.0
                != item.content_hash
            {
                return denied();
            }
        }
        "DecisionRecord" => {
            let decision = read_in_transaction(tx, domain, &rows[0][7])?;
            if decision.decision_id != item.object_id
                || decision.object_version != item.object_version
                || decision.decision_content_hash != item.content_hash
            {
                return denied();
            }
        }
        _ => return denied(),
    }
    Ok(())
}

pub(super) fn validate_manifest(
    tx: &mut Transaction<'_, '_>,
    domain: &str,
    id: &str,
    version: &str,
    expected_hash: &str,
) -> Result<(String, String)> {
    let rows = tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),r.operation_id,r.content_hash,CAST(r.canonical_json AS TEXT),e.content_hash,CAST(e.canonical_json AS TEXT) FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version JOIN main.gogoke_events e ON e.domain_id=r.domain_id AND e.event_id=r.event_id WHERE o.domain_id=? AND o.object_type='ContextManifest' AND o.object_id=? AND o.object_version=? AND r.receipt_type='ContextManifestCommitted'", &[domain,id,version],7)?;
    if rows.len() != 1
        || content_hash(rows[0][1].as_bytes()) != rows[0][0]
        || content_hash(rows[0][4].as_bytes()) != rows[0][3]
        || content_hash(rows[0][6].as_bytes()) != rows[0][5]
    {
        return denied();
    }
    let (body, manifest_hash) = canonical_object_without_string_field(
        rows[0][1].as_bytes(),
        "manifestHash",
        "ObjectiveOutcome.Manifest",
    )
    .map_err(OrchestrationError::Atomic)?;
    if manifest_hash != expected_hash
        || content_hash(&body) != manifest_hash
        || json_text(tx, &rows[0][1], "$.manifestId")? != id
        || json_text(tx, &rows[0][1], "$.domainId")? != domain
        || json_text(tx, &rows[0][4], "$.manifestHash")? != manifest_hash
        || json_text(tx, &rows[0][4], "$.schema")? != "gogoke.context-manifest-commit.v1"
        || json_text(tx, &rows[0][6], "$.manifestHash")? != manifest_hash
        || json_text(tx, &rows[0][6], "$.manifestId")? != id
        || json_text(tx, &rows[0][6], "$.operationId")? != rows[0][2]
    {
        return denied();
    }
    let snapshot=tx.query("SELECT operation_id FROM main.gogoke_context_assembly_snapshots WHERE domain_id=? AND manifest_id=?",&[domain,id],1)?;
    if snapshot.len() != 1 || snapshot[0][0] != rows[0][2] {
        return denied();
    }
    Ok((rows[0][0].clone(), manifest_hash))
}

pub(super) fn completion(
    tx: &mut Transaction<'_, '_>,
    domain: &str,
    operation: &str,
    completion_ref: &str,
) -> Result<(String, String, String)> {
    identifier(operation)?;
    identifier(completion_ref)?;
    let rows=tx.query("SELECT reservation_id,semantic_digest,attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,native_request_id,native_session_id,trusted_receipt_ref,evidence_hash,disposition,receipt_id FROM main.gogoke_action_completion_receipts WHERE domain_id=? AND operation_id=?",&[domain,operation],14)?;
    if rows.len() != 1 {
        return denied();
    }
    let r = &rows[0];
    if !matches!(r[12].as_str(), "COMPLETED" | "REJECTED") || r[13] != completion_ref {
        return denied();
    }
    let object = canonical_object(vec![
        ("attemptId".into(), quote(&r[2])),
        ("bindingId".into(), quote(&r[4])),
        ("disposition".into(), quote(&r[12])),
        ("evidenceHash".into(), quote(&r[11])),
        ("generation".into(), quote(&r[5])),
        ("nativeRequestId".into(), quote(&r[8])),
        ("nativeSessionId".into(), quote(&r[9])),
        ("operationId".into(), quote(operation)),
        ("reservationId".into(), quote(&r[0])),
        ("runtimeInstanceId".into(), quote(&r[7])),
        ("semanticDigest".into(), quote(&r[1])),
        ("sendAuthority".into(), quote(&r[3])),
        ("sourceEpoch".into(), quote(&r[6])),
        ("trustedReceiptRef".into(), quote(&r[10])),
    ])
    .into_bytes();
    let time = tx.query(
        "SELECT recorded_at FROM main.gogoke_receipts WHERE domain_id=? AND receipt_id=?",
        &[domain, &r[13]],
        1,
    )?;
    if time.len() != 1 {
        return denied();
    }
    let object_hash = content_hash(&object);
    let event_id = format!(
        "action-event:{}",
        &content_hash(
            format!("{domain}\0ActionCompletion\0complete:{operation}\0event").as_bytes()
        )[7..]
    );
    let receipt_id = format!(
        "action-receipt:{}",
        &content_hash(
            format!("{domain}\0ActionCompletion\0complete:{operation}\0receipt").as_bytes()
        )[7..]
    );
    let completion_operation = format!("complete:{operation}");
    let event = canonical_object(vec![
        ("objectHash".into(), quote(&object_hash)),
        ("operationId".into(), quote(&completion_operation)),
        ("type".into(), quote("ActionCompletionRecorded")),
    ]);
    let receipt = canonical_object(vec![
        ("objectHash".into(), quote(&object_hash)),
        ("operationId".into(), quote(&completion_operation)),
        ("schema".into(), quote("ActionCompletionRecorded")),
    ]);
    let records=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),e.event_id,e.content_hash,CAST(e.canonical_json AS TEXT),r.receipt_id,r.content_hash,CAST(r.canonical_json AS TEXT),r.event_id,r.object_type,r.object_id,r.object_version,r.receipt_type FROM main.gogoke_objects o JOIN main.gogoke_events e ON e.domain_id=o.domain_id JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id WHERE o.domain_id=? AND o.object_type='ActionCompletion' AND o.object_id=? AND o.object_version='1' AND r.operation_id=? AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version AND e.event_id=r.event_id",&[domain,operation,&format!("complete:{operation}")],13)?;
    if records.len() != 1
        || records[0][0] != object_hash
        || records[0][1].as_bytes() != object.as_slice()
        || content_hash(records[0][1].as_bytes()) != records[0][0]
        || records[0][2] != event_id
        || records[0][3] != content_hash(event.as_bytes())
        || records[0][4].as_bytes() != event.as_bytes()
        || records[0][5] != receipt_id
        || records[0][6] != content_hash(receipt.as_bytes())
        || records[0][7].as_bytes() != receipt.as_bytes()
        || records[0][8] != event_id
        || records[0][9] != "ActionCompletion"
        || records[0][10] != operation
        || records[0][11] != "1"
        || records[0][12] != "ActionCompletionRecorded"
    {
        return denied();
    }
    let native=tx.query("SELECT receipt_ref,reservation_id,semantic_digest,attempt_id,send_authority,binding_id,generation,source_epoch,runtime_instance_id,native_request_id,native_session_id,evidence_hash,disposition,receipt_id FROM main.gogoke_action_native_receipts WHERE domain_id=? AND operation_id=?",&[domain,operation],14)?;
    if native.len() != 1
        || native[0][0] != r[10]
        || native[0][1] != r[0]
        || native[0][2] != r[1]
        || native[0][3] != r[2]
        || native[0][4] != r[3]
        || native[0][5] != r[4]
        || native[0][6] != r[5]
        || native[0][7] != r[6]
        || native[0][8] != r[7]
        || native[0][9] != r[8]
        || native[0][10] != r[9]
        || native[0][11] != r[11]
        || native[0][12] != r[12]
    {
        return denied();
    }
    let native_op = format!("native-receipt:{operation}");
    let native_object = canonical_object(vec![
        ("attemptId".into(), quote(&native[0][3])),
        ("bindingId".into(), quote(&native[0][5])),
        ("disposition".into(), quote(&native[0][12])),
        ("evidenceHash".into(), quote(&native[0][11])),
        ("generation".into(), quote(&native[0][6])),
        ("nativeRequestId".into(), quote(&native[0][9])),
        ("nativeSessionId".into(), quote(&native[0][10])),
        ("operationId".into(), quote(operation)),
        ("runtimeInstanceId".into(), quote(&native[0][8])),
        ("semanticDigest".into(), quote(&native[0][2])),
        ("sendAuthority".into(), quote(&native[0][4])),
        ("sourceEpoch".into(), quote(&native[0][7])),
        ("trustedReceiptRef".into(), quote(&native[0][0])),
    ])
    .into_bytes();
    let native_time = tx.query(
        "SELECT recorded_at FROM main.gogoke_receipts WHERE domain_id=? AND receipt_id=?",
        &[domain, &native[0][13]],
        1,
    )?;
    if native_time.len() != 1 {
        return denied();
    }
    let native_hash = content_hash(&native_object);
    let native_event_id = format!(
        "action-event:{}",
        &content_hash(format!("{domain}\0ActionNativeReceipt\0{native_op}\0event").as_bytes())[7..]
    );
    let native_receipt_id = format!(
        "action-receipt:{}",
        &content_hash(format!("{domain}\0ActionNativeReceipt\0{native_op}\0receipt").as_bytes())
            [7..]
    );
    let native_event = canonical_object(vec![
        ("objectHash".into(), quote(&native_hash)),
        ("operationId".into(), quote(&native_op)),
        ("type".into(), quote("ActionNativeReceiptRecorded")),
    ]);
    let native_receipt = canonical_object(vec![
        ("objectHash".into(), quote(&native_hash)),
        ("operationId".into(), quote(&native_op)),
        ("schema".into(), quote("ActionNativeReceiptRecorded")),
    ]);
    let native_records=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),e.event_id,e.content_hash,CAST(e.canonical_json AS TEXT),r.receipt_id,r.content_hash,CAST(r.canonical_json AS TEXT),r.event_id,r.object_type,r.object_id,r.object_version,r.receipt_type FROM main.gogoke_objects o JOIN main.gogoke_events e ON e.domain_id=o.domain_id JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id WHERE o.domain_id=? AND o.object_type='ActionNativeReceipt' AND o.object_id=? AND o.object_version='1' AND r.operation_id=? AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version AND e.event_id=r.event_id",&[domain,operation,&native_op],13)?;
    if native_records.len() != 1
        || native_records[0][0] != native_hash
        || native_records[0][1].as_bytes() != native_object.as_slice()
        || content_hash(native_records[0][1].as_bytes()) != native_records[0][0]
        || native_records[0][2] != native_event_id
        || native_records[0][3] != content_hash(native_event.as_bytes())
        || native_records[0][4].as_bytes() != native_event.as_bytes()
        || native_records[0][5] != native_receipt_id
        || native_records[0][6] != content_hash(native_receipt.as_bytes())
        || native_records[0][7].as_bytes() != native_receipt.as_bytes()
        || native_records[0][8] != native_event_id
        || native_records[0][9] != "ActionNativeReceipt"
        || native_records[0][10] != operation
        || native_records[0][11] != "1"
        || native_records[0][12] != "ActionNativeReceiptRecorded"
    {
        return denied();
    }
    Ok((r[12].clone(), object_hash, r[0].clone()))
}

fn resolve(
    tx: &mut Transaction<'_, '_>,
    input: &AppendObjectiveOutcome,
) -> Result<(String, String, String, Vec<u8>)> {
    for value in [
        &input.domain_id,
        &input.outcome_id,
        &input.operation_id,
        &input.event_id,
        &input.receipt_id,
        &input.manifest_id,
        &input.decision_id,
        &input.action_operation_id,
        &input.action_completion_ref,
    ] {
        identifier(value)?;
    }
    let revision_number = revision(&input.revision)?;
    if revision_number == 0 {
        return denied();
    }
    revision(&input.manifest_version)?;
    revision(&input.decision_version)?;
    hash(&input.manifest_hash)?;
    hash(&input.decision_hash)?;
    if input.observation.starts_at.is_empty()
        || input.observation.ends_at.is_empty()
        || input.observation.starts_at >= input.observation.ends_at
        || !matches!(
            input.observation.status.as_str(),
            "PENDING" | "OBSERVED" | "CENSORED"
        )
    {
        return denied();
    }
    let (manifest_record_hash, _) = validate_manifest(
        tx,
        &input.domain_id,
        &input.manifest_id,
        &input.manifest_version,
        &input.manifest_hash,
    )?;
    let decision_row=tx.query("SELECT operation_id FROM main.gogoke_receipts WHERE domain_id=? AND object_type='DecisionRecord' AND object_id=? AND object_version=? AND receipt_type='DecisionApplied'",&[&input.domain_id,&input.decision_id,&input.decision_version],1)?;
    if decision_row.len() != 1 {
        return denied();
    }
    let decision = read_in_transaction(tx, &input.domain_id, &decision_row[0][0])?;
    if decision.decision_id != input.decision_id
        || decision.object_version != input.decision_version
        || decision.decision_content_hash != input.decision_hash
        || decision.action_intent_ref != input.action_operation_id
    {
        return denied();
    }
    let (completion_state, completion_hash, semantic_digest) = completion(
        tx,
        &input.domain_id,
        &input.action_operation_id,
        &input.action_completion_ref,
    )?;
    let intent=tx.query("SELECT package_operation_id,target_domain_id FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?",&[&input.domain_id,&input.action_operation_id],2)?;
    if intent.len() != 1 || intent[0][1] != input.domain_id {
        return denied();
    }
    let package = read_authorized_task_package_in_transaction(tx, &input.domain_id, &intent[0][0])?
        .ok_or(OrchestrationError::AccessDenied)?;
    if package.target.domain_id != input.domain_id {
        return denied();
    }
    let result_refs = canonical_refs(&input.result_refs)?;
    let evidence_refs = canonical_refs(&input.evidence_refs)?;
    if input.result_refs.is_empty() || input.evidence_refs.is_empty() {
        return denied();
    }
    for item in input.result_refs.iter().chain(input.evidence_refs.iter()) {
        validate_record_ref(tx, &input.domain_id, item)?;
    }
    let identity_fields = vec![
        (
            "actionCompletionRef".into(),
            quote(&input.action_completion_ref),
        ),
        ("actionCompletionState".into(), quote(&completion_state)),
        (
            "actionOperationId".into(),
            quote(&input.action_operation_id),
        ),
        ("actionSemanticDigest".into(), quote(&semantic_digest)),
        ("decisionHash".into(), quote(&input.decision_hash)),
        ("decisionId".into(), quote(&input.decision_id)),
        ("decisionRevision".into(), quote(&input.decision_version)),
        ("domainId".into(), quote(&input.domain_id)),
        ("evidenceRefs".into(), evidence_refs),
        ("labelSource".into(), quote("OBJECTIVE")),
        ("manifestHash".into(), quote(&input.manifest_hash)),
        ("manifestId".into(), quote(&input.manifest_id)),
        ("manifestRecordHash".into(), quote(&manifest_record_hash)),
        ("manifestVersion".into(), quote(&input.manifest_version)),
        (
            "observationWindow".into(),
            canonical_object(vec![
                ("endsAt".into(), quote(&input.observation.ends_at)),
                ("startsAt".into(), quote(&input.observation.starts_at)),
                ("status".into(), quote(&input.observation.status)),
            ]),
        ),
        ("outcomeId".into(), quote(&input.outcome_id)),
        ("projectId".into(), quote(&package.source.project_id)),
        ("resultRefs".into(), result_refs),
        ("revision".into(), quote(&input.revision)),
    ];
    let body = canonical_object(identity_fields).into_bytes();
    Ok((
        completion_state,
        completion_hash,
        package.source.project_id,
        body,
    ))
}

fn apply(
    tx: &mut Transaction<'_, '_>,
    input: &AppendObjectiveOutcome,
) -> Result<DomainRecordReceipt> {
    let (_, _, _, body) = resolve(tx, input)?;
    let replay=tx.query("SELECT receipt_id,event_id,object_type,object_id,object_version FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",&[&input.domain_id,&input.operation_id],5)?;
    if !replay.is_empty() {
        if replay.len() != 1
            || replay[0][0] != input.receipt_id
            || replay[0][1] != input.event_id
            || replay[0][2] != "OutcomeRecord"
            || replay[0][3] != input.outcome_id
            || replay[0][4] != input.revision
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let stored=tx.query("SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='OutcomeRecord' AND object_id=? AND object_version=?",&[&input.domain_id,&input.outcome_id,&input.revision],1)?;
        if stored.len() != 1 || stored[0][0].as_bytes() != body.as_slice() {
            return Err(OrchestrationError::OperationConflict);
        }
        let event = canonical_object(vec![
            ("contentHash".into(), quote(&content_hash(&body))),
            ("outcomeId".into(), quote(&input.outcome_id)),
            (
                "previousHash".into(),
                input
                    .previous
                    .as_ref()
                    .map(|p| quote(&p.content_hash))
                    .unwrap_or_else(|| "null".into()),
            ),
            ("revision".into(), quote(&input.revision)),
        ]);
        let receipt = canonical_object(vec![
            ("schema".into(), quote("gogoke.objective-outcome.v1")),
            ("source".into(), quote("PRODUCT_AUTHORITY")),
        ]);
        let record = DomainRecordInput {
            domain_id: input.domain_id.clone(),
            object_type: "OutcomeRecord".into(),
            object_id: input.outcome_id.clone(),
            object_version: input.revision.clone(),
            object_bytes: body,
            native_identity: None,
            event_id: input.event_id.clone(),
            stream_id: format!("gogoke.objective-outcome.v1/{}", input.outcome_id),
            expected_previous_counter: revision(&input.revision)?
                .checked_sub(2)
                .map(|n| n.to_string()),
            counter: (revision(&input.revision)? - 1).to_string(),
            event_type: "ObjectiveOutcomeAppended".into(),
            occurred_at: input.recorded_at.clone(),
            event_bytes: event.into_bytes(),
            receipt_id: input.receipt_id.clone(),
            operation_id: input.operation_id.clone(),
            receipt_type: "ObjectiveOutcomeAppended".into(),
            recorded_at: input.recorded_at.clone(),
            receipt_bytes: receipt.into_bytes(),
        };
        return tx.apply_domain_record(record);
    }
    let current=tx.query("SELECT object_version,content_hash,CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='OutcomeRecord' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",&[&input.domain_id,&input.outcome_id],3)?;
    let prev_hash = match (&input.previous, current.first()) {
        (None, None) if input.revision == "1" => None,
        (Some(prev), Some(row))
            if revision(&prev.revision)?.checked_add(1) == Some(revision(&input.revision)?)
                && row[0] == prev.revision
                && row[1] == prev.content_hash =>
        {
            hash(&prev.content_hash)?;
            Some(prev.content_hash.clone())
        }
        _ => return Err(OrchestrationError::OperationConflict),
    };
    let previous = prev_hash
        .map(|h| quote(&h))
        .unwrap_or_else(|| "null".into());
    let event = canonical_object(vec![
        ("contentHash".into(), quote(&content_hash(&body))),
        ("outcomeId".into(), quote(&input.outcome_id)),
        ("previousHash".into(), previous),
        ("revision".into(), quote(&input.revision)),
    ]);
    let receipt = canonical_object(vec![
        ("schema".into(), quote("gogoke.objective-outcome.v1")),
        ("source".into(), quote("PRODUCT_AUTHORITY")),
    ]);
    tx.apply_domain_record(DomainRecordInput {
        domain_id: input.domain_id.clone(),
        object_type: "OutcomeRecord".into(),
        object_id: input.outcome_id.clone(),
        object_version: input.revision.clone(),
        object_bytes: body,
        native_identity: None,
        event_id: input.event_id.clone(),
        stream_id: format!("gogoke.objective-outcome.v1/{}", input.outcome_id),
        expected_previous_counter: revision(&input.revision)?
            .checked_sub(2)
            .map(|n| n.to_string()),
        counter: (revision(&input.revision)? - 1).to_string(),
        event_type: "ObjectiveOutcomeAppended".into(),
        occurred_at: input.recorded_at.clone(),
        event_bytes: event.into_bytes(),
        receipt_id: input.receipt_id.clone(),
        operation_id: input.operation_id.clone(),
        receipt_type: "ObjectiveOutcomeAppended".into(),
        recorded_at: input.recorded_at.clone(),
        receipt_bytes: receipt.into_bytes(),
    })
}

pub(crate) fn append_objective_outcome(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AppendObjectiveOutcome,
) -> Result<DomainRecordReceipt> {
    transaction::run(connection, |tx| apply(tx, input))
}

pub(crate) fn read_objective_outcome(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain: &str,
    outcome_id: &str,
    revision_value: &str,
) -> Result<ObjectiveOutcomeVersion> {
    identifier(domain)?;
    identifier(outcome_id)?;
    revision(revision_value)?;
    transaction::run(connection, |tx| {
        let rows=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),r.operation_id,r.content_hash,CAST(r.canonical_json AS TEXT),r.receipt_id,r.event_id,e.content_hash,CAST(e.canonical_json AS TEXT),r.object_type,r.object_id,r.object_version,r.receipt_type FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version JOIN main.gogoke_events e ON e.domain_id=r.domain_id AND e.event_id=r.event_id WHERE o.domain_id=? AND o.object_type='OutcomeRecord' AND o.object_id=? AND o.object_version=? AND r.receipt_type='ObjectiveOutcomeAppended'",&[domain,outcome_id,revision_value],13)?;
        if rows.len() != 1
            || content_hash(rows[0][1].as_bytes()) != rows[0][0]
            || content_hash(rows[0][4].as_bytes()) != rows[0][3]
            || content_hash(rows[0][8].as_bytes()) != rows[0][7]
            || rows[0][9] != "OutcomeRecord"
            || rows[0][10] != outcome_id
            || rows[0][11] != revision_value
            || rows[0][12] != "ObjectiveOutcomeAppended"
        {
            return denied();
        }
        if json_text(tx, &rows[0][4], "$.schema")? != "gogoke.objective-outcome.v1"
            || json_text(tx, &rows[0][4], "$.source")? != "PRODUCT_AUTHORITY"
        {
            return denied();
        }
        let body = &rows[0][1];
        if json_text(tx, body, "$.labelSource")? != "OBJECTIVE"
            || json_text(tx, body, "$.outcomeId")? != outcome_id
            || json_text(tx, body, "$.domainId")? != domain
            || json_text(tx, body, "$.revision")? != revision_value
        {
            return denied();
        }
        let check_hash = |tx: &mut Transaction<'_, '_>, path: &str| -> Result<String> {
            json_text(tx, body, path)
        };
        let manifest_id = json_text(tx, body, "$.manifestId")?;
        let manifest_version = json_text(tx, body, "$.manifestVersion")?;
        let manifest_hash = check_hash(tx, "$.manifestHash")?;
        let (manifest_record_hash, resolved_manifest_hash) =
            validate_manifest(tx, domain, &manifest_id, &manifest_version, &manifest_hash)?;
        if resolved_manifest_hash != manifest_hash
            || manifest_record_hash != json_text(tx, body, "$.manifestRecordHash")?
        {
            return denied();
        }
        let decision_id = json_text(tx, body, "$.decisionId")?;
        let decision_version = json_text(tx, body, "$.decisionRevision")?;
        let decision_hash = json_text(tx, body, "$.decisionHash")?;
        let op=tx.query("SELECT operation_id FROM main.gogoke_receipts WHERE domain_id=? AND object_type='DecisionRecord' AND object_id=? AND object_version=? AND receipt_type='DecisionApplied'",&[domain,&decision_id,&decision_version],1)?;
        if op.len() != 1 {
            return denied();
        }
        let decision = read_in_transaction(tx, domain, &op[0][0])?;
        if decision.decision_content_hash != decision_hash
            || decision.action_intent_ref != json_text(tx, body, "$.actionOperationId")?
        {
            return denied();
        }
        let action_id = json_text(tx, body, "$.actionOperationId")?;
        let completion_ref = json_text(tx, body, "$.actionCompletionRef")?;
        let completion_facts = completion(tx, domain, &action_id, &completion_ref)?;
        if completion_facts.0 != json_text(tx, body, "$.actionCompletionState")?
            || completion_facts.2 != json_text(tx, body, "$.actionSemanticDigest")?
        {
            return denied();
        }
        let intent=tx.query("SELECT package_operation_id,target_domain_id FROM main.gogoke_action_authority_intents WHERE domain_id=? AND operation_id=?",&[domain,&action_id],2)?;
        if intent.len() != 1 || intent[0][1] != domain {
            return denied();
        }
        let package = read_authorized_task_package_in_transaction(tx, domain, &intent[0][0])?
            .ok_or(OrchestrationError::AccessDenied)?;
        if package.target.domain_id != domain
            || package.source.project_id != json_text(tx, body, "$.projectId")?
        {
            return denied();
        }
        for path in ["$.resultRefs", "$.evidenceRefs"] {
            let refs=tx.query("SELECT json_extract(value,'$.objectType'),json_extract(value,'$.objectId'),json_extract(value,'$.objectVersion'),json_extract(value,'$.contentHash') FROM json_each(?,?) ORDER BY CAST(key AS INTEGER)",&[body,path],4)?;
            if refs.is_empty() || refs.len() > 64 {
                return denied();
            }
            for item in refs {
                let reference = ObjectiveEvidenceRef {
                    object_type: item[0].clone(),
                    object_id: item[1].clone(),
                    object_version: item[2].clone(),
                    content_hash: item[3].clone(),
                };
                validate_record_ref(tx, domain, &reference)?;
            }
        }
        let body_hash = rows[0][0].clone();
        Ok(ObjectiveOutcomeVersion {
            domain_id: domain.into(),
            outcome_id: outcome_id.into(),
            revision: revision_value.into(),
            content_hash: body_hash,
            canonical_outcome: body.as_bytes().to_vec(),
        })
    })
}
