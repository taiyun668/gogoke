//! One authenticated, original-intent migration of a proven legacy Codex SOURCE.
//! This F journal never advances an old instance operation or H/C outcome.

use super::{credential_registry, legacy_fence, managed_cli, registry, resolve_codex_instance_home};
use crate::process::{CredentialBinding, LegacyAclInventory, NativeLegacyHoldersGone};
use crate::root::RootLock;
use crate::store::atomic::{Json, JsonString, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::digest::sha256_hex;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
use std::collections::BTreeSet;

pub(super) const SCHEMA: &str = "CREATE TABLE gogoke_v37_instance_legacy_source_migrations(request_id TEXT PRIMARY KEY,target_id TEXT NOT NULL,raw_hex TEXT NOT NULL,raw_sha256 TEXT NOT NULL,original_request_id TEXT NOT NULL,original_fingerprint TEXT NOT NULL,original_receipt_hex TEXT NOT NULL,original_home_identity TEXT NOT NULL,root_identity TEXT NOT NULL,database_identity TEXT NOT NULL,qualification_digest TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('PREPARING','APPLIED','UNKNOWN')),source_fingerprint TEXT,receipt_hex TEXT,CHECK((phase='PREPARING' AND source_fingerprint IS NULL AND receipt_hex IS NULL) OR (phase='UNKNOWN' AND source_fingerprint IS NULL AND receipt_hex IS NULL) OR (phase='APPLIED' AND source_fingerprint IS NOT NULL AND receipt_hex IS NOT NULL))) STRICT";

pub(crate) struct LegacySourceRequest<'a> {
    pub(crate) raw: &'a [u8],
    pub(crate) request_id: &'a str,
    pub(crate) target_id: &'a str,
    pub(crate) original_request_id: &'a str,
    pub(crate) original_fingerprint: &'a str,
    pub(crate) original_receipt_hex: &'a str,
    pub(crate) original_home_identity: &'a str,
}

pub(crate) struct LegacySourceResult {
    pub(crate) receipt: Vec<u8>,
}

struct Qualification {
    stage: String, digest: String, version: String, home: String,
    source_fingerprint: String, qualification_digest: String,
}

struct Journal {
    target: String, raw_hex: String, raw_sha256: String,
    original_request: String, original_fingerprint: String,
    original_receipt_hex: String, original_home: String,
    root: String, database: String, qualification_digest: String,
    phase: String, source_fingerprint: String, receipt_hex: String,
}

type Result<T> = std::result::Result<T, OrchestrationError>;
fn fail(context: &str, error: impl std::fmt::Debug) -> OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("legacy SOURCE {context}: {error:?}"))
}
fn denied() -> OrchestrationError { OrchestrationError::AccessDenied }

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes { out.push(DIGITS[(byte >> 4) as usize] as char); out.push(DIGITS[(byte & 15) as usize] as char); }
    out
}
fn unhex(value: &str) -> Result<Vec<u8>> {
    if value.is_empty() || value.len() > 131_072 || value.len() % 2 != 0 ||
        !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err(denied());
    }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let digit = |byte: u8| if byte.is_ascii_digit() { byte - b'0' } else { byte - b'a' + 10 };
        Ok((digit(pair[0]) << 4) | digit(pair[1]))
    }).collect()
}
fn unframe(value: &str) -> Result<Vec<Vec<u8>>> {
    let bytes=unhex(value)?;
    let mut fields=Vec::new();
    let mut offset=0usize;
    while offset<bytes.len() {
        if bytes.len()-offset<8 || fields.len()>=16 { return Err(denied()); }
        let length=u64::from_be_bytes(bytes[offset..offset+8].try_into().map_err(|_|denied())?);
        offset+=8;
        let length=usize::try_from(length).map_err(|_|denied())?;
        let end=offset.checked_add(length).ok_or_else(denied)?;
        if end>bytes.len() { return Err(denied()); }
        fields.push(bytes[offset..end].to_vec());
        offset=end;
    }
    Ok(fields)
}
fn frame(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value);
}
fn valid_atom(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
fn valid_request(input: &LegacySourceRequest<'_>) -> Result<()> {
    if input.raw.is_empty() || input.raw.len() > 65_536 ||
        !valid_atom(input.request_id) || !registry::valid_id(input.target_id) ||
        !valid_atom(input.original_request_id) || input.request_id == input.original_request_id ||
        input.original_fingerprint.len() > 262_144 ||
        input.original_fingerprint.is_empty() || input.original_fingerprint.len() % 2 != 0 ||
        !input.original_fingerprint.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) ||
        input.original_receipt_hex.len() > 1024 || unhex(input.original_receipt_hex).is_err() ||
        input.original_home_identity.is_empty() || input.original_home_identity.len() > 256 {
        return Err(denied());
    }
    Ok(())
}

fn rows(db: &VerifiedDatabaseConnection<'_>, sql: &str, values: &[&str], columns: i32)
    -> Result<Vec<Vec<String>>> {
    let statement = Statement::prepare(db.as_ptr(), sql)?;
    for (index, value) in values.iter().enumerate() { statement.bind_text((index + 1) as i32, value)?; }
    let mut found = Vec::new();
    while statement.step_row()? {
        if found.len() >= 200_000 { return Err(denied()); }
        found.push((0..columns).map(|index| statement.column_text(index))
            .collect::<std::result::Result<Vec<_>, _>>()?);
    }
    Ok(found)
}
fn any(db: &VerifiedDatabaseConnection<'_>, sql: &str, values: &[&str]) -> Result<bool> {
    Ok(!rows(db, sql, values, 1)?.is_empty())
}

/// Canonical, bounded readback of every native gogoke table, including all
/// affected H/C/F revisions. The migration table is omitted because PREPARING
/// itself must not change the qualification fingerprint. All selected names
/// come from the verified main schema and are checked before SQL construction.
fn database_snapshot(db: &VerifiedDatabaseConnection<'_>) -> Result<String> {
    let tables = rows(db,
        "SELECT name,sql FROM main.sqlite_schema WHERE type='table' AND name LIKE 'gogoke%' ORDER BY name", &[], 2)?;
    if tables.is_empty() { return Err(denied()); }
    let mut bytes = Vec::new();
    for table in tables {
        let name = &table[0];
        if name == "gogoke_v37_instance_legacy_source_migrations" { continue; }
        if !name.starts_with("gogoke_") || !name.bytes().all(|byte|
            byte.is_ascii_alphanumeric() || byte == b'_') { return Err(denied()); }
        frame(&mut bytes, name.as_bytes()); frame(&mut bytes, table[1].as_bytes());
        let columns = rows(db, &format!("PRAGMA main.table_info(\"{name}\")"), &[], 2)?;
        if columns.is_empty() || columns.len() > 128 { return Err(denied()); }
        let mut expressions = Vec::new();
        for column in &columns {
            let name = &column[1];
            if !name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
                return Err(denied());
            }
            expressions.push(format!("typeof(\"{name}\")||':'||hex(CAST(\"{name}\" AS BLOB))"));
        }
        let sql = format!("SELECT {} FROM main.\"{name}\"", expressions.join(","));
        let mut selected = rows(db, &sql, &[], columns.len() as i32)?;
        selected.sort();
        for row in selected {
            frame(&mut bytes, b"row");
            for field in row { frame(&mut bytes, field.as_bytes()); }
            if bytes.len() > 64 * 1024 * 1024 { return Err(denied()); }
        }
    }
    Ok(sha256_hex(&bytes))
}

fn source_fingerprint(id: &str, stage: &str, digest: &str, version: &str,
    home: &str, registration: &str) -> String {
    let mut bytes = Vec::new();
    for value in [id, "MANAGED", stage, digest, version, home, registration, "1"] {
        frame(&mut bytes, value.as_bytes());
    }
    sha256_hex(&bytes)
}
fn source_rows(db: &VerifiedDatabaseConnection<'_>, id: &str) -> Result<Vec<Vec<String>>> {
    rows(db, "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id,CAST(revision AS TEXT) FROM main.gogoke_v37_instance_program_sources WHERE instance_id=?1", &[id], 7)
}

fn read_journal(db: &VerifiedDatabaseConnection<'_>, request: &str) -> Result<Option<Journal>> {
    let result = rows(db, "SELECT target_id,raw_hex,raw_sha256,original_request_id,original_fingerprint,original_receipt_hex,original_home_identity,root_identity,database_identity,qualification_digest,phase,COALESCE(source_fingerprint,''),COALESCE(receipt_hex,'') FROM main.gogoke_v37_instance_legacy_source_migrations WHERE request_id=?1", &[request], 13)?;
    if result.len() > 1 { return Err(denied()); }
    Ok(result.into_iter().next().map(|v| Journal {
        target:v[0].clone(),raw_hex:v[1].clone(),raw_sha256:v[2].clone(),
        original_request:v[3].clone(),original_fingerprint:v[4].clone(),
        original_receipt_hex:v[5].clone(),original_home:v[6].clone(),
        root:v[7].clone(),database:v[8].clone(),qualification_digest:v[9].clone(),
        phase:v[10].clone(),source_fingerprint:v[11].clone(),receipt_hex:v[12].clone(),
    }))
}
fn same_intent(db: &VerifiedDatabaseConnection<'_>, input: &LegacySourceRequest<'_>,
    journal: &Journal) -> bool {
    journal.target == input.target_id && journal.raw_hex == hex(input.raw) &&
    journal.raw_sha256 == sha256_hex(input.raw) &&
    journal.original_request == input.original_request_id &&
    journal.original_fingerprint == input.original_fingerprint &&
    journal.original_receipt_hex == input.original_receipt_hex &&
    journal.original_home == input.original_home_identity &&
    journal.root == db.root_identity().opaque() && journal.database == db.identity().opaque()
}
fn applied_readback(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &LegacySourceRequest<'_>, journal: &Journal,
    expected_receipt: &[u8]) -> Result<LegacySourceResult> {
    if !same_intent(db, input, journal) || journal.phase != "APPLIED" ||
        journal.qualification_digest.is_empty() { return Err(denied()); }
    if any(db, "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE request_id=?1 LIMIT 1",
        &[input.request_id])? || any(db,
        "SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE request_id=?1 LIMIT 1",
        &[input.request_id])? { return Err(denied()); }
    let source = source_rows(db, input.target_id)?;
    if source.len() != 1 || source[0][0] != "MANAGED" ||
        source[0][4] != input.original_home_identity ||
        source[0][5] != input.original_request_id || source[0][6] != "1" ||
        journal.receipt_hex != hex(expected_receipt) ||
        source_fingerprint(input.target_id, &source[0][1], &source[0][2], &source[0][3],
            &source[0][4], &source[0][5]) != journal.source_fingerprint ||
        registry::observed_home(root, input.target_id)
            .map_err(|error| fail("replay HOME", error))?
            .is_none_or(|home| home.opaque() != input.original_home_identity) {
        return Err(denied());
    }
    let instance = rows(db, "SELECT driver_id,program_digest,version,home_identity FROM main.gogoke_v37_instances WHERE instance_id=?1", &[input.target_id], 4)?;
    if instance.len() != 1 || instance[0][0] != "codex" ||
        instance[0][1] != source[0][2] || instance[0][2] != source[0][3] ||
        instance[0][3] != source[0][4] { return Err(denied()); }
    let receipt = unhex(&journal.receipt_hex)?;
    Ok(LegacySourceResult { receipt })
}

fn request_field(request: &crate::store::session_transport::V37Request,
    name: &str) -> Option<String> {
    match request.payload.get(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string(), _ => None,
    }
}

/// STOPPED is a durable H/C outcome, but its strings are meaningful only after
/// the original execution producer, HOME, binding, generation and operation
/// have been independently re-associated. A table digest alone cannot do it.
fn qualified_stopped_h(db: &VerifiedDatabaseConnection<'_>, operation: &str,
    instance: &str, domain: &str, generation: &str, proof: &str) -> Result<()> {
    let original = rows(db,
        "SELECT e.request_id,e.session_id,e.generation,COALESCE(e.old_generation,''),e.raw_hex,CAST(e.previous_revision AS TEXT),COALESCE(CAST(e.result_revision AS TEXT),''),e.home_id,e.binding_id,COALESCE(e.seat_id,''),COALESCE(e.seat_incarnation,''),COALESCE(e.stop_request_id,''),e.phase,COALESCE(e.stop_fact_id,''),c.ticket,c.custodian_nonce,c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.state,COALESCE(c.stop_proof_hash,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.process_operation_id=?1 AND e.instance_id=?2 AND e.domain_id=?3", &[operation,instance,domain], 22)?;
    if original.len()!=1 { return Err(denied()); }
    let e=&original[0];
    let pid=e[16].parse::<u32>().map_err(|error|fail("STOPPED C PID",error))?;
    let created=e[17].parse::<u64>().map_err(|error|fail("STOPPED C FILETIME",error))?;
    if e[2]!=generation || e[12]!="STOPPED" || e[13]!=proof ||
        e[20]!="STOPPED" || e[21]!=proof || e[14].is_empty() || e[15].is_empty() ||
        pid==0 || created==0 || e[16]!=pid.to_string() || e[17]!=created.to_string() ||
        e[18].is_empty() || !e[19].starts_with("sha256:") || e[19].len()!=71 ||
        e[9].is_empty() || e[10].is_empty() { return Err(denied()); }
    let raw=unhex(&e[4])?;
    let request=crate::store::session_transport::decode_request(&raw)
        .map_err(|error|fail("original STOPPED H raw",error))?;
    let resume=matches!(request.operation.as_str(),"resume"|"compact"|"renew-session");
    let kind=if resume{"resume"}else{"open"};
    let before=e[5].parse::<u64>().map_err(|error|fail("H previous revision",error))?;
    let after=e[6].parse::<u64>().map_err(|error|fail("H result revision",error))?;
    if request.family!="K-SESSION" || !matches!(request.operation.as_str(),
        "open"|"resume"|"compact"|"renew-session") ||
        request.request_id!=e[0] || request.target_id!=e[1] ||
        request.domain_id!=domain || before.checked_add(1)!=Some(after) ||
        operation!=format!("h-{kind}-{}",&sha256_hex(&raw)[..40]) { return Err(denied()); }
    if resume {
        let old=e[3].parse::<u64>().map_err(|error|fail("H old generation",error))?;
        let current=generation.parse::<u64>().map_err(|error|fail("H generation",error))?;
        if old.checked_add(1)!=Some(current) || request.payload.len()!=1 ||
            request_field(&request,"generation").as_deref()!=Some(e[3].as_str()) ||
            any(db,"SELECT 1 FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 LIMIT 1",
                &[domain,&e[0]])? { return Err(denied()); }
        let predecessor=rows(db,
            "SELECT p.process_operation_id,c.ticket,c.custodian_nonce FROM main.gogoke_v37_h_process_episode p JOIN main.gogoke_coordination_process_custody c ON c.operation_id=p.process_operation_id AND c.profile_id=p.instance_id AND c.domain_id=p.domain_id AND c.generation=p.generation WHERE p.instance_id=?1 AND p.domain_id=?2 AND p.session_id=?3 AND p.generation=?4 AND p.phase='STOPPED' AND p.stop_fact_id IS NOT NULL AND c.state='STOPPED' AND c.stop_proof_hash=p.stop_fact_id", &[instance,domain,&e[1],&e[3]],3)?;
        if predecessor.len()!=1 { return Err(denied()); }
        if request.operation=="compact" || request.operation=="renew-session" {
            let change=rows(db,
                "SELECT raw_hex,operation,old_generation,old_process_operation_id,old_ticket,old_nonce,stage,COALESCE(owner_stop_request_id,''),COALESCE(CAST(result_revision AS TEXT),''),COALESCE(CAST(unknown_revision AS TEXT),'') FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND request_id=?2 AND session_id=?3", &[domain,&e[0],&e[1]],10)?;
            if change.len()!=1 || change[0][0]!=e[4] || change[0][1]!=request.operation ||
                change[0][2]!=e[3] || change[0][3]!=predecessor[0][0] ||
                change[0][4]!=predecessor[0][1] || change[0][5]!=predecessor[0][2] ||
                change[0][6]!="APPLIED" || !change[0][7].is_empty() || change[0][8]!=e[6] ||
                !(request.expected_revision==before ||
                    request.expected_revision.checked_add(1)==Some(before) &&
                    change[0][9]==e[5]) { return Err(denied()); }
        } else if request.expected_revision!=before { return Err(denied()); }
    } else {
        if !e[3].is_empty() || request.expected_revision!=before { return Err(denied()); }
        let formal=request_field(&request,"purpose").as_deref()==Some("FORMAL_REVIEW");
        if domain=="global" {
            if !request.payload.is_empty() { return Err(denied()); }
            let selected=rows(db,
                "SELECT 1 FROM main.gogoke_v37_effective_seat s JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=s.domain_id AND d.seat_id=s.seat_id AND d.incarnation=s.seat_incarnation WHERE s.domain_id='global' AND s.session_id=?1 AND s.generation=?2 AND s.seat_id=?3 AND s.seat_incarnation=?4 AND s.selected_instance_id=?5",
                &[&e[1],generation,&e[9],&e[10],instance],1)?;
            if selected.len()!=1 { return Err(denied()); }
        } else if request.payload.len()!=if formal{5}else{4} ||
            request_field(&request,"generation").as_deref()!=Some(generation) ||
            request_field(&request,"seatId").as_deref()!=Some(e[9].as_str()) ||
            request_field(&request,"repositoryId").is_none_or(|value|value.is_empty()) ||
            request_field(&request,"worktreeId").is_none_or(|value|value.is_empty()) {
            return Err(denied());
        }
        let opened=rows(db,
            "SELECT raw_hex,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND operation='open'", &[domain,&e[0],&e[1]],4)?;
        if opened.len()!=1 || opened[0][0]!=e[4] || opened[0][1]!="APPLIED" ||
            opened[0][2]!=e[5] || opened[0][3]!=e[6] { return Err(denied()); }
    }
    if !any(db,"SELECT 1 FROM main.gogoke_v37_h_generation WHERE domain_id=?1 AND session_id=?2 AND generation=?3 AND request_id=?4 AND process_operation_id=?5",
        &[domain,&e[1],generation,&e[0],operation])? { return Err(denied()); }
    let home=rows(db,
        "SELECT directory_ref,directory_identity,state FROM main.gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5",
        &[&e[7],instance,domain,&e[1],generation],3)?;
    let expected_ref=format!("temp-home-{}",sha256_hex(e[7].as_bytes()));
    if home.len()!=1 || home[0][0]!=expected_ref ||
        !home[0][1].starts_with("volume:") || !home[0][1].contains("/file:") ||
        !matches!(home[0][2].as_str(),"ACTIVE"|"CLOSED"|"CLEANED") { return Err(denied()); }
    let creation=rows(db,
        "SELECT request_id,request_hex FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase='APPLIED' AND native_receipt_id=?2",
        &[&e[7],&home[0][1]],2)?;
    if creation.len()!=1 { return Err(denied()); }
    let fields=unframe(&creation[0][1])?;
    let preparation=sha256_hex(format!("{domain}\n{}",e[0]).as_bytes());
    let expected_request=format!("homeprep-{}",&preparation[..40]);
    if fields.len()!=9 || fields[0]!=b"temporary-create-detached" ||
        creation[0][0]!=expected_request || fields[1]!=raw ||
        fields[2]!=e[7].as_bytes() ||
        fields[3]!=instance.as_bytes() || fields[4]!=domain.as_bytes() ||
        fields[5]!=b"SESSION" || fields[6]!=e[1].as_bytes() ||
        fields[7]!=generation.as_bytes() ||
        !fields[8].starts_with(b"S-1-15-2-") { return Err(denied()); }
    let binding=rows(db,
        "SELECT state FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5",
        &[&e[8],instance,domain,&e[1],generation],1)?;
    if binding.len()!=1 || !matches!(binding[0][0].as_str(),"ACTIVE"|"REVOKED") { return Err(denied()); }
    if any(db,"SELECT 1 FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2 AND generation=?3 LIMIT 1",
        &[domain,&e[1],generation])? { return Err(denied()); }
    if !e[11].is_empty() {
        let stopped=rows(db,
            "SELECT raw_hex,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT) FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND operation='stop'", &[domain,&e[11],&e[1]],4)?;
        if stopped.len()!=1 || stopped[0][1]!="APPLIED" ||
            stopped[0][2].parse::<u64>().ok().and_then(|value|value.checked_add(1))
                .map(|value|value.to_string())!=Some(stopped[0][3].clone()) { return Err(denied()); }
        let stop=crate::store::session_transport::decode_request(&unhex(&stopped[0][0])?)
            .map_err(|error|fail("original STOP request",error))?;
        if stop.family!="K-SESSION" || stop.operation!="stop" ||
            stop.request_id!=e[11] || stop.target_id!=e[1] || stop.domain_id!=domain ||
            stop.expected_revision.to_string()!=stopped[0][2] || stop.payload.len()!=2 ||
            request_field(&stop,"generation").as_deref()!=Some(generation) ||
            request_field(&stop,"seatId").as_deref()!=Some(e[9].as_str()) { return Err(denied()); }
    }
    let current=rows(db,
        "SELECT generation,home_id,binding_id,COALESCE(process_operation_id,''),COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND instance_id=?3", &[domain,&e[1],instance],5)?;
    if current.len()>1 { return Err(denied()); }
    if let Some(claim)=current.first() {
        if claim[0]==generation && (claim[1]!=e[7] || claim[2]!=e[8] ||
            claim[3]!=operation || claim[4]!=proof) { return Err(denied()); }
    }
    Ok(())
}

/// All affected original C rows are read. STOPPED requires its original proof;
/// an unsettled H row needs H's unique original-source kernel fact; a global
/// historical UNKNOWN needs the native whole-boot fact over every original
/// global C identity, independently of any ACL fence.
fn qualified_executions(db: &VerifiedDatabaseConnection<'_>, evidence: &mut Vec<u8>) -> Result<()> {
    let custody = rows(db,
        "SELECT c.operation_id,c.profile_id,c.domain_id,c.generation,c.state,COALESCE(c.stop_proof_hash,''),c.pid,c.creation_time_100ns FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id WHERE i.driver_id='codex' ORDER BY c.operation_id", &[], 8)?;
    let mut seen_h = BTreeSet::new();
    let mut global_unknown = BTreeSet::new();
    for c in &custody {
        let (operation, instance, domain, generation, state, stop) =
            (&c[0], &c[1], &c[2], &c[3], &c[4], &c[5]);
        let episode = rows(db,
            "SELECT home_id,session_id,generation,binding_id,phase,COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_process_episode WHERE process_operation_id=?1 AND instance_id=?2 AND domain_id=?3", &[operation, instance, domain], 6)?;
        if episode.len() > 1 { return Err(denied()); }
        if state == "STOPPED" && !stop.is_empty() {
            if let Some(h) = episode.first() {
                if h[2] != *generation || h[4] != "STOPPED" || h[5] != *stop {
                    return Err(denied());
                }
                qualified_stopped_h(db, operation, instance, domain, generation, stop)?;
                seen_h.insert(operation.clone());
            } else if domain != "global" { return Err(denied()); }
            continue;
        }
        if state != "UNKNOWN" || !stop.is_empty() { return Err(denied()); }
        if episode.is_empty() && domain == "global" {
            global_unknown.insert(instance.clone()); continue;
        }
        if episode.len() != 1 || episode[0][2] != *generation ||
            !matches!(episode[0][4].as_str(), "ACTIVE" | "UNKNOWN") || !episode[0][5].is_empty() {
            return Err(denied());
        }
        let (fact, source_digest, pairs) = crate::store::session_transport::observe_exact_home_gone(
            db, &episode[0][0], instance, domain, &episode[0][1], generation, &episode[0][3])
            .map_err(|error| fail("original H execution", error))?;
        fact.validate(&pairs).map_err(|error| fail("H kernel recheck", error))?;
        seen_h.insert(operation.clone());
        for value in [source_digest.as_str(), &fact.boot_start().to_string()] {
            frame(evidence, value.as_bytes());
        }
        for (pid, creation) in pairs { frame(evidence, &pid.to_be_bytes()); frame(evidence, &creation.to_be_bytes()); }
    }
    for instance in global_unknown {
        let original = rows(db,
            "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND domain_id='global' ORDER BY operation_id",
            &[&instance], 12)?;
        let expected: Vec<_> = custody.iter().filter(|c| c[1] == instance && c[2] == "global").collect();
        if original.is_empty() || original.len() != expected.len() ||
            original.iter().filter(|c| c[10] == "UNKNOWN").count() !=
                expected.iter().filter(|c| c[4] == "UNKNOWN").count() { return Err(denied()); }
        let mut pairs = Vec::with_capacity(original.len());
        let mut source = Vec::new();
        for c in &original {
            let original_login_suffix=c[0].strip_prefix("owner-login-")
                .or_else(||c[0].strip_prefix("login-observe-"));
            if c[7] != instance || c[8] != "global" || c[0].is_empty() || c[1].is_empty()
                || c[2].is_empty() || c[5].is_empty() || c[6].len() != 71
                || !c[6].starts_with("sha256:") || !c[6][7..].bytes().all(|byte|byte.is_ascii_hexdigit())
                || c[9].parse::<u64>().ok().filter(|v| *v != 0).is_none()
                || !expected.iter().any(|row| row[0] == c[0] && row[3] == c[9]
                    && row[4] == c[10] && row[5] == c[11] && row[6] == c[3] && row[7] == c[4])
                || (c[10] == "UNKNOWN" && (!c[11].is_empty()
                    || original_login_suffix.is_none_or(|suffix|suffix.len()!=40
                        || !suffix.bytes().all(|byte|byte.is_ascii_hexdigit()))))
                || (c[10] == "STOPPED" && c[11].is_empty())
                || !matches!(c[10].as_str(), "STOPPED" | "UNKNOWN") { return Err(denied()); }
            let pid = c[3].parse::<u32>().map_err(|error| fail("global original PID", error))?;
            let created = c[4].parse::<u64>().map_err(|error| fail("global original FILETIME", error))?;
            if pid == 0 || created == 0 || c[3] != pid.to_string() || c[4] != created.to_string()
                || pairs.contains(&(pid, created)) { return Err(denied()); }
            pairs.push((pid, created));
            for field in c { frame(&mut source, field.as_bytes()); }
        }
        let fact = NativeLegacyHoldersGone::observe(&pairs)
            .map_err(|error| fail("global kernel fact", error))?;
        fact.validate(&pairs).map_err(|error| fail("global kernel recheck", error))?;
        frame(evidence, sha256_hex(&source).as_bytes());
        frame(evidence, &fact.boot_start().to_be_bytes());
        for (pid, created) in pairs { frame(evidence, &pid.to_be_bytes()); frame(evidence, &created.to_be_bytes()); }
    }
    // Every H episode with an original process must be accounted for by the
    // corresponding custody row above. A process-less COMMITTED claim cannot
    // be discharged by a native absence observation.
    let episodes = rows(db, "SELECT e.process_operation_id,e.phase,COALESCE(e.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id='codex' AND e.process_operation_id IS NOT NULL", &[], 3)?;
    for e in episodes {
        if !seen_h.contains(&e[0]) && !(e[1] == "FAILED" && e[2].is_empty()) {
            return Err(denied());
        }
    }
    Ok(())
}

/// A released reservation with no process is settled only by its original
/// reserve/commit/Owner-release history and a complete absence of launch
/// producers. An empty native pair list is never a holder-gone observation.
fn qualified_released_never_started(db: &VerifiedDatabaseConnection<'_>,
    instance: &str, domain: &str, session: &str, home: &str,
    binding: &str, generation: &str) -> Result<()> {
    let claims = rows(db,
        "SELECT state,CAST(revision AS TEXT),COALESCE(process_operation_id,''),COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND domain_id=?2 AND session_id=?3 AND home_id=?4 AND binding_id=?5 AND generation=?6",
        &[instance,domain,session,home,binding,generation],4)?;
    if claims.len()!=1 || claims[0][0]!="RELEASED" || !claims[0][2].is_empty()
        || !claims[0][3].is_empty() { return Err(denied()); }
    let owner = rows(db,
        "SELECT state FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5",
        &[binding,instance,domain,session,generation],1)?;
    if owner.len()!=1 || !matches!(owner[0][0].as_str(),"ACTIVE"|"REVOKED")
        || !any(db,"SELECT 1 FROM main.gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
            &[home,instance,domain,session,generation])? { return Err(denied()); }
    for (sql,values) in [
        ("SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND domain_id=?2 AND generation=?3 LIMIT 1",vec![instance,domain,generation]),
    ] { if any(db,sql,&values)? { return Err(denied()); } }
    let history = rows(db,
        "SELECT operation,raw_hex,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT),request_id FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 ORDER BY revision",
        &[domain,session],6)?;
    if history.len()!=3 || claims[0][1]!="3" { return Err(denied()); }
    let physical_home=rows(db,
        "SELECT directory_ref,directory_identity FROM main.gogoke_v37_instance_homes WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5 AND state='ACTIVE'",
        &[home,instance,domain,session,generation],2)?;
    if physical_home.len()!=1 || physical_home[0][0]!=format!("temp-home-{}",sha256_hex(home.as_bytes()))
        || !physical_home[0][1].starts_with("volume:")
        || !physical_home[0][1].contains("/file:") { return Err(denied()); }
    let preparation=sha256_hex(format!("{domain}\n{}",history[0][5]).as_bytes());
    let creation=rows(db,
        "SELECT request_id,request_hex FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase='APPLIED' AND native_receipt_id=?2",
        &[home,&physical_home[0][1]],2)?;
    if creation.len()!=1 || creation[0][0]!=format!("homeprep-{}",&preparation[..40]) { return Err(denied()); }
    let fields=unframe(&creation[0][1])?;
    if fields.len()!=9 || fields[0]!=b"temporary-create-detached"
        || fields[1]!=unhex(&history[0][1])? || fields[2]!=home.as_bytes()
        || fields[3]!=instance.as_bytes() || fields[4]!=domain.as_bytes()
        || fields[5]!=b"SESSION" || fields[6]!=session.as_bytes()
        || fields[7]!=generation.as_bytes() || !fields[8].starts_with(b"S-1-15-2-") {
        return Err(denied());
    }
    let seat = rows(db,
        "SELECT seat_id,seat_incarnation,CAST(seat_authorization_generation AS TEXT),selected_instance_id FROM main.gogoke_v37_effective_seat WHERE domain_id=?1 AND session_id=?2 AND generation=?3",
        &[domain,session,generation],4)?;
    if seat.len()!=1 || seat[0][3]!=instance || seat[0][0].is_empty()
        || seat[0][1].is_empty() || seat[0][2]!=generation
        || !any(db,"SELECT 1 FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2 AND seat_id=?3 AND seat_incarnation=?4 AND generation=?5",
            &[domain,session,&seat[0][0],&seat[0][1],generation])? { return Err(denied()); }
    for (index,row) in history.iter().enumerate() {
        let operation=["admission-reserve","admission-commit","admission-release"][index];
        if row[0]!=operation || row[2]!="APPLIED" || row[3]!=index.to_string()
            || row[4]!=(index+1).to_string() { return Err(denied()); }
        let raw=unhex(&row[1])?;
        let request=crate::store::session_transport::decode_request(&raw)
            .map_err(|error|fail("original never-started H request",error))?;
        if request.family!="K-SESSION" || request.operation!=operation
            || request.domain_id!=domain || request.target_id!=session
            || request.request_id!=row[5] || request.expected_revision!=index as u64
            || request.payload.len()!=2
            || request_field(&request,"seatId").as_deref()!=Some(seat[0][0].as_str())
            || request_field(&request,"generation").as_deref()!=Some(generation) {
            return Err(denied());
        }
    }
    Ok(())
}

fn qualified_resources(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for (sql, values) in [
        // F producers address the instance, temporary HOME, private history,
        // and credential-instance digest in distinct target namespaces. An
        // unresolved F intent anywhere on this verified connection refuses;
        // no derived target can disappear through an instance-only join.
        ("SELECT 1 FROM main.gogoke_v37_instance_operations WHERE phase!='APPLIED' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE open_driver_id='codex' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id='managed-cli-codex' AND (state!='STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_operation WHERE status!='APPLIED' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_generation_change WHERE stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_stdin_journal WHERE phase!='RECEIPTED' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id='codex' AND (a.cleanup_status IN ('NOT_REQUESTED','UNKNOWN') OR a.phase IN ('INTENT','PREPARE_STARTED','UNKNOWN')) LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id='codex' AND h.state IN ('PREPARING','CLOSE_UNKNOWN','CLEANUP_UNKNOWN','UNKNOWN') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_objects o JOIN main.gogoke_v37_instances i ON i.instance_id=o.instance_id WHERE i.driver_id='codex' AND o.phase!='ACTIVE' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_aliases a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id='codex' AND a.state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id WHERE i.driver_id='codex' AND p.state!='REVOKED' LIMIT 1", vec![]),
    ] {
        if any(db, sql, &values)? { return Err(denied()); }
    }
    let claims = rows(db,
        "SELECT c.instance_id,c.domain_id,c.session_id,c.state,COALESCE(c.process_operation_id,''),COALESCE(c.stop_fact_id,''),c.generation,c.binding_id FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id='codex'", &[], 8)?;
    for c in &claims {
        if c[4].is_empty() {
            if c[3]!="RELEASED" || !c[5].is_empty() || any(db,
                "SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2 LIMIT 1",
                &[&c[1],&c[2]])? { return Err(denied()); }
            continue;
        }
        let custody = rows(db, "SELECT state,COALESCE(stop_proof_hash,''),generation FROM main.gogoke_coordination_process_custody WHERE operation_id=?1 AND profile_id=?2 AND domain_id=?3", &[&c[4], &c[0], &c[1]], 3)?;
        if custody.len() != 1 || custody[0][2] != c[6] { return Err(denied()); }
        let settled = matches!(c[3].as_str(), "RELEASED" | "STOPPED") &&
            !c[5].is_empty() && custody[0][0] == "STOPPED" && custody[0][1] == c[5];
        let native = matches!(c[3].as_str(), "COMMITTED" | "UNKNOWN" | "RELEASED") &&
            c[5].is_empty() && custody[0][0] == "UNKNOWN";
        if !settled && !native { return Err(denied()); }
    }
    let bindings = rows(db,
        "SELECT b.instance_id,b.binding_id,b.domain_id,b.owner_id,b.generation,b.state FROM main.gogoke_v37_h_owner_binding b JOIN main.gogoke_v37_instances i ON i.instance_id=b.instance_id WHERE i.driver_id='codex'", &[], 6)?;
    for b in bindings {
        if b[5] == "ACTIVE" && !claims.iter().any(|c| c[0] == b[0] && c[1] == b[2] &&
            c[2] == b[3] && c[6] == b[4] && c[7] == b[1]) { return Err(denied()); }
    }
    let generations = rows(db,
        "SELECT g.domain_id,g.session_id,g.generation,g.request_id,g.process_operation_id FROM main.gogoke_v37_h_generation g JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id='codex'", &[], 5)?;
    for g in &generations {
        if !any(db, "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE operation_id=?1 AND domain_id=?2 AND generation=?3 LIMIT 1",
            &[&g[4], &g[0], &g[2]])? { return Err(denied()); }
    }
    let all_generations = rows(db,
        "SELECT e.domain_id,e.session_id,e.generation,e.request_id,e.process_operation_id FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id='codex' AND e.process_operation_id IS NOT NULL", &[], 5)?;
    if generations.len() != all_generations.len() { return Err(denied()); }
    let active_homes = rows(db,
        "SELECT h.home_id,h.instance_id,h.domain_id,h.owner_id,h.generation FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id='codex' AND h.state='ACTIVE'", &[], 5)?;
    for h in active_homes {
        let execution = rows(db,
            "SELECT e.process_operation_id,c.state,COALESCE(c.stop_proof_hash,''),e.binding_id,e.phase,COALESCE(e.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5", &[&h[0], &h[1], &h[2], &h[3], &h[4]], 6)?;
        if execution.len()>1 { return Err(denied()); }
        if let Some(e)=execution.first() {
            if e[1]=="STOPPED" && !e[2].is_empty() && e[4]=="STOPPED" && e[5]==e[2] {
                qualified_stopped_h(db,&e[0],&h[1],&h[2],&h[4],&e[2])?;
            } else if e[1]=="UNKNOWN" && e[2].is_empty()
                && matches!(e[4].as_str(),"ACTIVE"|"UNKNOWN") && e[5].is_empty() {
                let (fact,_,pairs)=crate::store::session_transport::observe_exact_home_gone(
                    db,&h[0],&h[1],&h[2],&h[3],&h[4],&e[3])
                    .map_err(|error|fail("ACTIVE HOME original execution",error))?;
                fact.validate(&pairs).map_err(|error|fail("ACTIVE HOME kernel recheck",error))?;
            } else { return Err(denied()); }
        } else {
            let claim = rows(db,"SELECT binding_id,state,COALESCE(process_operation_id,'') FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND domain_id=?2 AND session_id=?3 AND home_id=?4 AND generation=?5",
                &[&h[1],&h[2],&h[3],&h[0],&h[4]],3)?;
            if claim.len()!=1 || claim[0][1]!="RELEASED" || !claim[0][2].is_empty() { return Err(denied()); }
            qualified_released_never_started(db,&h[1],&h[2],&h[3],&h[0],&claim[0][0],&h[4])?;
        }
    }
    Ok(())
}

/// Reproduce F's original profile naming from every C custody row and its
/// unique H episode. A missing H association cannot silently omit a SID from
/// the physical ACL inventory.
fn historical_profile_names(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    instance: &str, home: &crate::root::RootIdentity) -> Result<Vec<String>> {
    let owner = sha256_hex(format!("{}\n{instance}", home.opaque()).as_bytes());
    let mut names = vec![format!("Gogoke37.OwnerLogin.{}", &owner[..40])];
    let custody = rows(db,
        "SELECT operation_id,profile_id,domain_id,generation FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 ORDER BY operation_id",
        &[instance], 4)?;
    for c in custody {
        if c[1] != instance { return Err(denied()); }
        if c[2] == "global" { continue; }
        let episode = rows(db,
            "SELECT domain_id,session_id,seat_incarnation,generation,instance_id FROM main.gogoke_v37_h_process_episode WHERE process_operation_id=?1",
            &[&c[0]], 5)?;
        if episode.len() != 1 || episode[0][0] != c[2] || episode[0][3] != c[3]
            || episode[0][4] != instance || episode[0][2].is_empty() { return Err(denied()); }
        let e = &episode[0];
        let digest = sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            root.canonical_root().identity.opaque(), e[0], e[1], e[2], e[3]).as_bytes());
        names.push(format!("Gogoke37.Session.{}", &digest[..40]));
    }
    names.sort(); names.dedup();
    Ok(names)
}

fn qualify(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &LegacySourceRequest<'_>) -> Result<Qualification> {
    valid_request(input)?;
    if any(db, "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE request_id=?1 LIMIT 1",
        &[input.request_id])? || any(db,
        "SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE request_id=?1 LIMIT 1",
        &[input.request_id])? { return Err(denied()); }
    let before = database_snapshot(db)?;
    let candidates = rows(db,
        "SELECT i.instance_id,i.home_identity,i.program_digest,i.version,COALESCE(CAST(p.enabled AS TEXT),''),COALESCE(CAST(p.tombstoned AS TEXT),'') FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id LEFT JOIN main.gogoke_v37_instance_program_sources s ON s.instance_id=i.instance_id WHERE i.driver_id='codex' AND (p.tombstoned IS NULL OR p.tombstoned=0) AND s.instance_id IS NULL ORDER BY i.instance_id", &[], 6)?;
    if candidates.len() != 1 || candidates[0][0] != input.target_id ||
        candidates[0][4] != "0" || candidates[0][5] != "0" ||
        candidates[0][1] != input.original_home_identity ||
        !source_rows(db, input.target_id)?.is_empty() { return Err(denied()); }
    let pin = managed_cli::read_fixed_official_cli("codex").ok_or_else(denied)?;
    let digest = format!("sha256:{}", pin.image_sha256);
    if candidates[0][2] != digest || candidates[0][3] != pin.version { return Err(denied()); }
    let copy = managed_cli::read_managed_cli(db, root, "codex")
        .map_err(|error| fail("READY copy", error))?.ok_or_else(denied)?;
    let stage = copy.stage_name.ok_or_else(denied)?;
    if copy.state != "READY" || copy.version.as_deref() != Some(pin.version) { return Err(denied()); }
    let path = managed_cli::locate_ready_managed_program(db, root, "codex", &digest, pin.version)
        .map_err(|error| fail("official stage", error))?.ok_or_else(denied)?;
    let observed = registry::ProgramObservation::observe(&path, pin.version)
        .map_err(|error| fail("official executable", error))?;
    if !observed.matches_pin(&digest, pin.version) { return Err(denied()); }
    if registry::classify_legacy_source_registration(db, root, input.target_id,
        input.original_request_id, input.original_fingerprint, input.original_receipt_hex,
        input.original_home_identity, &observed)
        .map_err(|error| fail("original USER replay", error))?
        != registry::LegacyReplayClass::ProvenIneligible { return Err(denied()); }
    // A missing credential registry row does not mean auth.json is absent.
    // Resolve the original registered HOME, then ask the native metadata and
    // ACL producers about the physical source without reading its contents.
    let home = resolve_codex_instance_home(db, root, input.target_id)
        .map_err(|error| fail("registered physical HOME", error))?;
    if home.identity.opaque() != input.original_home_identity { return Err(denied()); }
    let source = home.path.join("auth.json");
    let (source_identity, links) = CredentialBinding::observe_source_metadata(
        root, &source, &home.identity).map_err(|error| fail("physical credential metadata", error))?;
    if links != 1 { return Err(denied()); }
    let aliases = credential_registry::read_credential_aliases(db, input.target_id)
        .map_err(|error| fail("credential aliases", error))?;
    if aliases.iter().any(|alias| alias.state != "REMOVED") { return Err(denied()); }
    let object = credential_registry::read_credential_object(db, input.target_id)
        .map_err(|error| fail("credential object", error))?;
    if let Some(object) = &object {
        let backend = credential_registry::read_usable_credential_backend(db, input.target_id)
            .map_err(|error| fail("credential source", error))?;
        if object.root_identity.opaque() != db.root_identity().opaque()
            || object.home_identity.opaque() != home.identity.opaque()
            || object.source_parent_identity.opaque() != home.identity.opaque()
            || object.file_identity != source_identity
            || backend.home_identity.opaque() != home.identity.opaque() { return Err(denied()); }
    } else if any(db,
        "SELECT 1 FROM main.gogoke_v37_credential_backend_evidence WHERE instance_id=?1 LIMIT 1",
        &[input.target_id])? {
        // A partial credential registration is neither the pristine legacy
        // shape nor an authenticated registered object.
        return Err(denied());
    }
    let binding = CredentialBinding::open_registered(root, &source, &home.identity,
        &source_identity, &[]).map_err(|error| fail("held physical credential source", error))?;
    let names = historical_profile_names(db, root, input.target_id, &home.identity)?;
    let provenance = sha256_hex(names.join("\n").as_bytes());
    let fence = legacy_fence::read_legacy_fence(db, input.target_id)
        .map_err(|error| fail("prior legacy ACL fence", error))?;
    let inventory = if let Some(record) = &fence {
        if record.database_identity != db.identity().opaque()
            || record.root_identity != db.root_identity().opaque()
            || record.home_identity != home.identity.opaque()
            || record.source_identity != source_identity.opaque()
            || record.source_parent_identity != home.identity.opaque()
            || record.source_revision != object.as_ref().map(|row| row.revision)
            || record.acl_provenance_digest != provenance { return Err(denied()); }
        let inventory = LegacyAclInventory::restore(root, &home.path, &home.identity,
            &binding, &names, &record.native_snapshot, &record.native_snapshot_digest)
            .map_err(|error| fail("complete fenced physical HOME ACL", error))?;
        if record.home_acl_digest != inventory.home_original_digest()
            || record.source_acl_digest != inventory.source_original_digest() { return Err(denied()); }
        let home_step=legacy_fence::read_legacy_step(db,input.target_id,legacy_fence::LegacyAclStep::Home)
            .map_err(|error|fail("legacy HOME ACL step",error))?;
        let baseline_step=legacy_fence::read_legacy_step(db,input.target_id,legacy_fence::LegacyAclStep::Baseline)
            .map_err(|error|fail("legacy baseline step",error))?;
        if baseline_step.is_some() && home_step.is_none() { return Err(denied()); }
        for (step,intent) in [(legacy_fence::LegacyAclStep::Home,home_step),
            (legacy_fence::LegacyAclStep::Baseline,baseline_step)] {
            if let Some(intent) = intent {
                if intent.phase != legacy_fence::LegacyStepPhase::Applied { return Err(denied()); }
                if step == legacy_fence::LegacyAclStep::Home
                    && intent.target_home_acl_digest != inventory.home_target_digest() { return Err(denied()); }
                if step == legacy_fence::LegacyAclStep::Baseline
                    && intent.target_source_acl_digest != inventory.source_target_digest() { return Err(denied()); }
            }
        }
        inventory
    } else {
        if legacy_fence::read_legacy_step(db, input.target_id, legacy_fence::LegacyAclStep::Home)
            .map_err(|error| fail("unbound HOME ACL step", error))?.is_some()
            || legacy_fence::read_legacy_step(db, input.target_id, legacy_fence::LegacyAclStep::Baseline)
                .map_err(|error| fail("unbound baseline step", error))?.is_some() { return Err(denied()); }
        LegacyAclInventory::capture(root, &home.path, &home.identity,
            &binding, &names).map_err(|error| fail("fresh physical historical HOME ACL", error))?
    };
    qualified_resources(db)?;
    let mut evidence = Vec::new();
    qualified_executions(db, &mut evidence)?;
    for value in [inventory.original_digest(), inventory.home_original_digest(),
        inventory.source_original_digest(), provenance,
        source_identity.opaque()] { frame(&mut evidence, value.as_bytes()); }
    let after = database_snapshot(db)?;
    if after != before { return Err(denied()); }
    frame(&mut evidence, before.as_bytes());
    frame(&mut evidence, db.identity().opaque().as_bytes());
    frame(&mut evidence, db.root_identity().opaque().as_bytes());
    let source_fingerprint = source_fingerprint(input.target_id, &stage, &digest,
        pin.version, input.original_home_identity, input.original_request_id);
    Ok(Qualification { stage, digest, version: pin.version.to_owned(),
        home: input.original_home_identity.to_owned(), source_fingerprint,
        qualification_digest: sha256_hex(&evidence) })
}

fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>,
    work: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T>) -> Result<T> {
    db.execute("BEGIN IMMEDIATE").map_err(|error| fail("begin", error))?;
    match work(db) {
        Ok(value) => { db.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?; Ok(value) },
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(fail("rollback unknown", (original, rollback))),
        },
    }
}

pub(crate) fn migrate_legacy_codex_source(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, input: &LegacySourceRequest<'_>,
    applied_receipt: &[u8]) -> Result<LegacySourceResult> {
    valid_request(input)?;
    if let Some(prior) = read_journal(db, input.request_id)? {
        if !same_intent(db, input, &prior) { return Err(denied()); }
        return applied_readback(db, root, input, &prior, applied_receipt);
    }
    let initial = qualify(db, root, input)?;
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        if read_journal(db, input.request_id)?.is_some() || any(db,
            "SELECT 1 FROM main.gogoke_v37_instance_legacy_source_migrations WHERE target_id=?1 LIMIT 1", &[input.target_id])? ||
            !source_rows(db, input.target_id)?.is_empty() { return Err(denied()); }
        let current = qualify(db, root, input)?;
        if current.qualification_digest != initial.qualification_digest { return Err(denied()); }
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_legacy_source_migrations(request_id,target_id,raw_hex,raw_sha256,original_request_id,original_fingerprint,original_receipt_hex,original_home_identity,root_identity,database_identity,qualification_digest,phase) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'PREPARING')")?;
        for (index, value) in [input.request_id, input.target_id, &hex(input.raw),
            &sha256_hex(input.raw), input.original_request_id, input.original_fingerprint,
            input.original_receipt_hex, input.original_home_identity,
            &db.root_identity().opaque(), &db.identity().opaque(),
            &current.qualification_digest].iter().enumerate() {
            write.bind_text((index + 1) as i32, value)?;
        }
        write.step_done()?;
        let prior = read_journal(db, input.request_id)?.ok_or_else(denied)?;
        if !same_intent(db, input, &prior) || prior.phase != "PREPARING" ||
            prior.qualification_digest != current.qualification_digest { return Err(denied()); }
        Ok(())
    })?;
    // Only this uninterrupted invocation can enter the effect transaction.
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let prior = read_journal(db, input.request_id)?.ok_or_else(denied)?;
        if !same_intent(db, input, &prior) || prior.phase != "PREPARING" ||
            !source_rows(db, input.target_id)?.is_empty() ||
            any(db, "SELECT 1 FROM main.gogoke_v37_instance_legacy_source_migrations WHERE target_id=?1 AND request_id<>?2 LIMIT 1",
                &[input.target_id, input.request_id])? { return Err(denied()); }
        let current = qualify(db, root, input)?;
        if current.qualification_digest != prior.qualification_digest ||
            current.source_fingerprint != initial.source_fingerprint { return Err(denied()); }
        let source = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES(?1,'MANAGED',?2,?3,?4,?5,?6,1)")?;
        for (index, value) in [input.target_id, &current.stage, &current.digest,
            &current.version, &current.home, input.original_request_id].iter().enumerate() {
            source.bind_text((index + 1) as i32, value)?;
        }
        source.step_done()?;
        let applied = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_legacy_source_migrations SET phase='APPLIED',source_fingerprint=?1,receipt_hex=?2 WHERE request_id=?3 AND target_id=?4 AND raw_hex=?5 AND phase='PREPARING'")?;
        for (index, value) in [&current.source_fingerprint, &hex(applied_receipt),
            input.request_id, input.target_id, &hex(input.raw)].iter().enumerate() {
            applied.bind_text((index + 1) as i32, value)?;
        }
        applied.step_done()?;
        let changed = rows(db, "SELECT changes()", &[], 1)?;
        if changed.len() != 1 || changed[0][0] != "1" { return Err(denied()); }
        let journal = read_journal(db, input.request_id)?.ok_or_else(denied)?;
        let reconciled = applied_readback(db, root, input, &journal, applied_receipt)?;
        if reconciled.receipt != applied_receipt { return Err(denied()); }
        Ok(LegacySourceResult { receipt: applied_receipt.to_vec() })
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl for<'a> FnOnce(&'a RootLock, VerifiedDatabaseConnection<'a>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "gogoke-legacy-source-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut db = create_new(&root, &database).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        run(&root, db);
        drop(root);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn preparing_is_durable_refusal_and_applied_without_source_is_not_a_receipt() {
        fixture(|root, mut db| {
            let owner = crate::store::authority::initialize_profile(&mut db, root).unwrap();
            let raw = br#"{"schema":"gogoke.37.managed-cli.v1","command":"migrate-legacy-codex-source","driverId":"codex","requestId":"newA","targetId":"oldA","originalRequestId":"registerA","originalRequestFingerprint":"abcd","originalNativeReceiptHex":"686f6d65","originalHomeIdentity":"home"}"#;
            let input = LegacySourceRequest { raw, request_id:"newA", target_id:"oldA",
                original_request_id:"registerA", original_fingerprint:"abcd",
                original_receipt_hex:"686f6d65", original_home_identity:"home" };
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_legacy_source_migrations(request_id,target_id,raw_hex,raw_sha256,original_request_id,original_fingerprint,original_receipt_hex,original_home_identity,root_identity,database_identity,qualification_digest,phase) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'digest','PREPARING')").unwrap();
            for (index, value) in [input.request_id,input.target_id,&hex(raw),&sha256_hex(raw),
                input.original_request_id,input.original_fingerprint,input.original_receipt_hex,
                input.original_home_identity,&db.root_identity().opaque(),&db.identity().opaque()]
                .iter().enumerate() { insert.bind_text((index+1) as i32, value).unwrap(); }
            insert.step_done().unwrap(); drop(insert);
            let database = root.canonical_root().canonical_path.join("state.sqlite");
            db.close_checked().unwrap();
            let db = open_existing(root, &database).unwrap();
            let prior = read_journal(&db, input.request_id).unwrap().unwrap();
            assert!(same_intent(&db, &input, &prior));
            let mut db = db;
            assert!(migrate_legacy_codex_source(&mut db, root, &owner,
                &input, b"receipt").is_err());
            assert!(source_rows(&db, input.target_id).unwrap().is_empty());
            assert_eq!(read_journal(&db, input.request_id).unwrap().unwrap().phase, "PREPARING");
            assert!(applied_readback(&db, root, &input, &prior, b"receipt").is_err());
            let changed = LegacySourceRequest { raw: b"different", request_id:input.request_id,
                target_id:input.target_id, original_request_id:input.original_request_id,
                original_fingerprint:input.original_fingerprint,
                original_receipt_hex:input.original_receipt_hex,
                original_home_identity:input.original_home_identity };
            assert!(!same_intent(&db, &changed, &prior));
            db.execute("UPDATE main.gogoke_v37_instance_legacy_source_migrations SET phase='APPLIED',source_fingerprint='fake',receipt_hex='7b7d' WHERE request_id='newA'").unwrap();
            let corrupted = read_journal(&db, "newA").unwrap().unwrap();
            assert!(applied_readback(&db, root, &input, &corrupted, b"{}").is_err());
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn snapshot_sees_actual_row_and_revision_changes() {
        fixture(|_, mut db| {
            let before = database_snapshot(&db).unwrap();
            db.execute("INSERT INTO main.gogoke_v37_instances VALUES('oldA','codex','homeA','identityA','sha256:old','1','UNKNOWN','UNKNOWN',1)").unwrap();
            let after_insert = database_snapshot(&db).unwrap();
            assert_ne!(before, after_insert);
            assert_eq!(after_insert, database_snapshot(&db).unwrap());
            db.execute("UPDATE main.gogoke_v37_instances SET revision=2 WHERE instance_id='oldA'").unwrap();
            assert_ne!(after_insert, database_snapshot(&db).unwrap());
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn applied_replay_reads_exact_source_and_original_receipt_only() {
        fixture(|root, mut db| {
            let home = super::super::home::prepare_persistent_home(root, "oldA").unwrap();
            let identity = home.identity.opaque();
            let raw = b"exact OWNER frame";
            let receipt = b"exact original APPLIED receipt";
            let input = LegacySourceRequest { raw, request_id:"newA", target_id:"oldA",
                original_request_id:"registerA", original_fingerprint:"abcd",
                original_receipt_hex:"686f6d65", original_home_identity:&identity };
            let instance = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instances VALUES('oldA','codex','instance-home-oldA',?1,'sha256:current','2','INSTALLED','LOGGED_OUT',1)").unwrap();
            instance.bind_text(1, &identity).unwrap(); instance.step_done().unwrap(); drop(instance);
            let source = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_program_sources VALUES('oldA','MANAGED','stageA','sha256:current','2',?1,'registerA',1)").unwrap();
            source.bind_text(1, &identity).unwrap(); source.step_done().unwrap(); drop(source);
            let fingerprint = source_fingerprint("oldA","stageA","sha256:current","2",&identity,"registerA");
            let insert = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_legacy_source_migrations VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'qualified','APPLIED',?11,?12)").unwrap();
            for (index, value) in [input.request_id,input.target_id,&hex(raw),&sha256_hex(raw),
                input.original_request_id,input.original_fingerprint,input.original_receipt_hex,
                input.original_home_identity,&db.root_identity().opaque(),&db.identity().opaque(),
                &fingerprint,&hex(receipt)].iter().enumerate() {
                insert.bind_text((index+1) as i32, value).unwrap();
            }
            insert.step_done().unwrap(); drop(insert);
            let prior = read_journal(&db, "newA").unwrap().unwrap();
            assert_eq!(applied_readback(&db, root, &input, &prior, receipt).unwrap().receipt, receipt);
            db.execute("UPDATE main.gogoke_v37_instance_program_sources SET version='3' WHERE instance_id='oldA'").unwrap();
            assert!(applied_readback(&db, root, &input, &prior, receipt).is_err());
            std::fs::remove_file(home.path.join("gogoke-instance.marker")).unwrap();
            std::fs::remove_dir(home.path).unwrap();
            std::fs::remove_dir(root.canonical_root().canonical_path.join("v37-instances")).unwrap();
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn stopped_h_requires_original_raw_home_binding_generation_and_c_proof() {
        fixture(|root, mut db| {
            crate::store::authority::initialize_profile(&mut db,root).unwrap();
            crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
            crate::store::seat::initialize_schema(&mut db).unwrap();
            crate::store::session_transport::initialize_admission_schema(&mut db).unwrap();
            db.execute("INSERT INTO main.gogoke_v37_instances VALUES('oldA','codex','instance-home-oldA','homeIdentity','sha256:old','1','UNKNOWN','UNKNOWN',1)").unwrap();
            let raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"treeA"}}"#;
            let home_identity="volume:0000000000000001/file:00000000000000000000000000000001";
            let home_ref=format!("temp-home-{}",sha256_hex(b"temporaryA"));
            let home=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,directory_ref,directory_identity,state,revision) VALUES('temporaryA','oldA','projectA','SESSION','sessionA','1',?1,?2,'CLOSED',2)").unwrap();
            home.bind_text(1,&home_ref).unwrap();home.bind_text(2,home_identity).unwrap();home.step_done().unwrap();drop(home);
            let mut create_frame=Vec::new();
            for value in [&b"temporary-create-detached"[..],&raw[..],
                &b"temporaryA"[..],&b"oldA"[..],&b"projectA"[..],&b"SESSION"[..],
                &b"sessionA"[..],&b"1"[..],&b"S-1-15-2-7"[..]] { frame(&mut create_frame,value); }
            let preparation=sha256_hex(b"projectA\nopenA");
            let preparation_id=format!("homeprep-{}",&preparation[..40]);
            let create=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,native_receipt_id) VALUES(?1,?2,'temporaryA','APPLIED',?3)").unwrap();
            create.bind_text(1,&preparation_id).unwrap();create.bind_text(2,&hex(&create_frame)).unwrap();
            create.bind_text(3,home_identity).unwrap();
            create.step_done().unwrap();drop(create);
            db.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','oldA','projectA','SESSION','sessionA','1','REVOKED')").unwrap();
            let operation=format!("h-open-{}",&sha256_hex(raw)[..40]);
            let proof="sha256:1111111111111111111111111111111111111111111111111111111111111111";
            let original_hex=hex(raw);
            let h_operation=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openA',?1,'open','sessionA','APPLIED',1,2)").unwrap();
            h_operation.bind_text(1,&original_hex).unwrap();h_operation.step_done().unwrap();drop(h_operation);
            let claim=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('projectA','sessionA','oldA','temporaryA','bindingA','1','STOPPED',3,?1,?2)").unwrap();
            claim.bind_text(1,&operation).unwrap();claim.bind_text(2,proof).unwrap();claim.step_done().unwrap();drop(claim);
            let custody=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES(?1,'ticketA','nonceA','4294967295','1','synthetic.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','oldA','projectA','1','STOPPED',?2)").unwrap();
            custody.bind_text(1,&operation).unwrap();custody.bind_text(2,proof).unwrap();custody.step_done().unwrap();drop(custody);
            let episode=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('projectA','openA','sessionA','1',?1,1,2,?2,'oldA','temporaryA','bindingA','seatA','incA','STOPPED',?3)").unwrap();
            episode.bind_text(1,&original_hex).unwrap();episode.bind_text(2,&operation).unwrap();episode.bind_text(3,proof).unwrap();episode.step_done().unwrap();drop(episode);
            let generation=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openA',?1)").unwrap();
            generation.bind_text(1,&operation).unwrap();generation.step_done().unwrap();drop(generation);
            let check=|db:&VerifiedDatabaseConnection<'_>| qualified_executions(db,&mut Vec::new());
            check(&db).unwrap();
            let corrupt_create=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET request_hex='zz' WHERE request_id=?1").unwrap();
            corrupt_create.bind_text(1,&preparation_id).unwrap();corrupt_create.step_done().unwrap();drop(corrupt_create);
            assert!(check(&db).is_err());
            let restore_create=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id=?2").unwrap();
            restore_create.bind_text(1,&hex(&create_frame)).unwrap();
            restore_create.bind_text(2,&preparation_id).unwrap();restore_create.step_done().unwrap();drop(restore_create);
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET raw_hex='zz' WHERE request_id='openA'").unwrap();
            assert!(check(&db).is_err());
            let restore=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_h_process_episode SET raw_hex=?1 WHERE request_id='openA'").unwrap();
            restore.bind_text(1,&original_hex).unwrap();restore.step_done().unwrap();drop(restore);
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET home_id='otherHome' WHERE request_id='openA'").unwrap();
            assert!(check(&db).is_err());
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET home_id='temporaryA',binding_id='otherBinding' WHERE request_id='openA'").unwrap();
            assert!(check(&db).is_err());
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET binding_id='bindingA' WHERE request_id='openA'").unwrap();
            let corrupt_proof=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrong' WHERE operation_id=?1").unwrap();
            corrupt_proof.bind_text(1,&operation).unwrap();corrupt_proof.step_done().unwrap();drop(corrupt_proof);
            assert!(check(&db).is_err());
            db.close_checked().unwrap();
        });
    }
}
