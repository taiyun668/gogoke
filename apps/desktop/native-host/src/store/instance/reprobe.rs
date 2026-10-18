//! F.2 capability re-probe readback. The existing K-SESSION receipt is the
//! durable source; F never invents a capability cache or launches a model.

use super::*;
use crate::store::atomic::{Json, JsonString};
use crate::store::digest::sha256_hex;
use crate::store::session_transport::{decode_receipt, decode_request, V37Status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CapabilityReprobeEvidence {
    pub(crate) instance_id: String,
    pub(crate) version: String,
    pub(crate) program_digest: String,
    pub(crate) domain_id: String,
    pub(crate) session_id: String,
    pub(crate) generation: String,
    pub(crate) process_operation_id: String,
    pub(crate) request_id: String,
    pub(crate) receipt_sha256: String,
}

fn text(fields: &std::collections::BTreeMap<JsonString, Json>, name: &str)
    -> Result<String, OrchestrationError> {
    match fields.get(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string()
            .ok_or(OrchestrationError::AccessDenied),
        _ => Err(OrchestrationError::AccessDenied),
    }
}

/// After manual repin the prior CLI digest is no longer eligible. The caller
/// must run H's actual capability-probe and pass that original request ID;
/// this reader binds its stored receipt to the current instance, claim and
/// process custody. A missing/new-source receipt is None, never a PASS.
pub(crate) fn read_current_capability_reprobe(
    db: &VerifiedDatabaseConnection<'_>, instance_id: &str,
    domain_id: &str, request_id: &str,
) -> Result<Option<CapabilityReprobeEvidence>, OrchestrationError> {
    if [instance_id,domain_id,request_id].iter().any(|value| value.is_empty() || value.len()>96
        || !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte==b'-' || byte==b'_')) {
        return Err(OrchestrationError::AccessDenied);
    }
    let record = Statement::prepare(db.as_ptr(),
        "SELECT request_bytes,receipt_bytes FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
    record.bind_text(1,domain_id)?; record.bind_text(2,request_id)?;
    if !record.step_row()? { return Ok(None); }
    let raw_request = record.column_text(0)?.into_bytes();
    let raw_receipt = record.column_text(1)?.into_bytes();
    if record.step_row()? { return Err(OrchestrationError::AccessDenied); }
    let request = decode_request(&raw_request)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!("F.2 capability request: {error:?}")))?;
    let receipt = decode_receipt(&raw_receipt)
        .map_err(|error| OrchestrationError::V37StoreFailure(format!("F.2 capability receipt: {error:?}")))?;
    if request.family!="K-SESSION" || request.operation!="capability-probe" ||
        request.domain_id!=domain_id || request.request_id!=request_id ||
        receipt.family!=request.family || receipt.operation!=request.operation ||
        receipt.request_id!=request_id || receipt.target_id!=request.target_id ||
        receipt.status!=V37Status::Applied { return Err(OrchestrationError::AccessDenied); }
    let result=receipt.into_result();
    let generation=text(&result,"generation")?.to_owned();
    let operation=text(&result,"processOperationId")?.to_owned();
    let digest=text(&result,"binaryDigest")?.to_owned();
    let version=text(&result,"version")?.to_owned();
    let request_generation=match request.payload.get(&JsonString::from_str("generation")) {
        Some(Json::String(value))=>value.to_well_formed_string()
            .ok_or(OrchestrationError::AccessDenied)?,
        _=>return Err(OrchestrationError::AccessDenied),
    };
    if request_generation!=generation || text(&result,"driverId")? != "codex" ||
        text(&result,"evidenceBasis")? != "NATIVE_LOADED_THREAD_FEATURE_RESPONSE" ||
        text(&result,"modelBehaviour")? != "NOT_RUN" { return Err(OrchestrationError::AccessDenied); }
    let flags=match result.get(&JsonString::from_str("loadedThreadFeatures")) {
        Some(Json::Object(fields))=>fields,
        _=>return Err(OrchestrationError::AccessDenied),
    };
    for (name, expected) in [("memories",false),("multi_agent_v2",false),
        ("default_mode_request_user_input",true)] {
        if !matches!(flags.get(&JsonString::from_str(name)),Some(Json::Bool(actual)) if *actual==expected) {
            return Err(OrchestrationError::AccessDenied);
        }
    }
    let instance=Statement::prepare(db.as_ptr(),
        "SELECT version,program_digest FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
    instance.bind_text(1,instance_id)?;
    if !instance.step_row()? || instance.column_text(0)?!=version || instance.column_text(1)?!=digest ||
        instance.step_row()? { return Err(OrchestrationError::AccessDenied); }
    let claim=Statement::prepare(db.as_ptr(),
        "SELECT a.instance_id,a.generation,a.process_operation_id,COALESCE(a.stop_fact_id,''),COALESCE(c.state,''),COALESCE(c.stop_proof_hash,''),COALESCE(c.binary_digest_sha256,'') FROM main.gogoke_v37_h_claim a LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id AND c.generation=a.generation WHERE a.domain_id=?1 AND a.session_id=?2")?;
    claim.bind_text(1,domain_id)?; claim.bind_text(2,&request.target_id)?;
    if !claim.step_row()? || claim.column_text(0)?!=instance_id ||
        claim.column_text(1)?!=generation || claim.column_text(2)?!=operation ||
        claim.column_text(6)?!=digest { return Err(OrchestrationError::AccessDenied); }
    let stop_fact=claim.column_text(3)?;
    let custody=claim.column_text(4)?;
    let proof=claim.column_text(5)?;
    if !(custody=="ACTIVE" || (custody=="STOPPED" && !stop_fact.is_empty() && proof==stop_fact)) ||
        claim.step_row()? { return Err(OrchestrationError::AccessDenied); }
    Ok(Some(CapabilityReprobeEvidence { instance_id:instance_id.to_owned(), version,
        program_digest:digest, domain_id:domain_id.to_owned(), session_id:request.target_id,
        generation, process_operation_id:operation, request_id:request_id.to_owned(),
        receipt_sha256:sha256_hex(&raw_receipt) }))
}
