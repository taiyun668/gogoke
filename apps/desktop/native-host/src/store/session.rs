//! Line-delimited typed operations on the same-open connection.
//! Node does not receive SQL, a path capability, or a SQLite handle.

use super::atomic::Statement;
use super::action::apply_action_schema;
use super::context::{
    apply_context_schema, commit_context_version, ContextCommand, PromotionEvidence,
};
use super::orchestration::{
    apply_orchestration_slice_schema, commit_orchestration, read_events, read_snapshot_projects,
    record_rejected_command, EventProposal, OrchestrationCommand, OrchestrationError,
};
use super::protocol::{decode_flat_string_object, decode_operation_frame, ProtocolError};
use super::authority::{
    self, ContextAssemblySnapshot, ContextManifestCommitInput, ContextManifestReplayIdentity,
    ContextReadRequest, DecisionAuthoritySnapshot, DecisionCommitDisposition, DecisionCommitInput,
    DurableDecisionRecord, GrantRef, GranteeContextReadRequest, ManifestExpectedVersion,
    CommitTaskContextRequirements, MandatoryContextRef, OwnerIssuer,
    AppendExecutionRecipe, AppendObjectiveOutcome, ObjectiveEvidenceRef,
    ObjectiveObservationWindow, ObjectiveVersionRef, AppendEvaluation,
    EvaluationOutcomeRef, EvaluationEvidenceRef, EvaluationVersionRef,
    AppendDreamRun, AppendDreamProposal, DreamObjectRef, DreamEvaluationRef,
    DreamBudgetLease, DreamAllowedChange, DreamVersionRef, RecipeJsonObject,
    RecipeJsonString, RecipeJsonValue,
    SessionLineageCommand, SessionLineageOperation, NativeSessionIdentity, PendingActionRef,
};
use super::atomic::{Json as NativeJson, JsonString as NativeJsonString, Parser as NativeJsonParser};
use super::same_open::{create_new, open_existing, VerifiedDatabaseConnection};
use crate::ipc::PrivatePipeConnection;
use crate::root::RootLock;
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

pub fn open_product_database<'root>(
    root: &'root RootLock,
    database: &Path,
) -> Result<VerifiedDatabaseConnection<'root>, OrchestrationError> {
    let mut connection = if database.exists() {
        open_existing(root, database).map_err(|error| OrchestrationError::Atomic(error.into()))?
    } else {
        create_new(root, database).map_err(|error| OrchestrationError::Atomic(error.into()))?
    };
    apply_orchestration_slice_schema(&mut connection)?;
    apply_context_schema(&mut connection)?;
    apply_action_schema(&mut connection)?;
    let _owner_issuer = super::authority::initialize_profile(&mut connection, root)?;
    Ok(connection)
}

pub fn serve_lines<R: BufRead, W: Write>(
    connection: &mut VerifiedDatabaseConnection<'_>,
    input: R,
    output: &mut W,
) -> Result<(), OrchestrationError> {
    writeln!(output, "READY").map_err(|_| OrchestrationError::Invalid("stdout"))?;
    output
        .flush()
        .map_err(|_| OrchestrationError::Invalid("stdout"))?;
    for line in input.lines() {
        let line = line.map_err(|_| OrchestrationError::Invalid("stdin"))?;
        if line.is_empty() {
            continue;
        }
        let started = Instant::now();
        let reply = match handle_unprivileged_line(connection, &line) {
            Ok(body) => format!("OK\t{}\t{}us", body, started.elapsed().as_micros()),
            Err(error) => format!("ERR\t{error:?}\t{}us", started.elapsed().as_micros()),
        };
        writeln!(output, "{reply}").map_err(|_| OrchestrationError::Invalid("stdout"))?;
        output
            .flush()
            .map_err(|_| OrchestrationError::Invalid("stdout"))?;
        if line.contains("\"operation\":\"Shutdown\"") {
            break;
        }
    }
    Ok(())
}

fn valid_service_capability(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn capability_matches(expected: &str, observed: &str) -> bool {
    if !valid_service_capability(expected) || !valid_service_capability(observed) {
        return false;
    }
    expected.as_bytes().iter().zip(observed.as_bytes()).fold(0u8, |diff, (a,b)| diff | (a ^ b)) == 0
}

/// Authenticated main-service channel. The capability authenticates only the
/// service process instance; each authority-sensitive operation keeps its own
/// Product Authority checks. It is never an OwnerIssuer or reusable grant.
pub(crate) fn serve_authenticated_pipe(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    pipe: &PrivatePipeConnection,
    expected_capability: &str,
) -> Result<(), OrchestrationError> {
    if !valid_service_capability(expected_capability) {
        return Err(OrchestrationError::AccessDenied);
    }
    let frame = pipe.read_frame().map_err(|_| OrchestrationError::Invalid("pipe read"))?;
    let line = std::str::from_utf8(&frame).map_err(|_| OrchestrationError::Invalid("utf8"))?;
    let decoded = decode_operation_frame(line.as_bytes()).map_err(protocol_error)?;
    let fields = authority_fields(line)?;
    if decoded.name != "AuthenticateService" || fields.len() != 2
        || required(&fields, "operation")? != "AuthenticateService"
        || !capability_matches(expected_capability, required(&fields, "capability")?) {
        return Err(OrchestrationError::AccessDenied);
    }
    pipe.write_frame(b"OK\t{\"authenticated\":true}\t0us")
        .map_err(|_| OrchestrationError::Invalid("pipe write"))?;
    loop {
        let frame = pipe.read_frame().map_err(|_| OrchestrationError::Invalid("pipe read"))?;
        let line = std::str::from_utf8(&frame).map_err(|_| OrchestrationError::Invalid("utf8"))?;
        let started = Instant::now();
        let reply = match handle_authenticated_line(connection, owner, line) {
            Ok(body) => format!("OK\t{}\t{}us", body, started.elapsed().as_micros()),
            Err(error) => format!("ERR\t{error:?}\t{}us", started.elapsed().as_micros()),
        };
        pipe.write_frame(reply.as_bytes()).map_err(|_| OrchestrationError::Invalid("pipe write"))?;
        if line.contains("\"operation\":\"Shutdown\"") { break; }
    }
    Ok(())
}

pub fn serve_pipe(
    connection: &mut VerifiedDatabaseConnection<'_>,
    pipe: &PrivatePipeConnection,
) -> Result<(), OrchestrationError> {
    loop {
        let frame = pipe
            .read_frame()
            .map_err(|_| OrchestrationError::Invalid("pipe read"))?;
        let line = std::str::from_utf8(&frame).map_err(|_| OrchestrationError::Invalid("utf8"))?;
        let started = Instant::now();
        let reply = match handle_unprivileged_line(connection, line) {
            Ok(body) => format!("OK\t{}\t{}us", body, started.elapsed().as_micros()),
            Err(error) => format!("ERR\t{error:?}\t{}us", started.elapsed().as_micros()),
        };
        pipe.write_frame(reply.as_bytes())
            .map_err(|_| OrchestrationError::Invalid("pipe write"))?;
        if line.contains("\"operation\":\"Shutdown\"") {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod service_capability_tests {
    use super::capability_matches;

    #[test]
    fn service_capability_requires_exact_lower_hex_secret() {
        let secret = "ab".repeat(32);
        assert!(capability_matches(&secret, &secret));
        assert!(!capability_matches(&secret, &"ac".repeat(32)));
        assert!(!capability_matches(&secret, &secret.to_uppercase()));
        assert!(!capability_matches(&secret, "ab"));
    }
}

fn canonical_u64(value: &str) -> Result<u64, OrchestrationError> {
    if value.is_empty() || value.len() > 20 || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OrchestrationError::Invalid("canonical u64"));
    }
    value.parse::<u64>().map_err(|_| OrchestrationError::Invalid("canonical u64"))
}

fn optional_model(value: &str) -> Option<String> {
    if value.is_empty() { None } else { Some(value.to_owned()) }
}

fn decision_snapshot_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["actionDigest","actionOperationId","authRevision","bindingGeneration","bindingId",
        "candidateHash","candidateId","capabilityRevision","capacityTotal","operation","operationId","policyRevision",
        "resourceRef","resourceRevision","stateViewHash","taskRevision"])
}

fn decision_commit_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["actionIntentRef","backendKind","bindingGeneration","budgetUnits","candidateHash","choice",
        "deadlineEpochMs","decisionId","domainId","eventId","family","modelRequested","modelResolved","operation",
        "operationId","policyRevision","questionVersion","reason","receiptId","recordedAt","requiredCapacityUnits",
        "resourceReservationRef","rubricVersion","scenarioId","stateViewHash","taskRevision","capabilityRevision"])
}

const GRANTEE_READ_REQUESTS_V1: &str = "gogoke.grantee-context-read-requests.v1";
const MANIFEST_EXPECTED_VERSIONS_V1: &str = "gogoke.manifest-expected-versions.v1";
const AUTHORIZED_READ_SOURCES_V1: &str = "gogoke.authorized-context-read-sources.v1";
const ASSEMBLY_PARTITION_BINDINGS_V1: &str = "gogoke.context-assembly-partition-bindings.v1";
const TASK_MANDATORY_CONTEXT_REFS_V1: &str = "gogoke.task-mandatory-context-refs.v1";
const MAX_CONTEXT_RECORDS: usize = 64;

fn context_snapshot_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["admissionActionOperationId","admissionDigest","authRevision","bindingGeneration",
        "bindingId","domainId","manifestId","maxCandidates","maxContentBytes","operation","operationId",
        "partitionBindings","policyRevision","principalId","revocationHead","runtimeInstanceId","seatId","selectionDecisionId",
        "sessionId","sourceEpoch","taskId","taskRevision"])
}

fn grantee_read_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["operation", "readRequests"])
}

fn manifest_commit_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["canonicalManifest","eventId","expectedVersions","operation","operationId",
        "readRequests","receiptId","recordedAt","requestDigest"])
}

fn manifest_replay_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["bindingGeneration","bindingId","domainId","operation","operationId","principalId",
        "runtimeInstanceId","seatId","sessionId","sourceEpoch","taskId"])
}

fn assembly_read_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    manifest_replay_fields(line)
}

fn task_context_commit_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["domainId","eventId","expectedPreviousTaskRevision","mandatoryRefs","operation",
        "operationId","receiptId","recordedAt","taskId"])
}

fn task_context_read_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    action_fields(line, &["domainId","operation","taskId"])
}

fn assembly_identity(fields: &BTreeMap<String, String>) -> Result<ContextManifestReplayIdentity, OrchestrationError> {
    Ok(ContextManifestReplayIdentity {
        operation_id: required(fields,"operationId")?.to_owned(),
        principal_id: required(fields,"principalId")?.to_owned(),
        seat_id: required(fields,"seatId")?.to_owned(),
        task_id: required(fields,"taskId")?.to_owned(),
        session_id: required(fields,"sessionId")?.to_owned(),
        domain_id: required(fields,"domainId")?.to_owned(),
        binding_id: required(fields,"bindingId")?.to_owned(),
        binding_generation: required(fields,"bindingGeneration")?.to_owned(),
        source_epoch: required(fields,"sourceEpoch")?.to_owned(),
        runtime_instance_id: required(fields,"runtimeInstanceId")?.to_owned(),
    })
}

/// A record list is `tag|count|len:value...`, where every record has the
/// operation-fixed field width. Lengths count UTF-8 bytes, so delimiters in a
/// value are data rather than syntax. The outer 4 MiB frame remains the hard
/// byte bound and Context lists are additionally capped at 64 records.
fn decode_string_records(
    value: &str,
    tag: &'static str,
    width: usize,
    allow_empty: bool,
) -> Result<Vec<Vec<String>>, OrchestrationError> {
    let prefix = format!("{tag}|");
    let bytes = value.as_bytes();
    if !value.starts_with(&prefix) {
        return Err(OrchestrationError::Invalid("context record schema"));
    }
    let mut offset = prefix.len();
    let count_end = bytes[offset..]
        .iter()
        .position(|byte| *byte == b'|')
        .map(|index| offset + index)
        .ok_or(OrchestrationError::Invalid("context record count"))?;
    let count = canonical_u64(&value[offset..count_end])?;
    let count = usize::try_from(count)
        .map_err(|_| OrchestrationError::Invalid("context record count"))?;
    if count > MAX_CONTEXT_RECORDS || (!allow_empty && count == 0) {
        return Err(OrchestrationError::Invalid("context record count"));
    }
    offset = count_end + 1;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let mut record = Vec::with_capacity(width);
        for _ in 0..width {
            let length_end = bytes[offset..]
                .iter()
                .position(|byte| *byte == b':')
                .map(|index| offset + index)
                .ok_or(OrchestrationError::Invalid("context record length"))?;
            let length = canonical_u64(&value[offset..length_end])?;
            let length = usize::try_from(length)
                .map_err(|_| OrchestrationError::Invalid("context record length"))?;
            let start = length_end + 1;
            let end = start
                .checked_add(length)
                .filter(|end| *end <= bytes.len())
                .ok_or(OrchestrationError::Invalid("context record length"))?;
            let field = std::str::from_utf8(&bytes[start..end])
                .map_err(|_| OrchestrationError::Invalid("context record utf8"))?;
            record.push(field.to_owned());
            offset = end;
        }
        records.push(record);
    }
    if offset != bytes.len() {
        return Err(OrchestrationError::Invalid("context record trailing bytes"));
    }
    Ok(records)
}

fn encode_string_records(tag: &str, records: &[Vec<String>]) -> String {
    let mut encoded = format!("{tag}|{}|", records.len());
    for record in records {
        for field in record {
            encoded.push_str(&field.len().to_string());
            encoded.push(':');
            encoded.push_str(field);
        }
    }
    encoded
}

fn decode_grantee_read_requests(value: &str) -> Result<Vec<GranteeContextReadRequest>, OrchestrationError> {
    decode_string_records(value, GRANTEE_READ_REQUESTS_V1, 15, false)?
        .into_iter()
        .map(|field| Ok(GranteeContextReadRequest {
            principal_id: field[0].clone(),
            seat_id: field[1].clone(),
            source: ContextReadRequest {
                source_domain_id: field[2].clone(), context_id: field[3].clone(), version: field[4].clone(),
                expected_scope: field[5].clone(), expected_content_hash: field[6].clone(),
                expected_access_policy_revision: field[7].clone(), destination_domain_id: field[8].clone(),
                destination_scope: field[9].clone(), promotion_kind: field[10].clone(), policy_revision: field[11].clone(),
                grant: GrantRef { grant_id: field[12].clone(), revision: field[13].clone(), revocation_head: field[14].clone() },
            },
        }))
        .collect()
}

fn decode_expected_versions(value: &str) -> Result<Vec<ManifestExpectedVersion>, OrchestrationError> {
    decode_string_records(value, MANIFEST_EXPECTED_VERSIONS_V1, 6, true)?
        .into_iter()
        .map(|field| Ok(ManifestExpectedVersion {
            source_domain_id: field[0].clone(), context_id: field[1].clone(), version: field[2].clone(),
            content_hash: field[3].clone(), state_revision: field[4].clone(),
            access_policy_revision: field[5].clone(),
        }))
        .collect()
}

fn decode_partition_bindings(value: &str) -> Result<Vec<authority::ContextPartitionGrantBinding>, OrchestrationError> {
    decode_string_records(value, ASSEMBLY_PARTITION_BINDINGS_V1, 6, false)?
        .into_iter()
        .map(|field| Ok(authority::ContextPartitionGrantBinding {
            source_domain_id: field[0].clone(), destination_scope: field[1].clone(),
            promotion_kind: field[2].clone(), grant: GrantRef {
                grant_id: field[3].clone(), revision: field[4].clone(), revocation_head: field[5].clone(),
            },
        }))
        .collect()
}

fn decode_mandatory_context_refs(value: &str) -> Result<Vec<MandatoryContextRef>, OrchestrationError> {
    decode_string_records(value, TASK_MANDATORY_CONTEXT_REFS_V1, 3, true)?
        .into_iter()
        .map(|field| Ok(MandatoryContextRef {
            source_domain_id: field[0].clone(), context_id: field[1].clone(), version: field[2].clone(),
        }))
        .collect()
}

fn json_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '"' => quoted.push_str("\\\""),
            '\\' => quoted.push_str("\\\\"),
            '\u{0008}' => quoted.push_str("\\b"),
            '\u{000c}' => quoted.push_str("\\f"),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character if character < '\u{0020}' => {
                use std::fmt::Write as _;
                write!(&mut quoted, "\\u{:04x}", character as u32).expect("write to string");
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

fn authorized_read_set_body(set: &authority::AuthorizedContextReadSet) -> String {
    let records = set.sources.iter().map(|item| vec![
        item.grant_revision.clone(), item.revocation_head.clone(), item.state.clone(), item.state_revision.clone(),
        item.source.source_domain_id.clone(), item.source.context_id.clone(), item.source.version.clone(),
        item.source.scope.clone(), item.source.kind.clone(), item.source.content_hash.clone(),
        item.source.source_ref.clone(), item.source.source_hash.clone(), item.source.source_authority_kind.clone(),
        item.source.source_authority_ref.clone(), item.source.access_policy_revision.clone(),
    ]).collect::<Vec<_>>();
    let sources = encode_string_records(AUTHORIZED_READ_SOURCES_V1, &records);
    format!(
        "{{\"destinationDomainId\":{},\"destinationScope\":{},\"policyRevision\":{},\"principalId\":{},\"promotionKind\":{},\"revocationHead\":{},\"seatId\":{},\"sources\":{}}}",
        json_quote(&set.destination_domain_id), json_quote(&set.destination_scope), json_quote(&set.policy_revision),
        json_quote(&set.principal_id), json_quote(&set.promotion_kind), json_quote(&set.revocation_head),
        json_quote(&set.seat_id), json_quote(&sources)
    )
}

fn manifest_receipt_body(receipt: &authority::ContextManifestAuthorityReceipt) -> Result<String, OrchestrationError> {
    let canonical = std::str::from_utf8(&receipt.canonical_manifest)
        .map_err(|_| OrchestrationError::Invalid("canonical manifest utf8"))?;
    Ok(format!(
        "{{\"canonicalManifest\":{},\"disposition\":{},\"manifestHash\":{},\"manifestId\":{},\"operationId\":{}}}",
        json_quote(canonical), json_quote(receipt.disposition), json_quote(&receipt.manifest_hash),
        json_quote(&receipt.manifest_id), json_quote(&receipt.operation_id)
    ))
}

fn assembly_basis_body(basis: &authority::ContextAssemblyBasis) -> String {
    let partitions = encode_string_records(
        ASSEMBLY_PARTITION_BINDINGS_V1,
        &basis.partition_grant_bindings.iter().map(|binding| vec![
            binding.source_domain_id.clone(), binding.destination_scope.clone(), binding.promotion_kind.clone(),
            binding.grant.grant_id.clone(), binding.grant.revision.clone(), binding.grant.revocation_head.clone(),
        ]).collect::<Vec<_>>(),
    );
    let mandatory_refs = encode_string_records(
        TASK_MANDATORY_CONTEXT_REFS_V1,
        &basis.mandatory_refs.iter().map(|reference| vec![
            reference.source_domain_id.clone(), reference.context_id.clone(), reference.version.clone(),
        ]).collect::<Vec<_>>(),
    );
    format!(
        "{{\"authRevision\":{},\"bindingGeneration\":{},\"mandatoryRefs\":{},\"maxCandidates\":{},\"maxContentBytes\":{},\"operationId\":{},\"partitionBindings\":{},\"policyRevision\":{},\"revocationHead\":{},\"sourceEpoch\":{},\"taskRevision\":{}}}",
        json_quote(&basis.auth_revision), json_quote(&basis.binding_generation), json_quote(&mandatory_refs),
        json_quote(&basis.max_candidates.to_string()), json_quote(&basis.max_content_bytes.to_string()), json_quote(&basis.operation_id), json_quote(&partitions),
        json_quote(&basis.policy_revision), json_quote(&basis.revocation_head), json_quote(&basis.source_epoch),
        json_quote(&basis.task_revision),
    )
}

fn task_context_body(disposition: Option<&str>, operation_id: Option<&str>, current: &authority::TaskContextRequirements) -> String {
    let mandatory_refs = encode_string_records(
        TASK_MANDATORY_CONTEXT_REFS_V1,
        &current.mandatory_refs.iter().map(|reference| vec![
            reference.source_domain_id.clone(), reference.context_id.clone(), reference.version.clone(),
        ]).collect::<Vec<_>>(),
    );
    match (disposition, operation_id) {
        (Some(disposition), Some(operation_id)) => format!(
            "{{\"contentHash\":{},\"disposition\":{},\"domainId\":{},\"mandatoryRefs\":{},\"operationId\":{},\"taskId\":{},\"taskRevision\":{}}}",
            json_quote(&current.content_hash), json_quote(disposition), json_quote(&current.domain_id),
            json_quote(&mandatory_refs), json_quote(operation_id), json_quote(&current.task_id), json_quote(&current.task_revision)),
        _ => format!(
            "{{\"contentHash\":{},\"domainId\":{},\"mandatoryRefs\":{},\"taskId\":{},\"taskRevision\":{}}}",
            json_quote(&current.content_hash), json_quote(&current.domain_id), json_quote(&mandatory_refs),
            json_quote(&current.task_id), json_quote(&current.task_revision)),
    }
}

fn assembly_sources_body(operation: &str, sources: &[authority::ContextAssemblySource]) -> String {
    let encoded = encode_string_records(
        "gogoke.context-assembly-sources.v1",
        &sources.iter().map(|source| vec![
            source.source_domain_id.clone(), source.context_id.clone(), source.version.clone(), source.scope.clone(),
            source.kind.clone(), source.content_hash.clone(), source.source_ref.clone(), source.source_hash.clone(),
            source.source_authority_kind.clone(), source.source_authority_ref.clone(),
            source.access_policy_revision.clone(), source.state_revision.clone(), source.grant_id.clone(),
            source.grant_revision.clone(), source.grant_revocation_head.clone(),
        ]).collect::<Vec<_>>(),
    );
    format!("{{\"operationId\":{},\"sources\":{}}}", json_quote(operation), json_quote(&encoded))
}

fn json_string_array(values: &[String]) -> String {
    format!("[{}]", values.iter().map(|value| json_quote(value)).collect::<Vec<_>>().join(","))
}

fn delegation_grant_body(grant: &authority::DelegationGrantSnapshot) -> String {
    let parent = grant.parent.as_ref().map_or_else(
        || "null".to_owned(),
        |parent| format!("{{\"grantRef\":{},\"revision\":{}}}", json_quote(&parent.grant_id), json_quote(&parent.revision)),
    );
    let ceiling = &grant.ceiling;
    format!(
        "{{\"binding\":{{\"executionId\":{},\"generation\":{},\"sessionId\":{}}},\"ceiling\":{{\"allowedActions\":{},\"allowedContinuationResponses\":{},\"allowedMaterialClasses\":{},\"allowedSinks\":{},\"allowedTargetDomainIds\":{},\"allowedTargetPrincipalIds\":{},\"explicitPrivateMaterialIds\":{},\"maxMaterialBytes\":{},\"maxMaterialItems\":{},\"maxResponseBytes\":{}}},\"expiresAtEpochMs\":{},\"grantRef\":{},\"issuerId\":{},\"parentGrant\":{},\"policyRevision\":{},\"principal\":{{\"domainId\":{},\"principalId\":{},\"projectId\":{},\"role\":{},\"seatId\":{}}},\"revision\":{},\"revocationHead\":{},\"seatId\":{}}}",
        json_quote(&grant.binding.execution_id), json_quote(&grant.binding.generation), json_quote(&grant.binding.session_id),
        json_string_array(&ceiling.allowed_actions), json_string_array(&ceiling.allowed_continuation_responses),
        json_string_array(&ceiling.allowed_material_classes), json_string_array(&ceiling.allowed_sinks),
        json_string_array(&ceiling.allowed_target_domain_ids), json_string_array(&ceiling.allowed_target_principal_ids),
        json_string_array(&ceiling.explicit_private_material_ids), json_quote(&ceiling.max_material_bytes.to_string()),
        json_quote(&ceiling.max_material_items.to_string()), json_quote(&ceiling.max_response_bytes.to_string()),
        json_quote(&grant.expires_at_epoch_ms.to_string()), json_quote(&grant.reference.grant_id),
        json_quote(&grant.issuer_id), parent, json_quote(&grant.policy_revision),
        json_quote(&grant.principal.domain_id), json_quote(&grant.principal.principal_id),
        json_quote(&grant.principal.project_id), json_quote(&grant.principal.role),
        json_quote(&grant.principal.seat_id), json_quote(&grant.reference.revision),
        json_quote(&grant.reference.revocation_head), json_quote(&grant.principal.seat_id),
    )
}

fn replay_body(replay: &authority::DurableDecisionReplay) -> String {
    format!(
        "{{\"kind\":\"replayed\",\"operationId\":\"{}\",\"decisionReceiptId\":\"{}\",\"record\":{}}}",
        replay.record.operation_id, replay.receipt_id, authority::durable_record_json(&replay.record)
    )
}

const ACTION_PREPARE_FIELDS: [&str; 13] = [
    "actionKind", "actionOperationId", "contextManifestId", "domainId", "lane", "operation",
    "packageOperationId", "parentGrantRef", "payload", "recipeId", "reservationId", "sessionId", "taskId",
];

fn handle_authenticated_line(
    connection: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer,
    line: &str,
) -> Result<String, OrchestrationError> {
    let decoded = decode_operation_frame(line.as_bytes()).map_err(protocol_error)?;
    match decoded.name {
        "CommitTaskContextRequirements" => {
            let fields = task_context_commit_fields(line)?;
            let expected = required(&fields,"expectedPreviousTaskRevision")?;
            let receipt = authority::commit_task_context_requirements(connection, &CommitTaskContextRequirements {
                operation_id: required(&fields,"operationId")?.to_owned(),
                domain_id: required(&fields,"domainId")?.to_owned(),
                task_id: required(&fields,"taskId")?.to_owned(),
                expected_previous_revision: if expected.is_empty() { None } else { Some(expected.to_owned()) },
                mandatory_refs: decode_mandatory_context_refs(required(&fields,"mandatoryRefs")?)?,
                event_id: required(&fields,"eventId")?.to_owned(),
                receipt_id: required(&fields,"receiptId")?.to_owned(),
                recorded_at: required(&fields,"recordedAt")?.to_owned(),
            })?;
            Ok(task_context_body(Some(receipt.disposition), Some(&receipt.operation_id), &receipt.current))
        }
        "ReadTaskContextRequirements" => {
            let fields = task_context_read_fields(line)?;
            let current = authority::read_task_context_requirements(
                connection, required(&fields,"domainId")?, required(&fields,"taskId")?)?;
            Ok(task_context_body(None, None, &current))
        }
        "ReadCurrentDelegationGrant" => {
            let fields = action_fields(line, &["grantRef", "operation"])?;
            let grant = authority::read_current_delegation(connection, required(&fields, "grantRef")?)?;
            Ok(delegation_grant_body(&grant))
        }
        "AppendExecutionRecipe" => append_execution_recipe_frame(connection, owner, line),
        "ReadCurrentExecutionRecipe" | "ReadExecutionRecipeRevision" => {
            read_execution_recipe_frame(connection, owner, line, decoded.name)
        }
        "AppendObjectiveOutcome" => append_objective_outcome_frame(connection, line),
        "ReadObjectiveOutcome" => read_objective_outcome_frame(connection, line),
        "AppendEvaluation" => append_evaluation_frame(connection, line),
        "ReadEvaluation" => read_evaluation_frame(connection, line),
        "AppendDreamRun" => append_dream_run_frame(connection, line),
        "ReadDreamRun" => read_dream_record_frame(connection, line, true),
        "AppendDreamProposal" => append_dream_proposal_frame(connection, line),
        "ReadDreamProposal" => read_dream_record_frame(connection, line, false),
        "ReadSessionLineage" => read_session_lineage_frame(connection, line),
        "ReadExposureReceipt" => read_exposure_frame(connection, line),
        "PublishContextAssemblySnapshot" => {
            let fields = context_snapshot_fields(line)?;
            authority::publish_context_assembly_snapshot(connection, &ContextAssemblySnapshot {
                operation_id: required(&fields,"operationId")?.to_owned(),
                principal_id: required(&fields,"principalId")?.to_owned(),
                seat_id: required(&fields,"seatId")?.to_owned(),
                task_id: required(&fields,"taskId")?.to_owned(),
                session_id: required(&fields,"sessionId")?.to_owned(),
                domain_id: required(&fields,"domainId")?.to_owned(),
                binding_id: required(&fields,"bindingId")?.to_owned(),
                binding_generation: required(&fields,"bindingGeneration")?.to_owned(),
                source_epoch: required(&fields,"sourceEpoch")?.to_owned(),
                runtime_instance_id: required(&fields,"runtimeInstanceId")?.to_owned(),
                task_revision: required(&fields,"taskRevision")?.to_owned(),
                policy_revision: required(&fields,"policyRevision")?.to_owned(),
                auth_revision: required(&fields,"authRevision")?.to_owned(),
                revocation_head: required(&fields,"revocationHead")?.to_owned(),
                selection_decision_id: required(&fields,"selectionDecisionId")?.to_owned(),
                manifest_id: required(&fields,"manifestId")?.to_owned(),
                admission_action_operation_id: required(&fields,"admissionActionOperationId")?.to_owned(),
                admission_digest: required(&fields,"admissionDigest")?.to_owned(),
                max_content_bytes: canonical_u64(required(&fields,"maxContentBytes")?)?,
                max_candidates: canonical_u64(required(&fields,"maxCandidates")?)?,
                partition_grant_bindings: decode_partition_bindings(required(&fields,"partitionBindings")?)?,
            })?;
            Ok("{\"published\":true}".into())
        }
        "ReadGranteeContextSet" => {
            let fields = grantee_read_fields(line)?;
            let requests = decode_grantee_read_requests(required(&fields,"readRequests")?)?;
            let set = authority::read_grantee_context_set(connection, &requests)?;
            Ok(authorized_read_set_body(&set))
        }
        "ReadContextAssemblyBasis" => {
            let fields = assembly_read_fields(line)?;
            let identity = assembly_identity(&fields)?;
            let basis = authority::read_context_assembly_basis(connection, &identity)?;
            Ok(assembly_basis_body(&basis))
        }
        "ListContextAssemblySources" => {
            let fields = assembly_read_fields(line)?;
            let identity = assembly_identity(&fields)?;
            let sources = authority::list_context_assembly_sources(connection, &identity)?;
            Ok(assembly_sources_body(&identity.operation_id, &sources))
        }
        "CommitContextManifest" => {
            let fields = manifest_commit_fields(line)?;
            let input = ContextManifestCommitInput {
                operation_id: required(&fields,"operationId")?.to_owned(),
                request_digest: required(&fields,"requestDigest")?.to_owned(),
                event_id: required(&fields,"eventId")?.to_owned(),
                receipt_id: required(&fields,"receiptId")?.to_owned(),
                recorded_at: required(&fields,"recordedAt")?.to_owned(),
                read_requests: decode_grantee_read_requests(required(&fields,"readRequests")?)?,
                expected_versions: decode_expected_versions(required(&fields,"expectedVersions")?)?,
                canonical_manifest: required(&fields,"canonicalManifest")?.as_bytes().to_vec(),
            };
            let receipt = authority::commit_context_manifest(connection, &input)?;
            manifest_receipt_body(&receipt)
        }
        "ReadContextManifest" => {
            let fields = manifest_replay_fields(line)?;
            let receipt = authority::read_context_manifest(connection, &assembly_identity(&fields)?)?;
            manifest_receipt_body(&receipt)
        }
        "PublishDecisionSnapshot" => {
            let fields = decision_snapshot_fields(line)?;
            authority::publish_decision_snapshot(connection, &DecisionAuthoritySnapshot {
                operation_id: required(&fields,"operationId")?.to_owned(),
                candidate_id: required(&fields,"candidateId")?.to_owned(),
                state_view_hash: required(&fields,"stateViewHash")?.to_owned(),
                candidate_hash: required(&fields,"candidateHash")?.to_owned(),
                task_revision: required(&fields,"taskRevision")?.to_owned(),
                policy_revision: required(&fields,"policyRevision")?.to_owned(),
                capability_revision: required(&fields,"capabilityRevision")?.to_owned(),
                binding_id: required(&fields,"bindingId")?.to_owned(),
                binding_generation: required(&fields,"bindingGeneration")?.to_owned(),
                auth_revision: required(&fields,"authRevision")?.to_owned(),
                resource_ref: required(&fields,"resourceRef")?.to_owned(),
                resource_revision: required(&fields,"resourceRevision")?.to_owned(),
                capacity_total: canonical_u64(required(&fields,"capacityTotal")?)?,
                action_operation_id: required(&fields,"actionOperationId")?.to_owned(),
                action_digest: required(&fields,"actionDigest")?.to_owned(),
            })?;
            Ok("{\"published\":true}".into())
        }
        "CommitDecision" => {
            let fields = decision_commit_fields(line)?;
            let input = DecisionCommitInput {
                domain_id: required(&fields,"domainId")?.to_owned(),
                decision_id: required(&fields,"decisionId")?.to_owned(),
                event_id: required(&fields,"eventId")?.to_owned(),
                receipt_id: required(&fields,"receiptId")?.to_owned(),
                recorded_at: required(&fields,"recordedAt")?.to_owned(),
                record: DurableDecisionRecord {
                    operation_id: required(&fields,"operationId")?.to_owned(),
                    scenario_id: required(&fields,"scenarioId")?.to_owned(),
                    family: required(&fields,"family")?.to_owned(),
                    state_view_hash: required(&fields,"stateViewHash")?.to_owned(),
                    candidate_hash: required(&fields,"candidateHash")?.to_owned(),
                    question_version: required(&fields,"questionVersion")?.to_owned(),
                    rubric_version: required(&fields,"rubricVersion")?.to_owned(),
                    model_requested: optional_model(required(&fields,"modelRequested")?),
                    model_resolved: optional_model(required(&fields,"modelResolved")?),
                    task_revision: required(&fields,"taskRevision")?.to_owned(),
                    policy_revision: required(&fields,"policyRevision")?.to_owned(),
                    capability_revision: required(&fields,"capabilityRevision")?.to_owned(),
                    binding_generation: required(&fields,"bindingGeneration")?.to_owned(),
                    backend_kind: required(&fields,"backendKind")?.to_owned(),
                    choice: required(&fields,"choice")?.to_owned(),
                    reason: required(&fields,"reason")?.to_owned(),
                    budget_units: canonical_u64(required(&fields,"budgetUnits")?)?,
                    deadline_epoch_ms: canonical_u64(required(&fields,"deadlineEpochMs")?)?,
                },
                resource_reservation_ref: required(&fields,"resourceReservationRef")?.to_owned(),
                action_intent_ref: required(&fields,"actionIntentRef")?.to_owned(),
                required_capacity_units: canonical_u64(required(&fields,"requiredCapacityUnits")?)?,
            };
            let result = authority::commit_decision(connection, &input)?;
            Ok(match result.disposition {
                DecisionCommitDisposition::Committed => format!(
                    "{{\"kind\":\"committed\",\"operationId\":\"{}\",\"decisionReceiptId\":\"{}\"}}",
                    result.replay.record.operation_id, result.replay.receipt_id
                ),
                DecisionCommitDisposition::Replayed => replay_body(&result.replay),
            })
        }
        "ReadDecisionReplay" => {
            let fields = action_fields(line, &["domainId","operation","operationId"])?;
            let replay = authority::read_durable_decision_replay(
                connection, required(&fields,"domainId")?, required(&fields,"operationId")?)?;
            Ok(replay_body(&replay))
        }
        _ => handle_line(connection, line),
    }
}

fn recipe_value(value: NativeJson) -> Result<RecipeJsonValue, OrchestrationError> {
    Ok(match value {
        NativeJson::Null => RecipeJsonValue::Null,
        NativeJson::Bool(v) => RecipeJsonValue::Bool(v),
        NativeJson::Number(v) => RecipeJsonValue::Number(v.parse().map_err(|_| OrchestrationError::Invalid("recipe number"))?),
        NativeJson::String(v) => RecipeJsonValue::String(RecipeJsonString::from_utf16_units(v.units().to_vec())),
        NativeJson::Array(v) => RecipeJsonValue::Array(v.into_iter().map(recipe_value).collect::<Result<_,_>>()?),
        NativeJson::Object(v) => RecipeJsonValue::Object(v.into_iter().map(|(k,v)| Ok((RecipeJsonString::from_utf16_units(k.units().to_vec()), recipe_value(v)?))).collect::<Result<_,OrchestrationError>>()?),
    })
}

fn recipe_object(value: &str) -> Result<RecipeJsonObject, OrchestrationError> {
    match NativeJsonParser::parse(value).map_err(|_| OrchestrationError::Invalid("recipe json"))? {
        NativeJson::Object(v) => v.into_iter().map(|(k,v)| Ok((RecipeJsonString::from_utf16_units(k.units().to_vec()), recipe_value(v)?))).collect(),
        _ => Err(OrchestrationError::Invalid("recipe object")),
    }
}

fn recipe_any(value: &str) -> Result<RecipeJsonValue, OrchestrationError> {
    recipe_value(NativeJsonParser::parse(value).map_err(|_| OrchestrationError::Invalid("recipe json"))?)
}

fn recipe_body(v: &authority::ExecutionRecipeVersion) -> String {
    format!("{{\"contentHash\":{},\"domainId\":{},\"recipeId\":{},\"revision\":{},\"seatId\":{},\"runtimeInstanceId\":{},\"contextManifestId\":{},\"admissionRef\":{}}}",
        json_quote(&v.content_hash), json_quote(&v.domain_id), json_quote(&v.recipe.recipe_id), json_quote(&v.recipe.revision), json_quote(&v.recipe.seat_id), json_quote(&v.recipe.runtime_instance_id), json_quote(&v.recipe.context_manifest_id), json_quote(&v.recipe.admission_ref))
}

fn append_execution_recipe_frame(connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, line: &str) -> Result<String, OrchestrationError> {
    let fields = action_fields(line, &["admissionRef","budgetPolicy","contextManifestId","domainId","eventId","expectedPreviousRevision","isolationProfile","modelRef","operation","operationId","receiptId","recordedAt","recipeId","runtimeInstanceId","seatId","toolProfile"])?;
    let expected = required(&fields,"expectedPreviousRevision")?;
    let receipt = authority::append_owner_execution_recipe(connection, owner, &AppendExecutionRecipe {
        operation_id: required(&fields,"operationId")?.into(), domain_id: required(&fields,"domainId")?.into(), expected_previous_revision: if expected.is_empty(){None}else{Some(expected.into())}, recipe_id: required(&fields,"recipeId")?.into(), seat_id: required(&fields,"seatId")?.into(), runtime_instance_id: required(&fields,"runtimeInstanceId")?.into(), model_ref: recipe_object(required(&fields,"modelRef")?)?, tool_profile: recipe_any(required(&fields,"toolProfile")?)?, isolation_profile: recipe_any(required(&fields,"isolationProfile")?)?, context_manifest_id: required(&fields,"contextManifestId")?.into(), budget_policy: recipe_any(required(&fields,"budgetPolicy")?)?, admission_ref: required(&fields,"admissionRef")?.into(), event_id: required(&fields,"eventId")?.into(), receipt_id: required(&fields,"receiptId")?.into(), recorded_at: required(&fields,"recordedAt")?.into(),
    })?;
    Ok(format!("{{\"disposition\":{},\"operationId\":{},\"currentnessStatus\":{},\"recipe\":{}}}", json_quote(receipt.disposition), json_quote(&receipt.operation_id), json_quote(receipt.currentness_status), recipe_body(&receipt.version)))
}

fn read_execution_recipe_frame(connection: &mut VerifiedDatabaseConnection<'_>, owner: &OwnerIssuer, line: &str, operation: &str) -> Result<String, OrchestrationError> {
    let fields = if operation == "ReadCurrentExecutionRecipe" { action_fields(line, &["domainId","operation","recipeId"])? } else { action_fields(line, &["domainId","operation","recipeId","revision"])? };
    let item = if operation == "ReadCurrentExecutionRecipe" { authority::read_current_execution_recipe(connection, owner, required(&fields,"domainId")?, required(&fields,"recipeId")?)? } else { authority::read_execution_recipe_revision(connection, owner, required(&fields,"domainId")?, required(&fields,"recipeId")?, required(&fields,"revision")?)? };
    item.map(|v| recipe_body(&v)).ok_or(OrchestrationError::Invalid("recipe missing"))
}

fn hash_ref(value: &str) -> Result<String, OrchestrationError> { Ok(value.to_owned()) }
fn objective_refs(value: &str) -> Result<Vec<ObjectiveEvidenceRef>, OrchestrationError> {
    decode_string_records(value, "gogoke.objective-evidence.v1", 4, true)?.into_iter().map(|f| Ok(ObjectiveEvidenceRef{object_type:f[0].clone(),object_id:f[1].clone(),object_version:f[2].clone(),content_hash:hash_ref(&f[3])?})).collect()
}
fn evaluation_outcomes(value: &str) -> Result<Vec<EvaluationOutcomeRef>, OrchestrationError> {
    decode_string_records(value, "gogoke.evaluation-outcomes.v1", 3, false)?.into_iter().map(|f| Ok(EvaluationOutcomeRef{outcome_id:f[0].clone(),revision:f[1].clone(),content_hash:f[2].clone()})).collect()
}
fn evaluation_evidence(value: &str) -> Result<Vec<EvaluationEvidenceRef>, OrchestrationError> {
    decode_string_records(value, "gogoke.evaluation-evidence.v1", 4, true)?.into_iter().map(|f| Ok(EvaluationEvidenceRef{object_type:f[0].clone(),object_id:f[1].clone(),object_version:f[2].clone(),content_hash:f[3].clone()})).collect()
}
fn append_objective_outcome_frame(connection: &mut VerifiedDatabaseConnection<'_>, line: &str) -> Result<String, OrchestrationError> {
    let f=action_fields(line,&["actionCompletionRef","actionOperationId","decisionHash","decisionId","decisionVersion","domainId","evidenceRefs","eventId","expectedPreviousRevision","expectedPreviousContentHash","manifestHash","manifestId","manifestVersion","observationEndsAt","observationStartsAt","observationStatus","operation","operationId","outcomeId","receiptId","recordedAt","resultRefs","revision"])?;
    let prev=required(&f,"expectedPreviousRevision")?; let parse_prev=if prev.is_empty(){None}else{Some(ObjectiveVersionRef{revision:prev.into(),content_hash:required(&f,"expectedPreviousContentHash")?.into()})};
    let x=AppendObjectiveOutcome{domain_id:required(&f,"domainId")?.into(),outcome_id:required(&f,"outcomeId")?.into(),revision:required(&f,"revision")?.into(),operation_id:required(&f,"operationId")?.into(),event_id:required(&f,"eventId")?.into(),receipt_id:required(&f,"receiptId")?.into(),recorded_at:required(&f,"recordedAt")?.into(),manifest_id:required(&f,"manifestId")?.into(),manifest_version:required(&f,"manifestVersion")?.into(),manifest_hash:required(&f,"manifestHash")?.into(),decision_id:required(&f,"decisionId")?.into(),decision_version:required(&f,"decisionVersion")?.into(),decision_hash:required(&f,"decisionHash")?.into(),action_operation_id:required(&f,"actionOperationId")?.into(),action_completion_ref:required(&f,"actionCompletionRef")?.into(),result_refs:objective_refs(required(&f,"resultRefs")?)?,evidence_refs:objective_refs(required(&f,"evidenceRefs")?)?,observation:ObjectiveObservationWindow{starts_at:required(&f,"observationStartsAt")?.into(),ends_at:required(&f,"observationEndsAt")?.into(),status:required(&f,"observationStatus")?.into()},previous:parse_prev};
    let r=authority::append_objective_outcome(connection,&x)?; Ok(format!("{{\"contentHash\":{},\"disposition\":{},\"objectId\":{},\"operationId\":{},\"receiptId\":{},\"revision\":{}}}",json_quote(&r.object_hash),json_quote(r.disposition),json_quote(&x.outcome_id),json_quote(&r.operation_id),json_quote(&r.receipt_id),json_quote(&x.revision)))
}
fn read_objective_outcome_frame(connection: &mut VerifiedDatabaseConnection<'_>, line: &str) -> Result<String, OrchestrationError> { let f=action_fields(line,&["domainId","operation","outcomeId","revision"])?; let r=authority::read_objective_outcome(connection,required(&f,"domainId")?,required(&f,"outcomeId")?,required(&f,"revision")?)?; Ok(format!("{{\"contentHash\":{},\"domainId\":{},\"objectId\":{},\"revision\":{}}}",json_quote(&r.content_hash),json_quote(&r.domain_id),json_quote(&r.outcome_id),json_quote(&r.revision))) }
fn append_evaluation_frame(connection: &mut VerifiedDatabaseConnection<'_>, line: &str) -> Result<String, OrchestrationError> { let f=action_fields(line,&["calibrationKey","calibrationVersion","datasetNamespace","datasetSplit","decisionFamily","domainId","evidenceRefs","eventId","evaluationId","expectedPreviousContentHash","expectedPreviousRevision","metricsHash","operation","operationId","outcomeRefs","privacyStatus","recordedAt","receiptId","revision","rubricVersion","safetyStatus","scorerVersion","sourceIdentity"])?; let prev=required(&f,"expectedPreviousRevision")?; let previous=if prev.is_empty(){None}else{Some(EvaluationVersionRef{revision:prev.into(),content_hash:required(&f,"expectedPreviousContentHash")?.into()})}; let x=AppendEvaluation{domain_id:required(&f,"domainId")?.into(),evaluation_id:required(&f,"evaluationId")?.into(),revision:required(&f,"revision")?.into(),operation_id:required(&f,"operationId")?.into(),event_id:required(&f,"eventId")?.into(),receipt_id:required(&f,"receiptId")?.into(),recorded_at:required(&f,"recordedAt")?.into(),source_identity:required(&f,"sourceIdentity")?.into(),outcome_refs:evaluation_outcomes(required(&f,"outcomeRefs")?)?,decision_family:required(&f,"decisionFamily")?.into(),scorer_version:required(&f,"scorerVersion")?.into(),rubric_version:required(&f,"rubricVersion")?.into(),calibration_key:required(&f,"calibrationKey")?.into(),calibration_version:required(&f,"calibrationVersion")?.into(),dataset_namespace:required(&f,"datasetNamespace")?.into(),dataset_split:required(&f,"datasetSplit")?.into(),evidence_refs:evaluation_evidence(required(&f,"evidenceRefs")?)?,metrics_hash:required(&f,"metricsHash")?.into(),safety_status:required(&f,"safetyStatus")?.into(),privacy_status:required(&f,"privacyStatus")?.into(),previous}; let r=authority::append_evaluation(connection,&x)?; Ok(format!("{{\"contentHash\":{},\"disposition\":{},\"objectId\":{},\"operationId\":{},\"receiptId\":{},\"revision\":{}}}",json_quote(&r.object_hash),json_quote(r.disposition),json_quote(&x.evaluation_id),json_quote(&r.operation_id),json_quote(&r.receipt_id),json_quote(&x.revision))) }
fn read_evaluation_frame(connection: &mut VerifiedDatabaseConnection<'_>, line: &str) -> Result<String, OrchestrationError> { let f=action_fields(line,&["domainId","evaluationId","operation","revision"])?; let r=authority::read_evaluation(connection,required(&f,"domainId")?,required(&f,"evaluationId")?,required(&f,"revision")?)?; Ok(format!("{{\"contentHash\":{},\"domainId\":{},\"objectId\":{},\"revision\":{}}}",json_quote(&r.content_hash),json_quote(&r.domain_id),json_quote(&r.evaluation_id),json_quote(&r.revision))) }
fn dream_object(value:&str)->Result<DreamObjectRef,OrchestrationError>{let mut rows=decode_string_records(value,"gogoke.dream-object.v1",4,false)?;if rows.len()!=1{return Err(OrchestrationError::Invalid("dream object"));}let f=rows.pop().ok_or(OrchestrationError::Invalid("dream object"))?;Ok(DreamObjectRef{object_type:f[0].clone(),object_id:f[1].clone(),revision:f[2].clone(),content_hash:f[3].clone()})}
fn dream_evals(value:&str)->Result<Vec<DreamEvaluationRef>,OrchestrationError>{decode_string_records(value,"gogoke.dream-evaluations.v1",3,true)?.into_iter().map(|f|Ok(DreamEvaluationRef{evaluation_id:f[0].clone(),revision:f[1].clone(),content_hash:f[2].clone()})).collect()}
fn dream_changes(value:&str)->Result<Vec<DreamAllowedChange>,OrchestrationError>{decode_string_records(value,"gogoke.dream-changes.v1",3,false)?.into_iter().map(|f|Ok(DreamAllowedChange{key:f[0].clone(),before_hash:f[1].clone(),after_hash:f[2].clone()})).collect()}
fn dream_prev(f:&BTreeMap<String,String>)->Result<Option<DreamVersionRef>,OrchestrationError>{let rev=required(f,"expectedPreviousRevision")?;if rev.is_empty(){Ok(None)}else{Ok(Some(DreamVersionRef{revision:rev.into(),content_hash:required(f,"expectedPreviousContentHash")?.into()}))}}
fn append_dream_run_frame(connection:&mut VerifiedDatabaseConnection<'_>,line:&str)->Result<String,OrchestrationError>{let f=action_fields(line,&["budgetLeaseRef","budgetOperationId","budgetResourceRef","budgetResourceRevision","budgetUnits","datasetNamespace","datasetSplit","datasetSplitHash","domainId","evaluationRefs","eventId","inputSnapshot","operation","operationId","recipeRef","recordedAt","receiptId","revision","runId","sourceIdentity","expectedPreviousRevision","expectedPreviousContentHash"])?;let x=AppendDreamRun{domain_id:required(&f,"domainId")?.into(),run_id:required(&f,"runId")?.into(),revision:required(&f,"revision")?.into(),operation_id:required(&f,"operationId")?.into(),event_id:required(&f,"eventId")?.into(),receipt_id:required(&f,"receiptId")?.into(),recorded_at:required(&f,"recordedAt")?.into(),source_identity:required(&f,"sourceIdentity")?.into(),input_snapshot:dream_object(required(&f,"inputSnapshot")?)?,dataset_namespace:required(&f,"datasetNamespace")?.into(),dataset_split:required(&f,"datasetSplit")?.into(),dataset_split_hash:required(&f,"datasetSplitHash")?.into(),recipe_ref:dream_object(required(&f,"recipeRef")?)?,budget_lease:DreamBudgetLease{lease_ref:required(&f,"budgetLeaseRef")?.into(),operation_id:required(&f,"budgetOperationId")?.into(),resource_ref:required(&f,"budgetResourceRef")?.into(),resource_revision:required(&f,"budgetResourceRevision")?.into(),units:required(&f,"budgetUnits")?.parse().map_err(|_|OrchestrationError::Invalid("budget"))?},evaluation_refs:dream_evals(required(&f,"evaluationRefs")?)?,previous:dream_prev(&f)?};let r=authority::append_dream_run(connection,&x)?;Ok(format!("{{\"contentHash\":{},\"disposition\":{},\"objectId\":{},\"operationId\":{},\"receiptId\":{},\"revision\":{}}}",json_quote(&r.object_hash),json_quote(r.disposition),json_quote(&x.run_id),json_quote(&r.operation_id),json_quote(&r.receipt_id),json_quote(&x.revision)))}
fn append_dream_proposal_frame(connection:&mut VerifiedDatabaseConnection<'_>,line:&str)->Result<String,OrchestrationError>{let f=action_fields(line,&["afterHash","allowedChangeSet","basePolicyRevision","beforeHash","candidateKind","domainId","eventId","expectedPreviousContentHash","expectedPreviousRevision","heldoutEvaluation","namespace","operation","operationId","proposalId","recordedAt","receiptId","revision","rollbackRef","runRef","sourceIdentity","testOnly"])?;let held=required(&f,"heldoutEvaluation")?;let heldout=if held.is_empty(){None}else{let mut rows=dream_evals(held)?;if rows.len()!=1{return Err(OrchestrationError::Invalid("heldout"));}Some(rows.pop().ok_or(OrchestrationError::Invalid("heldout"))?)};let x=AppendDreamProposal{domain_id:required(&f,"domainId")?.into(),proposal_id:required(&f,"proposalId")?.into(),revision:required(&f,"revision")?.into(),operation_id:required(&f,"operationId")?.into(),event_id:required(&f,"eventId")?.into(),receipt_id:required(&f,"receiptId")?.into(),recorded_at:required(&f,"recordedAt")?.into(),source_identity:required(&f,"sourceIdentity")?.into(),run_ref:dream_object(required(&f,"runRef")?)?,candidate_kind:required(&f,"candidateKind")?.into(),before_hash:required(&f,"beforeHash")?.into(),after_hash:required(&f,"afterHash")?.into(),allowed_change_set:dream_changes(required(&f,"allowedChangeSet")?)?,heldout_receipt:heldout,rollback_ref:dream_object(required(&f,"rollbackRef")?)?,base_policy_revision:required(&f,"basePolicyRevision")?.into(),namespace:required(&f,"namespace")?.into(),test_only:required(&f,"testOnly")?=="true",activation_grant:None,previous:dream_prev(&f)?};let r=authority::append_dream_proposal(connection,&x)?;Ok(format!("{{\"contentHash\":{},\"disposition\":{},\"objectId\":{},\"operationId\":{},\"receiptId\":{},\"revision\":{}}}",json_quote(&r.object_hash),json_quote(r.disposition),json_quote(&x.proposal_id),json_quote(&r.operation_id),json_quote(&r.receipt_id),json_quote(&x.revision)))}
fn read_dream_record_frame(connection:&mut VerifiedDatabaseConnection<'_>,line:&str,is_run:bool)->Result<String,OrchestrationError>{let f=action_fields(line,&["domainId","objectId","operation","revision"])?;let r=if is_run{authority::read_dream_run(connection,required(&f,"domainId")?,required(&f,"objectId")?,required(&f,"revision")?)}else{authority::read_dream_proposal(connection,required(&f,"domainId")?,required(&f,"objectId")?,required(&f,"revision")?)}?;Ok(format!("{{\"contentHash\":{},\"domainId\":{},\"objectId\":{},\"revision\":{}}}",json_quote(&r.content_hash),json_quote(&r.domain_id),json_quote(&r.object_id),json_quote(&r.revision)))}
fn read_session_lineage_frame(connection:&mut VerifiedDatabaseConnection<'_>,line:&str)->Result<String,OrchestrationError>{let f=action_fields(line,&["domainId","operation","sessionId"])?;let s=authority::read_session_lineage(connection,required(&f,"domainId")?,required(&f,"sessionId")?)?;Ok(format!("{{\"contentHash\":{},\"domainId\":{},\"lifecycle\":{},\"revision\":{},\"sessionId\":{}}}",json_quote(&s.content_hash),json_quote(&s.domain_id),json_quote(&s.lifecycle),json_quote(&s.revision),json_quote(&s.session_id)))}
fn read_exposure_frame(connection:&mut VerifiedDatabaseConnection<'_>,line:&str)->Result<String,OrchestrationError>{let f=action_fields(line,&["domainId","operation","receiptId"])?;let r=authority::read_exposure_receipt(connection,required(&f,"domainId")?,required(&f,"receiptId")?)?;Ok(format!("{{\"domainId\":{},\"receiptId\":{},\"sessionRevision\":{},\"sessionId\":{}}}",json_quote(&r.domain_id),json_quote(&r.exposure.receipt_id),json_quote(&r.session_revision),json_quote(&r.session_id)))}

fn handle_unprivileged_line(
    connection: &mut VerifiedDatabaseConnection<'_>,
    line: &str,
) -> Result<String, OrchestrationError> {
    let decoded = decode_operation_frame(line.as_bytes()).map_err(protocol_error)?;
    // Current-user SID is transport isolation, not Product Authority admission.
    // Until a typed service-actor capability is established, the textual/pipe
    // ingress may only request transport shutdown. Product reads and mutations
    // remain available through trusted in-process ProductDatabase methods.
    if decoded.name != "Shutdown" {
        return Err(OrchestrationError::AccessDenied);
    }
    handle_line(connection, line)
}

fn handle_line(
    connection: &mut VerifiedDatabaseConnection<'_>,
    line: &str,
) -> Result<String, OrchestrationError> {
    let decoded = decode_operation_frame(line.as_bytes()).map_err(protocol_error)?;
    match decoded.name {
        "CommitOrchestration" => {
            let command = parse_orchestration_command(line)?;
            let receipt = commit_orchestration(connection, command, None)?;
            Ok(format!(
                "{{\"disposition\":\"{}\",\"commandId\":\"{}\",\"resultSequence\":\"{}\"}}",
                receipt.disposition, receipt.command_id, receipt.result_sequence
            ))
        }
        "CommitContextVersion" => {
            let fields = context_fields(line)?;
            // Legacy string refs have no authenticated actor/current grant proof.
            // Keep this GLOBAL path unqualified until the typed authority commit is wired.
            if required(&fields, "scope")? == "GLOBAL" {
                return Err(OrchestrationError::AccessDenied);
            }
            let promotion = match fields.get("sourceVersionRef") {
                None => None,
                Some(source_version_ref) => Some(PromotionEvidence {
                    source_version_ref: source_version_ref.clone(),
                    source_grant_ref: required(&fields, "sourceGrantRef")?.to_owned(),
                    target_grant_ref: required(&fields, "targetGrantRef")?.to_owned(),
                    provenance_refs: split_list(fields.get("provenanceRefs")),
                }),
            };
            let receipt = commit_context_version(
                connection,
                ContextCommand {
                    operation_id: required(&fields, "operationId")?.to_owned(),
                    context_id: required(&fields, "contextId")?.to_owned(),
                    version: required(&fields, "version")?.to_owned(),
                    scope: required(&fields, "scope")?.to_owned(),
                    domain_id: required(&fields, "domainId")?.to_owned(),
                    kind: required(&fields, "kind")?.to_owned(),
                    content_hash: required(&fields, "contentHash")?.to_owned(),
                    source_ref: required(&fields, "sourceRef")?.to_owned(),
                    source_hash: required(&fields, "sourceHash")?.to_owned(),
                    source_authority_kind: required(&fields, "sourceAuthorityKind")?.to_owned(),
                    source_authority_ref: required(&fields, "sourceAuthorityRef")?.to_owned(),
                    derived_from: split_list(fields.get("derivedFrom")),
                    supersedes: split_list(fields.get("supersedes")),
                    access_policy_revision: required(&fields, "accessPolicyRevision")?.to_owned(),
                    visibility: required(&fields, "visibility")?.to_owned(),
                    read_grant_refs: split_list(fields.get("readGrantRefs")),
                    promotion,
                },
            )?;
            let invalidated = receipt
                .invalidated_version_refs
                .iter()
                .map(|reference| format!("\"{reference}\""))
                .collect::<Vec<_>>()
                .join(",");
            Ok(format!(
                "{{\"disposition\":\"{}\",\"operationId\":\"{}\",\"contextId\":\"{}\",\"version\":\"{}\",\"invalidatedVersionRefs\":[{}]}}",
                receipt.disposition,
                receipt.operation_id,
                receipt.context_id,
                receipt.version,
                invalidated
            ))
        }
        "ReserveAction" => {
            let fields = action_fields(line, &ACTION_PREPARE_FIELDS)?;
            let result = authority::prepare_action_authority(
                connection,
                &authority::PrepareActionAuthority {
                    domain_id: required(&fields, "domainId")?.to_owned(),
                    parent_grant_ref: required(&fields, "parentGrantRef")?.to_owned(),
                    package_operation_id: required(&fields, "packageOperationId")?.to_owned(),
                    task_id: required(&fields, "taskId")?.to_owned(),
                    recipe_id: required(&fields, "recipeId")?.to_owned(),
                    session_id: required(&fields, "sessionId")?.to_owned(),
                    context_manifest_id: required(&fields, "contextManifestId")?.to_owned(),
                    action_operation_id: required(&fields, "actionOperationId")?.to_owned(),
                    reservation_id: required(&fields, "reservationId")?.to_owned(),
                    action_kind: required(&fields, "actionKind")?.to_owned(),
                    lane: required(&fields, "lane")?.to_owned(),
                    payload: required(&fields, "payload")?.as_bytes().to_vec(),
                },
            )?;
            Ok(format!(
                "{{\"kind\":\"{}\",\"operationId\":\"{}\",\"semanticDigest\":\"{}\",\"reservationId\":\"{}\",\"packageDigest\":\"{}\",\"authorityStatus\":\"{}\"}}",
                if result.disposition == "COMMITTED" { "reserved" } else { "replay" },
                result.operation_id,
                result.semantic_digest,
                result.reservation_id,
                result.package_digest,
                result.authority_status,
            ))
        }
        "BeginActionCommitment" => {
            let fields = action_fields(
                line,
                &["domainId", "operation", "operationId", "reservationId"],
            )?;
            let operation_id = required(&fields, "operationId")?.to_owned();
            let reservation_id = required(&fields, "reservationId")?.to_owned();
            let result = authority::begin_committed_action(
                connection,
                &authority::BeginCommittedAction {
                    domain_id: required(&fields, "domainId")?.to_owned(),
                    operation_id: operation_id.clone(),
                    reservation_id: reservation_id.clone(),
                },
            )?;
            Ok(match result {
                authority::BeginCommittedDisposition::Granted { attempt_id, send_authority } => format!(
                    "{{\"kind\":\"granted\",\"operationId\":\"{}\",\"reservationId\":\"{}\",\"attemptId\":\"{}\",\"sendAuthority\":\"{}\"}}",
                    operation_id, reservation_id, attempt_id, send_authority),
                authority::BeginCommittedDisposition::Replay { state } => format!(
                    "{{\"kind\":\"replay\",\"operationId\":\"{}\",\"reservationId\":\"{}\",\"state\":\"{}\"}}",
                    operation_id, reservation_id, state),
                authority::BeginCommittedDisposition::CurrentFactsUnavailable { .. } => {
                    return Err(OrchestrationError::AccessDenied)
                }
            })
        }
        "ReadEvents" => {
            let fields = string_fields(line)?;
            let events = read_events(
                connection,
                required(&fields, "aggregateKind")?,
                required(&fields, "streamId")?,
                parse_i64(fields.get("fromSequenceExclusive").map(String::as_str).unwrap_or("-1"))?,
                parse_i64(fields.get("limit").map(String::as_str).unwrap_or("100"))?,
            )?;
            Ok(format!("{{\"count\":{}}}", events.len()))
        }
        "ReadSnapshot" => {
            let fields = string_fields(line)?;
            let rows = read_snapshot_projects(
                connection,
                parse_i64(fields.get("limit").map(String::as_str).unwrap_or("100"))?,
            )?;
            Ok(format!("{{\"count\":{}}}", rows.len()))
        }
        "RecordRejectedCommand" => {
            let fields = string_fields(line)?;
            let receipt = record_rejected_command(
                connection,
                required(&fields, "commandId")?,
                required(&fields, "commandType")?,
                required(&fields, "aggregateKind")?,
                required(&fields, "aggregateId")?,
                required(&fields, "occurredAt")?,
                required(&fields, "error")?,
                fields.get("accessAdmitted").map(String::as_str) != Some("false"),
            )?;
            Ok(format!(
                "{{\"disposition\":\"{}\",\"commandId\":\"{}\"}}",
                receipt.disposition, receipt.command_id
            ))
        }
        "GetReceipt" => {
            let fields = string_fields(line)?;
            let command_id = required(&fields, "commandId")?;
            let statement = Statement::prepare(
                connection.as_ptr(),
                "SELECT status FROM orchestration_command_receipts WHERE command_id = ?",
            )?;
            statement.bind_text(1, command_id)?;
            if statement.step_row()? {
                Ok(format!(
                    "{{\"found\":true,\"status\":\"{}\"}}",
                    statement.column_text(0)?
                ))
            } else {
                Ok("{\"found\":false}".into())
            }
        }
        "Shutdown" => Ok("{\"shutdown\":true}".into()),
        "ReconcileOperation" | "ReadAggregateRange" | "CommitDomainRecord" => {
            Err(OrchestrationError::Invalid("operation not admitted in this host slice"))
        }
        _ => Err(OrchestrationError::Invalid("unhandled operation")),
    }
}

fn protocol_error(error: ProtocolError) -> OrchestrationError {
    match error {
        ProtocolError::ForbiddenField(_) => OrchestrationError::Invalid("forbidden field"),
        ProtocolError::UnknownOperation(_) => OrchestrationError::Invalid("unknown operation"),
        ProtocolError::Oversize => OrchestrationError::Invalid("oversize"),
        ProtocolError::DuplicateKey => OrchestrationError::Invalid("duplicate key"),
        _ => OrchestrationError::Invalid("protocol"),
    }
}

fn required<'a>(
    fields: &'a BTreeMap<String, String>,
    key: &'static str,
) -> Result<&'a str, OrchestrationError> {
    fields
        .get(key)
        .map(String::as_str)
        .ok_or(OrchestrationError::Invalid(key))
}

fn parse_i64(value: &str) -> Result<i64, OrchestrationError> {
    value
        .parse::<i64>()
        .map_err(|_| OrchestrationError::Invalid("integer"))
}

fn authority_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    decode_flat_string_object(line.as_bytes()).map_err(protocol_error)
}

fn context_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    const REQUIRED: [&str; 17] = [
        "accessPolicyRevision",
        "contentHash",
        "contextId",
        "derivedFrom",
        "domainId",
        "kind",
        "operation",
        "operationId",
        "readGrantRefs",
        "scope",
        "sourceAuthorityKind",
        "sourceAuthorityRef",
        "sourceHash",
        "sourceRef",
        "supersedes",
        "version",
        "visibility",
    ];
    const PROMOTION: [&str; 4] = [
        "sourceVersionRef",
        "sourceGrantRef",
        "targetGrantRef",
        "provenanceRefs",
    ];
    let fields = authority_fields(line)?;
    let allowed = REQUIRED
        .iter()
        .chain(PROMOTION.iter())
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    if fields.keys().any(|key| !allowed.contains(key.as_str())) {
        return Err(OrchestrationError::Invalid("unknown context field"));
    }
    for required in REQUIRED {
        if !fields.contains_key(required) {
            return Err(OrchestrationError::Invalid("missing context field"));
        }
    }
    let promotion_count = PROMOTION
        .iter()
        .filter(|key| fields.contains_key(**key))
        .count();
    if promotion_count != 0 && promotion_count != PROMOTION.len() {
        return Err(OrchestrationError::Invalid("partial promotion fields"));
    }
    Ok(fields)
}

fn action_fields(
    line: &str,
    expected: &[&str],
) -> Result<BTreeMap<String, String>, OrchestrationError> {
    let fields = authority_fields(line)?;
    if fields.len() != expected.len()
        || fields
            .keys()
            .any(|key| !expected.contains(&key.as_str()))
        || expected.iter().any(|key| !fields.contains_key(*key))
    {
        return Err(OrchestrationError::Invalid("action frame fields"));
    }
    Ok(fields)
}

fn split_list(value: Option<&String>) -> Vec<String> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| value.split(',').map(str::to_owned).collect())
        .unwrap_or_default()
}

fn string_fields(line: &str) -> Result<BTreeMap<String, String>, OrchestrationError> {
    let mut fields = BTreeMap::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
        let key = rest[..end].to_owned();
        rest = &rest[end + 1..];
        let trimmed = rest.trim_start();
        if !trimmed.starts_with(':') {
            continue;
        }
        let after = trimmed[1..].trim_start();
        if after.starts_with('"') {
            let body = &after[1..];
            if let Some(close) = body.find('"') {
                fields.insert(key, body[..close].to_owned());
            }
        } else if after.starts_with("true") {
            fields.insert(key, "true".into());
        } else if after.starts_with("false") {
            fields.insert(key, "false".into());
        } else {
            let number: String = after
                .chars()
                .take_while(|ch| ch.is_ascii_digit() || *ch == '-')
                .collect();
            if !number.is_empty() {
                fields.insert(key, number);
            }
        }
    }
    Ok(fields)
}

fn parse_orchestration_command(line: &str) -> Result<OrchestrationCommand, OrchestrationError> {
    let fields = string_fields(line)?;
    if fields.contains_key("accessAdmitted") {
        return Err(OrchestrationError::Invalid("caller admission field"));
    }
    let events = parse_events(line)?;
    Ok(OrchestrationCommand {
        command_id: required(&fields, "commandId")?.to_owned(),
        command_type: required(&fields, "commandType")?.to_owned(),
        expected_stream_version: match fields.get("expectedStreamVersion") {
            None => None,
            Some(value) if value == "null" => None,
            Some(value) => Some(parse_i64(value)?),
        },
        // Only handle_line on an authenticated service channel can reach this
        // parser for mutation. Caller-provided admission booleans are forbidden.
        access_admitted: true,
        events,
    })
}

fn parse_events(line: &str) -> Result<Vec<EventProposal>, OrchestrationError> {
    let marker = "\"events\":";
    let start = line
        .find(marker)
        .ok_or(OrchestrationError::Invalid("events"))?
        + marker.len();
    let slice = line[start..].trim_start();
    if !slice.starts_with('[') {
        return Err(OrchestrationError::Invalid("events"));
    }
    let mut events = Vec::new();
    let mut rest = &slice[1..];
    loop {
        rest = rest.trim_start();
        if rest.starts_with(']') {
            break;
        }
        if rest.starts_with(',') {
            rest = rest[1..].trim_start();
            continue;
        }
        if !rest.starts_with('{') {
            return Err(OrchestrationError::Invalid("event"));
        }
        let end = rest
            .find('}')
            .ok_or(OrchestrationError::Invalid("event"))?;
        let object = &rest[..=end];
        events.push(event_from_object(object)?);
        rest = &rest[end + 1..];
    }
    if events.is_empty() {
        return Err(OrchestrationError::Invalid("events"));
    }
    Ok(events)
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use crate::store::action::{reserve_action, ActionReservation};
    use crate::root::RootLock;
    use crate::store::same_open::route_b_test_guard;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn cognition_codec_rejects_missing_typed_revision_fields() {
        let frame = r#"{"domainId":"d","operation":"ReadObjectiveOutcome","outcomeId":"o","revision":"1"}"#;
        assert!(action_fields(frame, &["domainId","operation","outcomeId","revision"]).is_ok());
        assert!(action_fields(frame, &["domainId","operation","outcomeId"]).is_err());
    }

    fn action_frame(operation:&str,digest:&str,reservation:&str,session:&str,execution:&str,generation:&str)->String{
        let bound=format!("sha256:{}","a".repeat(64));
        format!("{{\"actionKind\":\"queue\",\"authRevision\":\"7\",\"bindingId\":\"binding-1\",\"childCeilingDigest\":\"{bound}\",\"childExecutionId\":\"{execution}\",\"childGeneration\":\"{generation}\",\"childSessionId\":\"{session}\",\"executionId\":\"{execution}\",\"generation\":\"{generation}\",\"instructionDigest\":\"{bound}\",\"lane\":\"work\",\"materialSetDigest\":\"{bound}\",\"operation\":\"{operation}\",\"operationId\":\"opr_11111111111111111111111111111111\",\"packageDigest\":\"{bound}\",\"parentCeilingDigest\":\"{bound}\",\"parentGrantRef\":\"grant-one\",\"parentGrantRevision\":\"1\",\"payloadHex\":\"7b7d\",\"policyAction\":\"delegate\",\"profileId\":\"profile-1\",\"reservationId\":\"{reservation}\",\"route\":\"controller-worker\",\"runtimeInstanceId\":\"runtime-1\",\"semanticDigest\":\"{digest}\",\"sessionId\":\"{session}\",\"sink\":\"task-package\",\"sourceDomainId\":\"domain-source\",\"sourceExecutionId\":\"source-execution\",\"sourceGeneration\":\"1\",\"sourcePrincipalId\":\"principal-source\",\"sourceProjectId\":\"project-one\",\"sourceRole\":\"controller\",\"sourceSessionId\":\"source-session\",\"targetDomainId\":\"domain-target\",\"targetPrincipalId\":\"principal-target\",\"targetProjectId\":\"project-one\",\"targetRole\":\"worker\"}}")
    }

    #[test]
    fn context_frame_commits_and_reconciles_on_the_same_open_connection() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-context-frame-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = open_product_database(&root, &database).unwrap();
        let frame = format!(
            "{{\"accessPolicyRevision\":\"1\",\"contentHash\":\"sha256:{}\",\"contextId\":\"context-one\",\"derivedFrom\":\"\",\"domainId\":\"domain-one\",\"kind\":\"fact\",\"operation\":\"CommitContextVersion\",\"operationId\":\"operation-one\",\"readGrantRefs\":\"\",\"scope\":\"PROJECT\",\"sourceAuthorityKind\":\"repository\",\"sourceAuthorityRef\":\"authority://one\",\"sourceHash\":\"sha256:{}\",\"sourceRef\":\"source://one\",\"supersedes\":\"\",\"version\":\"1\",\"visibility\":\"OWNER_PRIVATE\"}}",
            "a".repeat(64),
            "b".repeat(64)
        );
        assert!(handle_line(&mut connection, &frame).unwrap().contains("COMMITTED"));
        assert!(handle_line(&mut connection, &frame).unwrap().contains("RECONCILED"));
        let nested_override = frame.replace(
            "\"version\":\"1\"",
            "\"version\":\"2\",\"extra\":{\"contextId\":\"inner\",\"visibility\":\"DOMAIN_GRANTED\"}",
        );
        assert!(handle_line(&mut connection, &nested_override).is_err());
        let numeric_version = frame.replace("\"version\":\"1\"", "\"version\":1.5");
        assert!(handle_line(&mut connection, &numeric_version).is_err());
        let unknown = frame.replace(
            "\"visibility\":\"OWNER_PRIVATE\"",
            "\"visibility\":\"OWNER_PRIVATE\",\"extra\":\"ignored\"",
        );
        assert!(handle_line(&mut connection, &unknown).is_err());
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn action_frames_reject_caller_authority_and_untrusted_completion_ingress() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-action-frame-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = open_product_database(&root, &database).unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        let legacy = action_frame("ReserveAction",&digest,"reservation-1","session-1","execution-1","11");
        assert!(handle_line(&mut connection, &legacy).is_err());
        let reserve = "{\"actionKind\":\"queue\",\"actionOperationId\":\"opr_11111111111111111111111111111111\",\"contextManifestId\":\"manifest-one\",\"domainId\":\"domain-one\",\"lane\":\"work\",\"operation\":\"ReserveAction\",\"packageOperationId\":\"package-one\",\"parentGrantRef\":\"grant-one\",\"payload\":\"instruction\",\"recipeId\":\"recipe-one\",\"reservationId\":\"reservation-one\",\"sessionId\":\"session-one\",\"taskId\":\"task-one\"}";
        assert!(handle_line(&mut connection, reserve).is_err());
        let begin = "{\"domainId\":\"domain-one\",\"operation\":\"BeginActionCommitment\",\"operationId\":\"opr_11111111111111111111111111111111\",\"reservationId\":\"reservation-one\"}";
        assert!(handle_line(&mut connection, begin).is_err());
        let outcome = format!("{{\"detail\":\"EOF:lost\",\"operation\":\"RecordActionOutcome\",\"operationId\":\"opr_11111111111111111111111111111111\",\"receiptRef\":\"\",\"reservationId\":\"reservation-one\",\"semanticDigest\":\"{digest}\",\"state\":\"outcome-unknown\"}}");
        assert!(handle_line(&mut connection, &outcome).is_err());
        let count = Statement::prepare(connection.as_ptr(), "SELECT count(*) FROM main.gogoke_action_reservations").unwrap();
        assert!(count.step_row().unwrap());
        assert_eq!(count.column_text(0).unwrap(), "0");
        drop(count);
        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn current_delegation_grant_ipc_reads_native_head_and_rejects_caller_state() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-delegation-frame-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = open_product_database(&root, &database).unwrap();
        let owner = authority::initialize_profile(&mut connection, &root).unwrap();
        let input = || authority::DelegationGrantInput {
            principal: authority::DelegationPrincipal {
                principal_id: owner.principal_id().to_owned(), project_id: "project-one".into(),
                domain_id: "domain-one".into(), role: "controller".into(), seat_id: owner.seat_id().to_owned(),
            },
            binding: authority::DelegationBinding {
                session_id: "session-one".into(), execution_id: "execution-one".into(), generation: "7".into(),
            },
            expires_at_epoch_ms: 4_102_444_800_000,
            ceiling: authority::AuthorityCeiling {
                allowed_actions: vec!["delegate".into()], allowed_target_principal_ids: vec!["worker-one".into()],
                allowed_target_domain_ids: vec!["domain-one".into()], allowed_sinks: vec!["task-package".into()],
                allowed_material_classes: vec!["task-context".into()], explicit_private_material_ids: vec!["private-one".into()],
                allowed_continuation_responses: vec!["continue".into()], max_material_items: 8,
                max_material_bytes: 4096, max_response_bytes: 2048,
            },
        };
        let issued = authority::issue_owner_delegation(&mut connection, &owner, input()).unwrap();
        let grant_id = issued.reference.grant_id.clone();
        authority::revise_owner_delegation(
            &mut connection, &owner,
            &authority::DelegationGrantIdentity { grant_id: grant_id.clone(), revision: issued.reference.revision },
            input(),
        ).unwrap();

        let frame = format!("{{\"grantRef\":{},\"operation\":\"ReadCurrentDelegationGrant\"}}", json_quote(&grant_id));
        let body = handle_authenticated_line(&mut connection, &owner, &frame).unwrap();
        for field in ["binding", "ceiling", "expiresAtEpochMs", "grantRef", "issuerId", "parentGrant",
            "policyRevision", "principal", "revision", "revocationHead", "seatId"] {
            assert!(body.contains(&format!("\"{field}\":")), "missing {field}: {body}");
        }
        assert!(body.contains("\"revision\":\"2\""), "must read current head: {body}");
        assert!(body.contains("\"revocationHead\":\"0\""));
        assert!(body.contains("\"policyRevision\":\"1\""));
        assert!(body.contains("\"maxMaterialItems\":\"8\""));
        let caller_supplied_revision = frame.replace(
            "\"operation\":\"ReadCurrentDelegationGrant\"",
            "\"operation\":\"ReadCurrentDelegationGrant\",\"revision\":\"1\"",
        );
        assert!(handle_authenticated_line(&mut connection, &owner, &caller_supplied_revision).is_err());
        let missing = frame.replace(&grant_id, "grant-missing");
        assert!(handle_authenticated_line(&mut connection, &owner, &missing).is_err());
        assert!(matches!(handle_unprivileged_line(&mut connection, &frame), Err(OrchestrationError::AccessDenied)));

        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }

    #[test]
    fn authenticated_manifest_snapshot_is_exact_and_record_codec_counts_utf8_bytes() {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-manifest-frame-{nonce}"));
        std::fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = open_product_database(&root, &database).unwrap();
        let owner = authority::initialize_profile(&mut connection, &root).unwrap();
        super::super::atomic::initialize_product_core_schema(&mut connection).unwrap();
        authority::initialize_task_context_schema(&mut connection).unwrap();
        let empty_mandatory = encode_string_records(TASK_MANDATORY_CONTEXT_REFS_V1, &[]);
        let task_frame = format!(
            "{{\"domainId\":\"domain-one\",\"eventId\":\"task-event\",\"expectedPreviousTaskRevision\":\"\",\"mandatoryRefs\":{},\"operation\":\"CommitTaskContextRequirements\",\"operationId\":\"task-create\",\"receiptId\":\"task-receipt\",\"recordedAt\":\"2026-09-21T00:00:00Z\",\"taskId\":\"task-one\"}}",
            json_quote(&empty_mandatory),
        );
        let task_reply = handle_authenticated_line(&mut connection, &owner, &task_frame).unwrap();
        assert!(task_reply.contains("\"disposition\":\"COMMITTED\""));
        assert!(task_reply.contains("\"taskRevision\":\"1\""));
        assert!(handle_authenticated_line(&mut connection, &owner, &task_frame).unwrap().contains("\"disposition\":\"RECONCILED\""));
        let read_task = "{\"domainId\":\"domain-one\",\"operation\":\"ReadTaskContextRequirements\",\"taskId\":\"task-one\"}";
        assert!(handle_authenticated_line(&mut connection, &owner, read_task).unwrap().contains(&json_quote(&empty_mandatory)));
        assert!(matches!(handle_unprivileged_line(&mut connection, read_task), Err(OrchestrationError::AccessDenied)));
        let grant = authority::issue_owner_grant(
            &mut connection,
            &owner,
            "1",
            "0",
            authority::GrantSpec {
                principal_id: "principal-one".into(),
                seat_id: "seat-one".into(),
                permission: "context.read".into(),
                promotion_kind: "PROJECT_ONLY".into(),
                source_domain_id: "domain-source".into(),
                destination_domain_id: "domain-one".into(),
                destination_scope: "PROJECT".into(),
                delegable_depth: 0,
            },
        )
        .unwrap();
        let digest = format!("sha256:{}", "c".repeat(64));
        reserve_action(
            &mut connection,
            ActionReservation {
                operation_id: "opr_11111111111111111111111111111111".into(),
                semantic_digest: digest.clone(),
                reservation_id: "manifest-action-reservation".into(),
                binding_id: "binding-one".into(),
                session_id: "session-one".into(),
                execution_id: "execution-one".into(),
                runtime_instance_id: "runtime-one".into(),
                profile_id: "profile-one".into(),
                auth_revision: "2".into(),
                generation: "7".into(),
                lane: "work".into(),
                action_kind: "queue".into(),
                payload_hex: "7b7d".into(),
                commitment: crate::store::action::test_commitment("session-one", "execution-one", "7"),
            },
        )
        .unwrap();
        let partition_bindings = encode_string_records(
            ASSEMBLY_PARTITION_BINDINGS_V1,
            &[vec![
                "domain-source".into(), "PROJECT".into(), "PROJECT_ONLY".into(),
                grant.grant_id, grant.revision, grant.revocation_head,
            ]],
        );
        let frame = format!(
            "{{\"admissionActionOperationId\":\"opr_11111111111111111111111111111111\",\"admissionDigest\":\"{digest}\",\"authRevision\":\"2\",\"bindingGeneration\":\"7\",\"bindingId\":\"binding-one\",\"domainId\":\"domain-one\",\"manifestId\":\"manifest-one\",\"maxCandidates\":\"8\",\"maxContentBytes\":\"4096\",\"operation\":\"PublishContextAssemblySnapshot\",\"operationId\":\"manifest-operation\",\"partitionBindings\":\"{partition_bindings}\",\"policyRevision\":\"1\",\"principalId\":\"principal-one\",\"revocationHead\":\"0\",\"runtimeInstanceId\":\"runtime-one\",\"seatId\":\"seat-one\",\"selectionDecisionId\":\"decision-one\",\"sessionId\":\"session-one\",\"sourceEpoch\":\"9\",\"taskId\":\"task-one\",\"taskRevision\":\"1\"}}"
        );
        assert_eq!(
            handle_authenticated_line(&mut connection, &owner, &frame).unwrap(),
            "{\"published\":true}"
        );
        assert_eq!(
            handle_authenticated_line(&mut connection, &owner, &frame).unwrap(),
            "{\"published\":true}"
        );
        let identity_frame = |operation: &str| format!(
            "{{\"bindingGeneration\":\"7\",\"bindingId\":\"binding-one\",\"domainId\":\"domain-one\",\"operation\":\"{operation}\",\"operationId\":\"manifest-operation\",\"principalId\":\"principal-one\",\"runtimeInstanceId\":\"runtime-one\",\"seatId\":\"seat-one\",\"sessionId\":\"session-one\",\"sourceEpoch\":\"9\",\"taskId\":\"task-one\"}}"
        );
        let basis_frame = identity_frame("ReadContextAssemblyBasis");
        assert_eq!(
            handle_authenticated_line(&mut connection, &owner, &basis_frame).unwrap(),
            format!("{{\"authRevision\":\"2\",\"bindingGeneration\":\"7\",\"mandatoryRefs\":{},\"maxCandidates\":\"8\",\"maxContentBytes\":\"4096\",\"operationId\":\"manifest-operation\",\"partitionBindings\":{},\"policyRevision\":\"1\",\"revocationHead\":\"0\",\"sourceEpoch\":\"9\",\"taskRevision\":\"1\"}}", json_quote(&empty_mandatory), json_quote(&partition_bindings))
        );
        let list_frame = identity_frame("ListContextAssemblySources");
        assert_eq!(
            handle_authenticated_line(&mut connection, &owner, &list_frame).unwrap(),
            "{\"operationId\":\"manifest-operation\",\"sources\":\"gogoke.context-assembly-sources.v1|0|\"}"
        );
        for operation_frame in [&basis_frame, &list_frame] {
            let extra_field = operation_frame.replace("\"taskId\":\"task-one\"", "\"taskId\":\"task-one\",\"allowed\":\"true\"");
            assert!(handle_authenticated_line(&mut connection, &owner, &extra_field).is_err());
            assert!(matches!(handle_unprivileged_line(&mut connection, operation_frame), Err(OrchestrationError::AccessDenied)));
        }
        let changed = frame.replace("\"maxCandidates\":\"8\"", "\"maxCandidates\":\"9\"");
        assert!(matches!(
            handle_authenticated_line(&mut connection, &owner, &changed),
            Err(OrchestrationError::OperationConflict)
        ));
        let extra = frame.replace(
            "\"taskRevision\":\"1\"",
            "\"taskRevision\":\"1\",\"extra\":\"forbidden\"",
        );
        assert!(handle_authenticated_line(&mut connection, &owner, &extra).is_err());
        assert!(handle_authenticated_line(
            &mut connection, &owner,
            "{\"operation\":\"ReadGranteeContextSet\",\"readRequests\":\"broken\"}"
        )
        .is_err());

        let encoded = encode_string_records("utf8.v1", &[vec!["é".into(), "汉".into()]]);
        assert_eq!(encoded, "utf8.v1|1|2:é3:汉");
        assert_eq!(
            decode_string_records(&encoded, "utf8.v1", 2, false).unwrap(),
            vec![vec!["é".to_owned(), "汉".to_owned()]]
        );
        let task_utf8 = encode_string_records(TASK_MANDATORY_CONTEXT_REFS_V1, &[vec!["域".into(), "上下文".into(), "1".into()]]);
        assert_eq!(decode_string_records(&task_utf8, TASK_MANDATORY_CONTEXT_REFS_V1, 3, true).unwrap()[0][1], "上下文");
        assert!(decode_string_records("gogoke.task-mandatory-context-refs.v1|1|1:a1:b", TASK_MANDATORY_CONTEXT_REFS_V1, 3, true).is_err());
        assert!(decode_string_records("gogoke.task-mandatory-context-refs.v1|0|x", TASK_MANDATORY_CONTEXT_REFS_V1, 3, true).is_err());

        connection.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).ok();
        std::fs::remove_dir(root_path).ok();
    }
}

fn event_from_object(object: &str) -> Result<EventProposal, OrchestrationError> {
    let fields = string_fields(object)?;
    match required(&fields, "type")? {
        "project.created" => Ok(EventProposal::ProjectCreated {
            event_id: required(&fields, "eventId")?.into(),
            project_id: required(&fields, "projectId")?.into(),
            title: required(&fields, "title")?.into(),
            workspace_root: required(&fields, "workspaceRoot")?.into(),
            occurred_at: required(&fields, "occurredAt")?.into(),
        }),
        "thread.created" => Ok(EventProposal::ThreadCreated {
            event_id: required(&fields, "eventId")?.into(),
            thread_id: required(&fields, "threadId")?.into(),
            project_id: required(&fields, "projectId")?.into(),
            title: required(&fields, "title")?.into(),
            model: required(&fields, "model")?.into(),
            occurred_at: required(&fields, "occurredAt")?.into(),
        }),
        "thread.message-sent" => Ok(EventProposal::MessageSent {
            event_id: required(&fields, "eventId")?.into(),
            message_id: required(&fields, "messageId")?.into(),
            thread_id: required(&fields, "threadId")?.into(),
            role: required(&fields, "role")?.into(),
            text: required(&fields, "text")?.into(),
            occurred_at: required(&fields, "occurredAt")?.into(),
        }),
        _ => Err(OrchestrationError::Invalid("event type")),
    }
}
