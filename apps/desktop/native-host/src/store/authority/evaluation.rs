//! Durable Evaluation receipts on the existing Product Authority record store.
//! Evaluations consume exact Objective Outcome revisions; they never write
//! Action, Outcome, Adoption, Goal Acceptance, or Release facts.
use super::super::atomic::{DomainRecordInput, DomainRecordReceipt};
use super::super::digest::content_hash;
use super::super::orchestration::OrchestrationError;
use super::super::same_open::VerifiedDatabaseConnection;
use super::decision_replay::read_in_transaction;
use super::model::{denied, identifier, revision};
use super::transaction::{self, Result, Transaction};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationOutcomeRef {
    pub outcome_id: String,
    pub revision: String,
    pub content_hash: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationEvidenceRef {
    pub object_type: String,
    pub object_id: String,
    pub object_version: String,
    pub content_hash: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationVersionRef {
    pub revision: String,
    pub content_hash: String,
}

#[derive(Clone, Debug)]
pub(crate) struct AppendEvaluation {
    pub domain_id: String,
    pub evaluation_id: String,
    pub revision: String,
    pub operation_id: String,
    pub event_id: String,
    pub receipt_id: String,
    pub recorded_at: String,
    pub source_identity: String,
    pub outcome_refs: Vec<EvaluationOutcomeRef>,
    pub decision_family: String,
    pub scorer_version: String,
    pub rubric_version: String,
    pub calibration_key: String,
    pub calibration_version: String,
    pub dataset_namespace: String,
    pub dataset_split: String,
    pub evidence_refs: Vec<EvaluationEvidenceRef>,
    pub metrics_hash: String,
    pub safety_status: String,
    pub privacy_status: String,
    pub previous: Option<EvaluationVersionRef>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvaluationReceipt {
    pub domain_id: String,
    pub evaluation_id: String,
    pub revision: String,
    pub content_hash: String,
    pub canonical_evaluation: Vec<u8>,
    pub decision_family: String,
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
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn object(mut fields: Vec<(String, String)>) -> String {
    fields.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    let entries = fields
        .into_iter()
        .map(|(k, v)| format!("{}:{v}", quote(&k)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{{entries}}}")
}
fn refs_json(values: &[EvaluationEvidenceRef]) -> Result<String> {
    if values.is_empty() || values.len() > 64 {
        return denied();
    }
    let mut refs = values.to_vec();
    for r in &refs {
        if !matches!(
            r.object_type.as_str(),
            "ActionCompletion" | "ContextManifest" | "DecisionRecord" | "ResultRecord"
        ) {
            return denied();
        }
        identifier(&r.object_id)?;
        revision(&r.object_version)?;
        hash(&r.content_hash)?;
    }
    refs.sort_by(|a, b| {
        (&a.object_type, &a.object_id, &a.object_version).cmp(&(
            &b.object_type,
            &b.object_id,
            &b.object_version,
        ))
    });
    if refs.windows(2).any(|p| {
        p[0].object_type == p[1].object_type
            && p[0].object_id == p[1].object_id
            && p[0].object_version == p[1].object_version
    }) {
        return denied();
    }
    Ok(format!(
        "[{}]",
        refs.iter()
            .map(|r| object(vec![
                ("contentHash".into(), quote(&r.content_hash)),
                ("objectId".into(), quote(&r.object_id)),
                ("objectType".into(), quote(&r.object_type)),
                ("objectVersion".into(), quote(&r.object_version))
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
fn validate_evidence(
    tx: &mut Transaction<'_, '_>,
    domain: &str,
    r: &EvaluationEvidenceRef,
) -> Result<()> {
    let rows=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),r.receipt_type,r.content_hash,CAST(r.canonical_json AS TEXT),r.operation_id FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version WHERE o.domain_id=? AND o.object_type=? AND o.object_id=? AND o.object_version=?",&[domain,&r.object_type,&r.object_id,&r.object_version],6)?;
    if rows.len() != 1
        || rows[0][0] != r.content_hash
        || content_hash(rows[0][1].as_bytes()) != rows[0][0]
        || content_hash(rows[0][4].as_bytes()) != rows[0][3]
    {
        return denied();
    }
    let expected = match r.object_type.as_str() {
        "ActionCompletion" => "ActionCompletionRecorded",
        "ContextManifest" => "ContextManifestCommitted",
        "DecisionRecord" => "DecisionApplied",
        "ResultRecord" => "TrustedResultRecorded",
        _ => return denied(),
    };
    if rows[0][2] != expected {
        return denied();
    }
    if r.object_type == "DecisionRecord" {
        let d = read_in_transaction(tx, domain, &rows[0][5])?;
        if d.decision_id != r.object_id
            || d.object_version != r.object_version
            || d.decision_content_hash != r.content_hash
        {
            return denied();
        }
    }
    Ok(())
}
fn validate_outcomes(
    tx: &mut Transaction<'_, '_>,
    domain: &str,
    refs: &[EvaluationOutcomeRef],
    family: &str,
) -> Result<()> {
    if refs.is_empty() || refs.len() > 64 {
        return denied();
    }
    let mut sorted = refs.to_vec();
    for r in &sorted {
        identifier(&r.outcome_id)?;
        revision(&r.revision)?;
        hash(&r.content_hash)?;
    }
    sorted.sort_by(|a, b| (&a.outcome_id, &a.revision).cmp(&(&b.outcome_id, &b.revision)));
    if sorted
        .windows(2)
        .any(|p| p[0].outcome_id == p[1].outcome_id && p[0].revision == p[1].revision)
    {
        return denied();
    }
    let mut resolved_family: Option<String> = None;
    for r in &sorted {
        let rows=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),rec.operation_id,rec.receipt_type,rec.content_hash,CAST(rec.canonical_json AS TEXT),e.content_hash,CAST(e.canonical_json AS TEXT),e.event_type FROM main.gogoke_objects o JOIN main.gogoke_receipts rec ON rec.domain_id=o.domain_id AND rec.object_type=o.object_type AND rec.object_id=o.object_id AND rec.object_version=o.object_version JOIN main.gogoke_events e ON e.domain_id=rec.domain_id AND e.event_id=rec.event_id WHERE o.domain_id=? AND o.object_type='OutcomeRecord' AND o.object_id=? AND o.object_version=?",&[domain,&r.outcome_id,&r.revision],9)?;
        if rows.len() != 1
            || rows[0][0] != r.content_hash
            || content_hash(rows[0][1].as_bytes()) != rows[0][0]
            || rows[0][3] != "ObjectiveOutcomeAppended"
            || content_hash(rows[0][5].as_bytes()) != rows[0][4]
            || content_hash(rows[0][7].as_bytes()) != rows[0][6]
            || rows[0][8] != "ObjectiveOutcomeAppended"
            || json_text(tx, &rows[0][1], "$.labelSource")? != "OBJECTIVE"
            || json_text(tx, &rows[0][1], "$.outcomeId")? != r.outcome_id
            || json_text(tx, &rows[0][1], "$.revision")? != r.revision
            || json_text(tx, &rows[0][1], "$.domainId")? != domain
            || json_text(tx, &rows[0][5], "$.schema")? != "gogoke.objective-outcome.v1"
            || json_text(tx, &rows[0][7], "$.contentHash")? != r.content_hash
            || json_text(tx, &rows[0][7], "$.outcomeId")? != r.outcome_id
            || json_text(tx, &rows[0][7], "$.revision")? != r.revision
        {
            return denied();
        }
        let decision_id = json_text(tx, &rows[0][1], "$.decisionId")?;
        let decision_revision = json_text(tx, &rows[0][1], "$.decisionRevision")?;
        let decision_hash = json_text(tx, &rows[0][1], "$.decisionHash")?;
        let decision=tx.query("SELECT operation_id FROM main.gogoke_receipts WHERE domain_id=? AND object_type='DecisionRecord' AND object_id=? AND object_version=? AND receipt_type='DecisionApplied'",&[domain,&decision_id,&decision_revision],1)?;
        if decision.len() != 1 {
            return denied();
        }
        let d = read_in_transaction(tx, domain, &decision[0][0])?;
        if d.decision_content_hash != decision_hash {
            return denied();
        }
        match &resolved_family {
            None => resolved_family = Some(d.record.family),
            Some(current) if current == &d.record.family => {}
            _ => return denied(),
        }
    }
    if resolved_family.as_deref() != Some(family) {
        return denied();
    }
    Ok(())
}
fn validate(input: &AppendEvaluation) -> Result<()> {
    for value in [
        &input.domain_id,
        &input.evaluation_id,
        &input.operation_id,
        &input.event_id,
        &input.receipt_id,
        &input.source_identity,
        &input.decision_family,
        &input.scorer_version,
        &input.rubric_version,
        &input.calibration_key,
        &input.calibration_version,
        &input.dataset_namespace,
        &input.dataset_split,
    ] {
        identifier(value)?;
    }
    if revision(&input.revision)? == 0 {
        return denied();
    }
    hash(&input.metrics_hash)?;
    if !matches!(
        input.safety_status.as_str(),
        "CLEAR" | "REVIEW_REQUIRED" | "BLOCKED"
    ) || !matches!(
        input.privacy_status.as_str(),
        "CLEAR" | "REDACTED" | "RESTRICTED"
    ) {
        return denied();
    }
    Ok(())
}
fn body(tx: &mut Transaction<'_, '_>, input: &AppendEvaluation) -> Result<Vec<u8>> {
    validate(input)?;
    validate_outcomes(
        tx,
        &input.domain_id,
        &input.outcome_refs,
        &input.decision_family,
    )?;
    let evidence = refs_json(&input.evidence_refs)?;
    for r in &input.evidence_refs {
        validate_evidence(tx, &input.domain_id, r)?;
    }
    let mut outcomes = input.outcome_refs.clone();
    outcomes.sort_by(|a, b| (&a.outcome_id, &a.revision).cmp(&(&b.outcome_id, &b.revision)));
    let outcome_json = format!(
        "[{}]",
        outcomes
            .iter()
            .map(|r| object(vec![
                ("contentHash".into(), quote(&r.content_hash)),
                ("outcomeId".into(), quote(&r.outcome_id)),
                ("revision".into(), quote(&r.revision))
            ]))
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok(object(vec![
        ("calibrationKey".into(), quote(&input.calibration_key)),
        (
            "calibrationVersion".into(),
            quote(&input.calibration_version),
        ),
        ("datasetNamespace".into(), quote(&input.dataset_namespace)),
        ("datasetSplit".into(), quote(&input.dataset_split)),
        ("decisionFamily".into(), quote(&input.decision_family)),
        ("evaluationId".into(), quote(&input.evaluation_id)),
        ("evidenceRefs".into(), evidence),
        ("labelSource".into(), quote("DURABLE_EVALUATION")),
        ("metricsHash".into(), quote(&input.metrics_hash)),
        ("outcomeRefs".into(), outcome_json),
        ("privacyStatus".into(), quote(&input.privacy_status)),
        ("revision".into(), quote(&input.revision)),
        ("rubricVersion".into(), quote(&input.rubric_version)),
        ("safetyStatus".into(), quote(&input.safety_status)),
        ("scorerVersion".into(), quote(&input.scorer_version)),
        ("sourceIdentity".into(), quote(&input.source_identity)),
        ("domainId".into(), quote(&input.domain_id)),
    ])
    .into_bytes())
}
fn append(tx: &mut Transaction<'_, '_>, input: &AppendEvaluation) -> Result<DomainRecordReceipt> {
    let bytes = body(tx, input)?;
    let replay=tx.query("SELECT receipt_id,event_id,object_type,object_id,object_version FROM main.gogoke_receipts WHERE domain_id=? AND operation_id=?",&[&input.domain_id,&input.operation_id],5)?;
    if !replay.is_empty() {
        if replay.len() != 1
            || replay[0][0] != input.receipt_id
            || replay[0][1] != input.event_id
            || replay[0][2] != "EvaluationRecord"
            || replay[0][3] != input.evaluation_id
            || replay[0][4] != input.revision
        {
            return Err(OrchestrationError::OperationConflict);
        }
        let existing=tx.query("SELECT CAST(canonical_json AS TEXT) FROM main.gogoke_objects WHERE domain_id=? AND object_type='EvaluationRecord' AND object_id=? AND object_version=?",&[&input.domain_id,&input.evaluation_id,&input.revision],1)?;
        if existing.len() != 1 || existing[0][0].as_bytes() != bytes.as_slice() {
            return Err(OrchestrationError::OperationConflict);
        }
        return record(tx, input, bytes);
    }
    let head=tx.query("SELECT object_version,content_hash FROM main.gogoke_objects WHERE domain_id=? AND object_type='EvaluationRecord' AND object_id=? ORDER BY length(object_version) DESC,object_version DESC LIMIT 1",&[&input.domain_id,&input.evaluation_id],2)?;
    match (&input.previous, head.first()) {
        (None, None) if input.revision == "1" => {}
        (Some(p), Some(h))
            if revision(&p.revision)?.checked_add(1) == Some(revision(&input.revision)?)
                && h[0] == p.revision
                && h[1] == p.content_hash =>
        {
            hash(&p.content_hash)?
        }
        _ => return Err(OrchestrationError::OperationConflict),
    }
    record(tx, input, bytes)
}
fn record(
    tx: &mut Transaction<'_, '_>,
    input: &AppendEvaluation,
    bytes: Vec<u8>,
) -> Result<DomainRecordReceipt> {
    let previous = input
        .previous
        .as_ref()
        .map(|p| quote(&p.content_hash))
        .unwrap_or_else(|| "null".into());
    let event = object(vec![
        ("contentHash".into(), quote(&content_hash(&bytes))),
        ("evaluationId".into(), quote(&input.evaluation_id)),
        ("previousHash".into(), previous),
        ("revision".into(), quote(&input.revision)),
    ]);
    let receipt = object(vec![
        ("schema".into(), quote("gogoke.evaluation.v1")),
        ("sourceIdentity".into(), quote(&input.source_identity)),
    ]);
    tx.apply_domain_record(DomainRecordInput {
        domain_id: input.domain_id.clone(),
        object_type: "EvaluationRecord".into(),
        object_id: input.evaluation_id.clone(),
        object_version: input.revision.clone(),
        object_bytes: bytes,
        native_identity: None,
        event_id: input.event_id.clone(),
        stream_id: format!("gogoke.evaluation.v1/{}", input.evaluation_id),
        expected_previous_counter: revision(&input.revision)?
            .checked_sub(2)
            .map(|n| n.to_string()),
        counter: (revision(&input.revision)? - 1).to_string(),
        event_type: "EvaluationRecorded".into(),
        occurred_at: input.recorded_at.clone(),
        event_bytes: event.into_bytes(),
        receipt_id: input.receipt_id.clone(),
        operation_id: input.operation_id.clone(),
        receipt_type: "EvaluationRecorded".into(),
        recorded_at: input.recorded_at.clone(),
        receipt_bytes: receipt.into_bytes(),
    })
}
pub(crate) fn append_evaluation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: &AppendEvaluation,
) -> Result<DomainRecordReceipt> {
    validate(input)?;
    transaction::run(connection, |tx| append(tx, input))
}
pub(crate) fn read_evaluation(
    connection: &mut VerifiedDatabaseConnection<'_>,
    domain: &str,
    id: &str,
    version: &str,
) -> Result<EvaluationReceipt> {
    identifier(domain)?;
    identifier(id)?;
    revision(version)?;
    transaction::run(connection, |tx| {
        let rows=tx.query("SELECT o.content_hash,CAST(o.canonical_json AS TEXT),r.content_hash,CAST(r.canonical_json AS TEXT),r.event_id,e.content_hash,CAST(e.canonical_json AS TEXT),r.object_type,r.object_id,r.object_version,r.receipt_type FROM main.gogoke_objects o JOIN main.gogoke_receipts r ON r.domain_id=o.domain_id AND r.object_type=o.object_type AND r.object_id=o.object_id AND r.object_version=o.object_version JOIN main.gogoke_events e ON e.domain_id=r.domain_id AND e.event_id=r.event_id WHERE o.domain_id=? AND o.object_type='EvaluationRecord' AND o.object_id=? AND o.object_version=? AND r.receipt_type='EvaluationRecorded'",&[domain,id,version],11)?;
        if rows.len() != 1
            || content_hash(rows[0][1].as_bytes()) != rows[0][0]
            || content_hash(rows[0][3].as_bytes()) != rows[0][2]
            || content_hash(rows[0][6].as_bytes()) != rows[0][5]
            || rows[0][7] != "EvaluationRecord"
            || rows[0][8] != id
            || rows[0][9] != version
            || rows[0][10] != "EvaluationRecorded"
        {
            return denied();
        }
        if json_text(tx, &rows[0][1], "$.labelSource")? != "DURABLE_EVALUATION"
            || json_text(tx, &rows[0][1], "$.evaluationId")? != id
            || json_text(tx, &rows[0][1], "$.revision")? != version
            || json_text(tx, &rows[0][3], "$.schema")? != "gogoke.evaluation.v1"
            || json_text(tx, &rows[0][3], "$.sourceIdentity")?
                != json_text(tx, &rows[0][1], "$.sourceIdentity")?
            || json_text(tx, &rows[0][6], "$.contentHash")? != rows[0][0]
            || json_text(tx, &rows[0][6], "$.evaluationId")? != id
            || json_text(tx, &rows[0][6], "$.revision")? != version
        {
            return denied();
        }
        let family = json_text(tx, &rows[0][1], "$.decisionFamily")?;
        for path in [
            "$.sourceIdentity",
            "$.scorerVersion",
            "$.rubricVersion",
            "$.calibrationKey",
            "$.calibrationVersion",
            "$.datasetNamespace",
            "$.datasetSplit",
        ] {
            identifier(&json_text(tx, &rows[0][1], path)?)?;
        }
        if !matches!(
            json_text(tx, &rows[0][1], "$.safetyStatus")?.as_str(),
            "CLEAR" | "REVIEW_REQUIRED" | "BLOCKED"
        ) || !matches!(
            json_text(tx, &rows[0][1], "$.privacyStatus")?.as_str(),
            "CLEAR" | "REDACTED" | "RESTRICTED"
        ) {
            return denied();
        }
        // Reparse references and rebind every source before returning a durable Evaluation receipt.
        let outcome_rows=tx.query("SELECT json_extract(value,'$.outcomeId'),json_extract(value,'$.revision'),json_extract(value,'$.contentHash') FROM json_each(?,'$.outcomeRefs') ORDER BY CAST(key AS INTEGER)",&[&rows[0][1]],3)?;
        let outcomes = outcome_rows
            .iter()
            .map(|r| EvaluationOutcomeRef {
                outcome_id: r[0].clone(),
                revision: r[1].clone(),
                content_hash: r[2].clone(),
            })
            .collect::<Vec<_>>();
        validate_outcomes(tx, domain, &outcomes, &family)?;
        let evidence_rows=tx.query("SELECT json_extract(value,'$.objectType'),json_extract(value,'$.objectId'),json_extract(value,'$.objectVersion'),json_extract(value,'$.contentHash') FROM json_each(?,'$.evidenceRefs') ORDER BY CAST(key AS INTEGER)",&[&rows[0][1]],4)?;
        let evidence = evidence_rows
            .iter()
            .map(|r| EvaluationEvidenceRef {
                object_type: r[0].clone(),
                object_id: r[1].clone(),
                object_version: r[2].clone(),
                content_hash: r[3].clone(),
            })
            .collect::<Vec<_>>();
        for r in &evidence {
            validate_evidence(tx, domain, r)?;
        }
        let metrics_hash = json_text(tx, &rows[0][1], "$.metricsHash")?;
        hash(&metrics_hash)?;
        Ok(EvaluationReceipt {
            domain_id: domain.into(),
            evaluation_id: id.into(),
            revision: version.into(),
            content_hash: rows[0][0].clone(),
            canonical_evaluation: rows[0][1].as_bytes().to_vec(),
            decision_family: family,
        })
    })
}
