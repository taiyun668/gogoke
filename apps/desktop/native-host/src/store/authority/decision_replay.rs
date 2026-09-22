//! Durable Decision replay reader on the existing canonical record/receipt store.
//! It reconstructs an earlier service Decision from the private DecisionApplied
//! receipt and cross-checks the public DecisionRecord. It does NOT establish
//! current eligibility, capacity, binding legality, or permission to dispatch.
use super::model::{denied, identifier, revision};
use super::transaction::{self, Result, Transaction};
use super::super::digest::content_hash;
use super::super::same_open::VerifiedDatabaseConnection;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DurableDecisionRecord {
    pub operation_id: String,
    pub scenario_id: String,
    pub family: String,
    pub state_view_hash: String,
    pub candidate_hash: String,
    pub question_version: String,
    pub rubric_version: String,
    pub model_requested: Option<String>,
    pub model_resolved: Option<String>,
    pub task_revision: String,
    pub policy_revision: String,
    pub capability_revision: String,
    pub binding_generation: String,
    pub backend_kind: String,
    pub choice: String,
    pub reason: String,
    pub budget_units: u64,
    pub deadline_epoch_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DurableDecisionReplay {
    pub decision_id: String,
    pub object_version: String,
    pub decision_content_hash: String,
    pub receipt_id: String,
    pub operation_fingerprint: String,
    pub resource_reservation_ref: String,
    pub action_intent_ref: String,
    pub record: DurableDecisionRecord,
}

fn nonempty(value: String) -> Result<String> {
    if value.is_empty() { return denied(); }
    Ok(value)
}
fn hash(value: &str) -> Result<()> {
    if value.len()!=71 || !value.starts_with("sha256:")
        || !value.as_bytes()[7..].iter().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(b)) {
        return denied();
    }
    Ok(())
}
fn json_type(tx:&mut Transaction<'_, '_>, json:&str, path:&str)->Result<String>{
    let rows=tx.query("SELECT COALESCE(json_type(?,?),'missing')",&[json,path],1)?;
    if rows.len()!=1{return denied();} Ok(rows[0][0].clone())
}
fn json_text(tx:&mut Transaction<'_, '_>, json:&str, path:&str)->Result<String>{
    if json_type(tx,json,path)?!="text"{return denied();}
    let rows=tx.query("SELECT json_extract(?,?)",&[json,path],1)?;
    if rows.len()!=1{return denied();} nonempty(rows[0][0].clone())
}
fn json_nullable_text(tx:&mut Transaction<'_, '_>, json:&str, path:&str)->Result<Option<String>>{
    match json_type(tx,json,path)?.as_str(){
        "null"=>Ok(None),
        "text"=>Ok(Some(json_text(tx,json,path)?)),
        _=>denied(),
    }
}
fn json_u64_decimal_string(tx:&mut Transaction<'_, '_>, json:&str, path:&str)->Result<u64>{
    let encoded=json_text(tx,json,path)?;
    let value=revision(&encoded)?;
    if value>9_007_199_254_740_991{return denied();}
    Ok(value)
}
fn keys(tx:&mut Transaction<'_, '_>, json:&str, path:Option<&str>)->Result<Vec<String>>{
    let rows=match path{
        None=>tx.query("SELECT key FROM json_each(?)",&[json],1)?,
        Some(path)=>tx.query("SELECT key FROM json_each(?,?)",&[json,path],1)?,
    };
    Ok(rows.into_iter().map(|row|row[0].clone()).collect())
}
fn require_exact_keys(actual:Vec<String>, expected:&[&str])->Result<()>{
    if actual.len()!=expected.len() || expected.iter().any(|key|!actual.iter().any(|actual|actual==key)){
        return denied();
    }
    Ok(())
}
pub(super) fn validate_public_decision(tx:&mut Transaction<'_, '_>, json:&str)->Result<()>{
    if json_type(tx,json,"$")?!="object"{return denied();}
    const REQUIRED:[&str;15]=["decisionId","family","stateViewHash","candidateHash","sourceRevisions",
        "backend","modelRequested","modelResolved","questionVersion","probabilities","nativeConfidence",
        "calibrationRef","mode","state","actionId"];
    let actual=keys(tx,json,None)?;
    if REQUIRED.iter().any(|key|!actual.iter().any(|actual|actual==key)){return denied();}
    if json_type(tx,json,"$.sourceRevisions")?!="object" || json_type(tx,json,"$.probabilities")?!="object"{
        return denied();
    }
    for field in ["decisionId","family","stateViewHash","candidateHash","backend","modelRequested",
        "modelResolved","questionVersion","calibrationRef","mode","state","actionId"]{
        json_text(tx,json,&format!("$.{field}"))?;
    }
    if json_text(tx,json,"$.state")?!="COMMITTED"{return denied();}
    Ok(())
}
fn source_revision(tx:&mut Transaction<'_, '_>, public:&str, key:&str)->Result<String>{
    let path=format!("$.sourceRevisions.{key}");
    let value=json_text(tx,public,&path)?;
    revision(&value)?;
    Ok(value)
}
fn internal_record(tx:&mut Transaction<'_, '_>, receipt:&str)->Result<DurableDecisionRecord>{
    const FIELDS:[&str;19]=["operationId","scenarioId","family","state","stateViewHash","candidateHash",
        "questionVersion","rubricVersion","modelRequested","modelResolved","taskRevision","policyRevision",
        "capabilityRevision","bindingGeneration","backendKind","choice","reason","budgetUnits","deadlineEpochMs"];
    require_exact_keys(keys(tx,receipt,Some("$.engineRecord"))?,&FIELDS)?;
    if json_text(tx,receipt,"$.engineRecord.state")?!="COMMITTED"{return denied();}
    let backend=json_text(tx,receipt,"$.engineRecord.backendKind")?;
    if !matches!(backend.as_str(),"RULES"|"FAKE"|"REPLAY"|"JEV"|"GENERATIVE"){return denied();}
    let choice=json_text(tx,receipt,"$.engineRecord.choice")?;
    let task_revision=json_text(tx,receipt,"$.engineRecord.taskRevision")?; revision(&task_revision)?;
    let policy_revision=json_text(tx,receipt,"$.engineRecord.policyRevision")?; revision(&policy_revision)?;
    let capability_revision=json_text(tx,receipt,"$.engineRecord.capabilityRevision")?; revision(&capability_revision)?;
    let binding_generation=json_text(tx,receipt,"$.engineRecord.bindingGeneration")?; revision(&binding_generation)?;
    Ok(DurableDecisionRecord{
        operation_id:json_text(tx,receipt,"$.engineRecord.operationId")?,
        scenario_id:json_text(tx,receipt,"$.engineRecord.scenarioId")?,
        family:json_text(tx,receipt,"$.engineRecord.family")?,
        state_view_hash:json_text(tx,receipt,"$.engineRecord.stateViewHash")?,
        candidate_hash:json_text(tx,receipt,"$.engineRecord.candidateHash")?,
        question_version:json_text(tx,receipt,"$.engineRecord.questionVersion")?,
        rubric_version:json_text(tx,receipt,"$.engineRecord.rubricVersion")?,
        model_requested:json_nullable_text(tx,receipt,"$.engineRecord.modelRequested")?,
        model_resolved:json_nullable_text(tx,receipt,"$.engineRecord.modelResolved")?,
        task_revision,policy_revision,capability_revision,binding_generation,
        backend_kind:backend,choice,
        reason:json_text(tx,receipt,"$.engineRecord.reason")?,
        budget_units:json_u64_decimal_string(tx,receipt,"$.engineRecord.budgetUnits")?,
        deadline_epoch_ms:json_u64_decimal_string(tx,receipt,"$.engineRecord.deadlineEpochMs")?,
    })
}

pub(super) fn read_in_transaction(tx:&mut Transaction<'_, '_>,domain:&str,operation:&str)->Result<DurableDecisionReplay>{
    identifier(domain)?; identifier(operation)?;
    let rows=tx.query(
        "SELECT r.receipt_id,r.object_id,r.object_version,r.operation_fingerprint,r.content_hash,CAST(r.canonical_json AS TEXT),o.content_hash,CAST(o.canonical_json AS TEXT) FROM gogoke_receipts r JOIN gogoke_objects o ON o.domain_id=r.domain_id AND o.object_type=r.object_type AND o.object_id=r.object_id AND o.object_version=r.object_version WHERE r.domain_id=? AND r.operation_id=? AND r.object_type='DecisionRecord' AND r.receipt_type='DecisionApplied'",
        &[domain,operation],8)?;
    if rows.len()!=1{return denied();}
    let row=&rows[0]; hash(&row[3])?; hash(&row[4])?; hash(&row[6])?;
    if content_hash(row[5].as_bytes())!=row[4] || content_hash(row[7].as_bytes())!=row[6]{return denied();}
    let receipt=&row[5]; let public=&row[7];
    if json_type(tx,receipt,"$")?!="object"{return denied();}
    const RECEIPT:[&str;6]=["actionIntentRef","candidateId","decisionContentHash","engineRecord",
        "resourceReservationRef","schema"];
    require_exact_keys(keys(tx,receipt,None)?,&RECEIPT)?;
    if json_text(tx,receipt,"$.schema")?!="gogoke.decision-commit-receipt.v1"{return denied();}
    let record=internal_record(tx,receipt)?;
    if record.operation_id!=operation{return denied();}
    let candidate=json_text(tx,receipt,"$.candidateId")?;
    let resource=json_text(tx,receipt,"$.resourceReservationRef")?;
    let action=json_text(tx,receipt,"$.actionIntentRef")?;
    if record.choice!=candidate{return denied();}
    let decision_hash=json_text(tx,receipt,"$.decisionContentHash")?; hash(&decision_hash)?;
    if decision_hash!=row[6]{return denied();}

    validate_public_decision(tx,public)?;
    let decision_id=json_text(tx,public,"$.decisionId")?;
    if decision_id!=row[1] || json_text(tx,public,"$.actionId")?!=action
        || json_text(tx,public,"$.family")?!=record.family
        || json_text(tx,public,"$.stateViewHash")?!=record.state_view_hash
        || json_text(tx,public,"$.candidateHash")?!=record.candidate_hash
        || json_text(tx,public,"$.questionVersion")?!=record.question_version
        || json_text(tx,public,"$.backend")?!=record.backend_kind
        || source_revision(tx,public,"taskRevision")?!=record.task_revision
        || source_revision(tx,public,"policyRevision")?!=record.policy_revision
        || source_revision(tx,public,"capabilityRevision")?!=record.capability_revision
        || source_revision(tx,public,"bindingGeneration")?!=record.binding_generation {
        return denied();
    }
    Ok(DurableDecisionReplay{decision_id,object_version:row[2].clone(),decision_content_hash:row[6].clone(),
        receipt_id:row[0].clone(),operation_fingerprint:row[3].clone(),resource_reservation_ref:resource,
        action_intent_ref:action,record})
}

/// Read-only durable history. Current legality/capacity must be checked separately
/// by the authoritative commit path before any new side effect.
pub(crate) fn read_durable_decision_replay(
    connection:&mut VerifiedDatabaseConnection<'_>,domain:&str,operation:&str,
)->Result<DurableDecisionReplay>{
    transaction::run(connection,|tx|read_in_transaction(tx,domain,operation))
}
