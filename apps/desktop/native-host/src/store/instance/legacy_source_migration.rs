//! One authenticated, original-intent migration of a proven legacy Codex SOURCE.
//! This F journal never advances an old instance operation or H/C outcome.

use super::{credential_registry, legacy_fence, managed_cli, registry,
    resolve_codex_instance_home, temporary};
use crate::process::{CredentialBinding, LegacyAclInventory, NativeLegacyHoldersGone};
use crate::root::RootLock;
use crate::store::atomic::{Json, JsonString, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::digest::sha256_hex;
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
#[cfg(all(test, windows))]
use crate::store::same_open::SameOpenError;
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
    pub(crate) expected_root_identity: &'a str,
    pub(crate) expected_database_identity: &'a str,
}

pub(crate) struct LegacySourceResult {
    pub(crate) receipt: Vec<u8>,
}

#[cfg(all(test, windows))]
impl LegacySourceResult {
    pub(crate) fn qualification_digest_for_test(db: &VerifiedDatabaseConnection<'_>,
        root: &RootLock, input: &LegacySourceRequest<'_>) -> Result<String> {
        qualify(db, root, input).map(|value| value.qualification_digest)
    }

    pub(crate) fn run_focused_history_fixtures() {
        tests::optional_legacy_fence_family_classifier_matrix();
        tests::plain_resume_accepts_only_direct_or_original_unknown_promotion_equation();
        tests::stopped_h_requires_original_raw_home_binding_generation_and_c_proof();
    }

    pub(crate) fn run_released_source_fixtures() {
        tests::global_native_never_started_release_uses_original_empty_payload();
    }
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
        input.original_home_identity.is_empty() || input.original_home_identity.len() > 256 ||
        input.expected_root_identity.is_empty() || input.expected_root_identity.len() > 128 ||
        input.expected_database_identity.is_empty() || input.expected_database_identity.len() > 128 {
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
    journal.root == input.expected_root_identity &&
    journal.database == input.expected_database_identity &&
    journal.root == db.root_identity().opaque() && journal.database == db.identity().opaque()
}
fn exact_physical_authority(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &LegacySourceRequest<'_>) -> Result<()> {
    if db.root_identity() != &root.canonical_root().identity ||
        input.expected_root_identity != db.root_identity().opaque() ||
        input.expected_database_identity != db.identity().opaque() {
        return Err(denied());
    }
    Ok(())
}
fn applied_readback(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &LegacySourceRequest<'_>, journal: &Journal,
    expected_receipt: &[u8]) -> Result<LegacySourceResult> {
    if !same_intent(db, input, journal) || journal.phase != "APPLIED" ||
        journal.qualification_digest.is_empty() { return Err(denied()); }
    if registry::verified_legacy_registration_anchors(db, root, input.target_id,
        input.original_request_id, input.original_fingerprint,
        input.original_receipt_hex, input.original_home_identity)
        .map_err(|error| fail("original registration replay anchors", error))?
        .is_none() { return Err(denied()); }
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

fn qualified_plain_resume_revisions(expected:u64,previous:u64,result:u64)->bool {
    let direct=expected==previous && expected.checked_add(1)==Some(result);
    let reconciled_unknown=expected.checked_add(1)==Some(previous)
        && expected.checked_add(2)==Some(result);
    direct || reconciled_unknown
}

/// STOPPED is a durable H/C outcome, but its strings are meaningful only after
/// the original execution producer, HOME, binding, generation and operation
/// have been independently re-associated. A table digest alone cannot do it.
fn qualified_stopped_h(db: &VerifiedDatabaseConnection<'_>, operation: &str,
    instance: &str, domain: &str, generation: &str, proof: &str) -> Result<String> {
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
        } else if request.operation=="resume" {
            // create_resume_intent persists raw.expectedRevision as
            // episode.previous_revision. A direct promote leaves it there and
            // writes result=expected+1. The only other producer is
            // mark_resume_unknown followed by promote_resume: UNKNOWN first
            // writes result=expected+1, then promote copies that exact result
            // to previous and writes result=expected+2. Plain resume persists
            // no separate UNKNOWN marker, so these are the two exact final
            // equations accepted here.
            if !qualified_plain_resume_revisions(request.expected_revision,before,after) {
                return Err(denied());
            }
        } else if request.expected_revision!=before { return Err(denied()); }
    } else {
        if !e[3].is_empty() || request.expected_revision!=before { return Err(denied()); }
        let formal=request_field(&request,"purpose").as_deref()==Some("FORMAL_REVIEW");
        if domain=="global" {
            if !request.payload.is_empty() { return Err(denied()); }
            // effective_seat projects generation from the current claim and
            // cannot authenticate an older stopped Secretary generation. The
            // native selection (or its legacy seat binding fallback) preserves
            // the original seat/incarnation independently of that projection.
            let selected=rows(db,
                "SELECT 1 FROM main.gogoke_v37_native_selection s JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=s.domain_id AND d.seat_id=s.seat_id AND d.incarnation=s.seat_incarnation WHERE s.domain_id='global' AND s.session_id=?1 AND s.seat_id=?2 AND s.seat_incarnation=?3 AND s.selected_instance_id=?4 UNION ALL SELECT 1 FROM main.gogoke_v37_h_seat_binding s JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=s.domain_id AND d.seat_id=s.seat_id AND d.incarnation=s.seat_incarnation JOIN main.gogoke_v37_h_claim c ON c.domain_id=s.domain_id AND c.session_id=s.session_id WHERE s.domain_id='global' AND s.session_id=?1 AND s.seat_id=?2 AND s.seat_incarnation=?3 AND c.instance_id=?4 AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_native_selection n WHERE n.domain_id=s.domain_id AND n.session_id=s.session_id)",
                &[&e[1],&e[9],&e[10],instance],1)?;
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
    let home_source=temporary::validate_original_execution_home_source(db,&e[7],instance,
        domain,&e[1],generation,&e[0],&request.operation)
        .map_err(|error|fail("original F HOME create source",error))?;
    let binding=rows(db,
        "SELECT state FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5",
        &[&e[8],instance,domain,&e[1],generation],1)?;
    if binding.len()!=1 || !matches!(binding[0][0].as_str(),"ACTIVE"|"REVOKED") { return Err(denied()); }
    if any(db,"SELECT 1 FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2 AND generation=?3 LIMIT 1",
        &[domain,&e[1],generation])? { return Err(denied()); }
    let stop_source=if !e[11].is_empty() {
        temporary::validate_original_execution_stop_source(db,&e[7],instance,domain,&e[1],
            generation,proof).map_err(|error|fail("original USER STOP source",error))?
    } else { String::new() };
    let current=rows(db,
        "SELECT generation,home_id,binding_id,COALESCE(process_operation_id,''),COALESCE(stop_fact_id,'') FROM main.gogoke_v37_h_claim WHERE domain_id=?1 AND session_id=?2 AND instance_id=?3", &[domain,&e[1],instance],5)?;
    if current.len()>1 { return Err(denied()); }
    if let Some(claim)=current.first() {
        if claim[0]==generation && (claim[1]!=e[7] || claim[2]!=e[8] ||
            claim[3]!=operation || claim[4]!=proof) { return Err(denied()); }
    }
    Ok(sha256_hex(format!("stopped-execution-v1\n{operation}\n{proof}\n{home_source}\n{stop_source}")
        .as_bytes()))
}

fn qualified_unknown_home_source(db: &VerifiedDatabaseConnection<'_>, home: &str,
    instance: &str, domain: &str, session: &str, generation: &str,
    episode_request_id: &str, episode_raw_hex: &str) -> Result<String> {
    let raw=unhex(episode_raw_hex)?;
    let request=crate::store::session_transport::decode_request(&raw)
        .map_err(|error|fail("original UNKNOWN H raw",error))?;
    if request.family!="K-SESSION" ||
        !matches!(request.operation.as_str(),"open"|"resume"|"compact"|"renew-session") ||
        request.request_id!=episode_request_id || request.target_id!=session ||
        request.domain_id!=domain { return Err(denied()); }
    temporary::validate_original_execution_home_source(db,home,instance,domain,
        session,generation,episode_request_id,&request.operation)
        .map_err(|error|fail("original UNKNOWN F HOME create source",error))
}

/// All affected original C rows are read. STOPPED requires its original proof;
/// an unsettled H row needs H's unique original-source kernel fact; a global
/// historical UNKNOWN needs the native whole-boot fact over every original
/// global C identity, independently of any ACL fence.
fn qualified_executions(db: &VerifiedDatabaseConnection<'_>, evidence: &mut Vec<u8>)
    -> Result<(BTreeSet<String>,BTreeSet<String>)> {
    let custody = rows(db,
        "SELECT c.operation_id,c.profile_id,c.domain_id,c.generation,c.state,COALESCE(c.stop_proof_hash,''),c.pid,c.creation_time_100ns FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id WHERE i.driver_id='codex' ORDER BY c.operation_id", &[], 8)?;
    let mut seen_h = BTreeSet::new();
    let mut qualified_bindings = BTreeSet::new();
    let mut qualified_operations = BTreeSet::new();
    let mut global_bare = BTreeSet::new();
    for c in &custody {
        let (operation, instance, domain, generation, state, stop) =
            (&c[0], &c[1], &c[2], &c[3], &c[4], &c[5]);
        let episode = rows(db,
            "SELECT home_id,session_id,generation,binding_id,phase,COALESCE(stop_fact_id,''),request_id,raw_hex FROM main.gogoke_v37_h_process_episode WHERE process_operation_id=?1 AND instance_id=?2 AND domain_id=?3", &[operation, instance, domain], 8)?;
        if episode.len() > 1 { return Err(denied()); }
        if state == "STOPPED" && !stop.is_empty() {
            if let Some(h) = episode.first() {
                if h[2] != *generation || h[4] != "STOPPED" || h[5] != *stop {
                    return Err(denied());
                }
                let source=qualified_stopped_h(db, operation, instance, domain, generation, stop)?;
                frame(evidence,source.as_bytes());
                seen_h.insert(operation.clone());
                qualified_bindings.insert(h[3].clone());
                qualified_operations.insert(operation.clone());
            } else if domain == "global" {
                global_bare.insert(instance.clone());
            } else { return Err(denied()); }
            continue;
        }
        if state != "UNKNOWN" || !stop.is_empty() { return Err(denied()); }
        if episode.is_empty() && domain == "global" {
            global_bare.insert(instance.clone()); continue;
        }
        if episode.len() != 1 || episode[0][2] != *generation ||
            !matches!(episode[0][4].as_str(), "ACTIVE" | "UNKNOWN") || !episode[0][5].is_empty() {
            return Err(denied());
        }
        let home_source=qualified_unknown_home_source(db,&episode[0][0],instance,domain,
            &episode[0][1],generation,&episode[0][6],&episode[0][7])?;
        let (fact, source_digest, pairs) = crate::store::session_transport::observe_exact_home_gone(
            db, &episode[0][0], instance, domain, &episode[0][1], generation, &episode[0][3])
            .map_err(|error| fail("original H execution", error))?;
        fact.validate(&pairs).map_err(|error| fail("H kernel recheck", error))?;
        seen_h.insert(operation.clone());
        qualified_bindings.insert(episode[0][3].clone());
        qualified_operations.insert(operation.clone());
        for value in [home_source.as_str(),source_digest.as_str(), &fact.boot_start().to_string()] {
            frame(evidence, value.as_bytes());
        }
        for (pid, creation) in pairs { frame(evidence, &pid.to_be_bytes()); frame(evidence, &creation.to_be_bytes()); }
    }
    for instance in global_bare {
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
            let original_login=original_login_suffix.is_some_and(|suffix|suffix.len()==40
                && suffix.bytes().all(|byte|byte.is_ascii_hexdigit()));
            let stopped_proof=c[11].len()==71 && c[11].starts_with("sha256:")
                && c[11][7..].bytes().all(|byte|byte.is_ascii_hexdigit());
            if c[7] != instance || c[8] != "global" || c[0].is_empty() || c[1].is_empty()
                || c[2].is_empty() || c[5].is_empty() || c[6].len() != 71
                || !c[6].starts_with("sha256:") || !c[6][7..].bytes().all(|byte|byte.is_ascii_hexdigit())
                || c[9].parse::<u64>().ok().filter(|v| *v != 0).is_none()
                || !expected.iter().any(|row| row[0] == c[0] && row[3] == c[9]
                    && row[4] == c[10] && row[5] == c[11] && row[6] == c[3] && row[7] == c[4])
                || (c[10] == "UNKNOWN" && (!c[11].is_empty()
                    || (!seen_h.contains(&c[0]) && !original_login)))
                || (c[10] == "STOPPED" && (!stopped_proof
                    || (!seen_h.contains(&c[0]) && !original_login)))
                || !matches!(c[10].as_str(), "STOPPED" | "UNKNOWN") { return Err(denied()); }
            let pid = c[3].parse::<u32>().map_err(|error| fail("global original PID", error))?;
            let created = c[4].parse::<u64>().map_err(|error| fail("global original FILETIME", error))?;
            if pid == 0 || created == 0 || c[3] != pid.to_string() || c[4] != created.to_string()
                || (c[10] == "UNKNOWN" && pairs.contains(&(pid, created))) { return Err(denied()); }
            pairs.push((pid, created));
            for field in c { frame(&mut source, field.as_bytes()); }
        }
        frame(evidence, sha256_hex(&source).as_bytes());
        if !pairs.is_empty() {
            let fact = NativeLegacyHoldersGone::observe(&pairs)
                .map_err(|error| fail("global kernel fact", error))?;
            fact.validate(&pairs).map_err(|error| fail("global kernel recheck", error))?;
            frame(evidence, &fact.boot_start().to_be_bytes());
            for (pid, created) in pairs {
                frame(evidence, &pid.to_be_bytes()); frame(evidence, &created.to_be_bytes());
            }
        }
        for c in original { qualified_operations.insert(c[0].clone()); }
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
    Ok((qualified_bindings,qualified_operations))
}

/// A released reservation with no process is settled only by its original
/// reserve/commit/Owner-release history and a complete absence of launch
/// producers. An empty native pair list is never a holder-gone observation.
fn qualified_released_never_started(db: &VerifiedDatabaseConnection<'_>,
    instance: &str, domain: &str, session: &str, home: &str,
    binding: &str, generation: &str, qualified_operations: &BTreeSet<String>) -> Result<()> {
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
        ("SELECT 1 FROM main.gogoke_v37_h_generation WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_v37_h_dos_alias WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
        ("SELECT 1 FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND session_id=?2 LIMIT 1",vec![domain,session]),
    ] { if any(db,sql,&values)? { return Err(denied()); } }
    qualified_custody_disjoint_from_session(db,instance,domain,session,generation,
        qualified_operations)?;
    let history = rows(db,
        "SELECT operation,raw_hex,status,CAST(previous_revision AS TEXT),CAST(revision AS TEXT),request_id FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND session_id=?2 ORDER BY revision",
        &[domain,session],6)?;
    let operations:&[&str]=match history.len() {
        2=>&["admission-reserve","admission-release"],
        3=>&["admission-reserve","admission-commit","admission-release"],
        _=>return Err(denied()),
    };
    if claims[0][1]!=history.len().to_string() { return Err(denied()); }
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
        || fields[7]!=generation.as_bytes() {
        return Err(denied());
    }
    let seat = rows(db,
            "SELECT seat_id,seat_incarnation,CAST(seat_authorization_generation AS TEXT),selected_instance_id,'NATIVE_V2' FROM main.gogoke_v37_native_selection WHERE domain_id=?1 AND session_id=?2 UNION ALL SELECT b.seat_id,b.seat_incarnation,b.generation,c.instance_id,'LEGACY_V1' FROM main.gogoke_v37_h_seat_binding b JOIN main.gogoke_v37_h_claim c ON c.domain_id=b.domain_id AND c.session_id=b.session_id WHERE b.domain_id=?1 AND b.session_id=?2 AND NOT EXISTS(SELECT 1 FROM main.gogoke_v37_native_selection n WHERE n.domain_id=b.domain_id AND n.session_id=b.session_id)",
            &[domain,session],5)?;
    if seat.len()!=1 || seat[0][3]!=instance || seat[0][0].is_empty()
        || seat[0][1].is_empty() || seat[0][2]!=generation { return Err(denied()); }
    // Match the original native admission naming and SID producer, rather
    // than accepting an arbitrary HOME/binding or an AppContainer SID prefix.
    let suffix=sha256_hex(format!("{}\n{domain}\n{session}\n{}\n{generation}",
        db.root_identity().opaque(),seat[0][1]).as_bytes());
    let suffix=&suffix[..40];
    if home!=format!("home-{suffix}") || binding!=format!("binding-{suffix}") {
        return Err(denied());
    }
    let profile=crate::process::AppContainerProfile::derive_for_revocation(
        &format!("Gogoke37.Session.{suffix}"))
        .map_err(|error|fail("original reservation HOME profile",error))?;
    let sid=profile.sid_identity().map_err(|error|
        fail("original reservation HOME SID",error))?;
    if fields[8]!=sid.as_bytes() { return Err(denied()); }
    if any(db,"SELECT 1 FROM main.gogoke_v37_session_binding_v2 WHERE domain_id=?1 AND session_id=?2 LIMIT 1",
        &[domain,session])? { return Err(denied()); }
    if seat[0][4]=="NATIVE_V2" {
        // Native reserve persists only its pending selection. The durable v2
        // session binding is an OPEN-side effect and cannot exist for a truly
        // never-started release; neither can a legacy seat binding.
        if any(db,"SELECT 1 FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2 LIMIT 1",
            &[domain,session])? { return Err(denied()); }
    } else if seat[0][4]!="LEGACY_V1" { return Err(denied()); }
    // Historical USER admission used the exact two fields on every domain.
    // The later empty global payload belongs only to the designated native
    // Secretary selection; it does not replace those original raw frames.
    let native_secretary = domain=="global" && seat[0][4]=="NATIVE_V2"
        && any(db,"SELECT 1 FROM main.gogoke_v37_seat_secretary WHERE domain_id=?1 AND seat_id=?2 AND incarnation=?3",
            &[domain,&seat[0][0],&seat[0][1]])?;
    for (index,row) in history.iter().enumerate() {
        let operation=operations[index];
        if row[0]!=operation || row[2]!="APPLIED" || row[3]!=index.to_string()
            || row[4]!=(index+1).to_string() { return Err(denied()); }
        let raw=unhex(&row[1])?;
        let request=crate::store::session_transport::decode_request(&raw)
            .map_err(|error|fail("original never-started H request",error))?;
        let payload_matches=(native_secretary && request.payload.is_empty())
            || (request.payload.len()==2
                && request_field(&request,"seatId").as_deref()==Some(seat[0][0].as_str())
                && request_field(&request,"generation").as_deref()==Some(generation));
        if request.family!="K-SESSION" || request.operation!=operation
            || request.domain_id!=domain || request.target_id!=session
            || request.request_id!=row[5] || request.expected_revision!=index as u64
            || !payload_matches {
            return Err(denied());
        }
    }
    Ok(())
}

fn qualified_custody_disjoint_from_session(db: &VerifiedDatabaseConnection<'_>,
    instance: &str, domain: &str, session: &str, generation: &str,
    qualified_operations: &BTreeSet<String>) -> Result<()> {
    let custody=rows(db,
        "SELECT operation_id FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND domain_id=?2 AND generation=?3 ORDER BY operation_id",
        &[instance,domain,generation],1)?;
    for c in custody {
        if !qualified_operations.contains(&c[0]) { return Err(denied()); }
        let episode=rows(db,
            "SELECT session_id,generation FROM main.gogoke_v37_h_process_episode WHERE process_operation_id=?1 AND instance_id=?2 AND domain_id=?3",
            &[&c[0],instance,domain],2)?;
        if episode.len()>1 { return Err(denied()); }
        if let Some(e)=episode.first() {
            if e[0]==session || e[1]!=generation { return Err(denied()); }
        } else if domain!="global" { return Err(denied()); }
    }
    Ok(())
}

fn qualified_resources(db: &VerifiedDatabaseConnection<'_>,
    qualified_operations: &BTreeSet<String>) -> Result<()> {
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
        // A generation without its exact original episode has no instance
        // owner to classify. Refuse it before the Codex joins can hide it.
        ("SELECT 1 FROM main.gogoke_v37_h_generation g WHERE NOT EXISTS(SELECT 1 FROM main.gogoke_v37_h_process_episode e WHERE e.domain_id=g.domain_id AND e.session_id=g.session_id AND e.generation=g.generation AND e.request_id=g.request_id AND e.process_operation_id=g.process_operation_id) LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id='codex' AND (a.cleanup_status IN ('NOT_REQUESTED','UNKNOWN') OR a.phase IN ('INTENT','PREPARE_STARTED','UNKNOWN')) LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id='codex' AND h.state IN ('PREPARING','CLOSE_UNKNOWN','CLEANUP_UNKNOWN','UNKNOWN') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_objects o JOIN main.gogoke_v37_instances i ON i.instance_id=o.instance_id WHERE i.driver_id='codex' AND o.phase!='ACTIVE' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_aliases a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id='codex' AND a.state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id WHERE i.driver_id='codex' AND p.state!='REVOKED' LIMIT 1", vec![]),
    ] {
        if any(db, sql, &values)? { return Err(denied()); }
    }
    let claims = rows(db,
        "SELECT c.instance_id,c.domain_id,c.session_id,c.state,COALESCE(c.process_operation_id,''),COALESCE(c.stop_fact_id,''),c.generation,c.binding_id,c.home_id FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id='codex'", &[], 9)?;
    for c in &claims {
        if c[4].is_empty() {
            if c[3]!="RELEASED" || !c[5].is_empty() || any(db,
                "SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2 LIMIT 1",
                &[&c[1],&c[2]])? { return Err(denied()); }
            qualified_released_never_started(db,&c[0],&c[1],&c[2],&c[8],&c[7],&c[6],
                qualified_operations)?;
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
            "SELECT e.process_operation_id,c.state,COALESCE(c.stop_proof_hash,''),e.binding_id,e.phase,COALESCE(e.stop_fact_id,''),e.request_id,e.raw_hex FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5", &[&h[0], &h[1], &h[2], &h[3], &h[4]], 8)?;
        if execution.len()>1 { return Err(denied()); }
        if let Some(e)=execution.first() {
            if e[1]=="STOPPED" && !e[2].is_empty() && e[4]=="STOPPED" && e[5]==e[2] {
                qualified_stopped_h(db,&e[0],&h[1],&h[2],&h[4],&e[2])?;
            } else if e[1]=="UNKNOWN" && e[2].is_empty()
                && matches!(e[4].as_str(),"ACTIVE"|"UNKNOWN") && e[5].is_empty() {
                qualified_unknown_home_source(db,&h[0],&h[1],&h[2],&h[3],&h[4],
                    &e[6],&e[7])?;
                let (fact,_,pairs)=crate::store::session_transport::observe_exact_home_gone(
                    db,&h[0],&h[1],&h[2],&h[3],&h[4],&e[3])
                    .map_err(|error|fail("ACTIVE HOME original execution",error))?;
                fact.validate(&pairs).map_err(|error|fail("ACTIVE HOME kernel recheck",error))?;
            } else { return Err(denied()); }
        } else {
            let claim = rows(db,"SELECT binding_id,state,COALESCE(process_operation_id,'') FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND domain_id=?2 AND session_id=?3 AND home_id=?4 AND generation=?5",
                &[&h[1],&h[2],&h[3],&h[0],&h[4]],3)?;
            if claim.len()!=1 || claim[0][1]!="RELEASED" || !claim[0][2].is_empty() { return Err(denied()); }
            qualified_released_never_started(db,&h[1],&h[2],&h[3],&h[0],&claim[0][0],&h[4],
                qualified_operations)?;
        }
    }
    Ok(())
}

/// ACTIVE is the durable owner-binding state, not a statement that this is the
/// current generation. A retained historical binding is accepted only after
/// its exact episode has been independently settled by qualified_executions.
fn qualified_owner_bindings(db: &VerifiedDatabaseConnection<'_>,
    historical: &BTreeSet<String>) -> Result<()> {
    let claims = rows(db,
        "SELECT c.instance_id,c.domain_id,c.session_id,c.generation,c.binding_id FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id='codex'", &[], 5)?;
    let bindings = rows(db,
        "SELECT b.instance_id,b.binding_id,b.domain_id,b.owner_id,b.generation,b.state FROM main.gogoke_v37_h_owner_binding b JOIN main.gogoke_v37_instances i ON i.instance_id=b.instance_id WHERE i.driver_id='codex'", &[], 6)?;
    for b in bindings {
        if b[5]!="ACTIVE" { continue; }
        let current=claims.iter().any(|c|c[0]==b[0] && c[1]==b[2] && c[2]==b[3]
            && c[3]==b[4] && c[4]==b[1]);
        if !current && !historical.contains(&b[1]) { return Err(denied()); }
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

fn qualified_unfenced_legacy_family(
    db: &VerifiedDatabaseConnection<'_>, target: &str,
) -> Result<()> {
    if legacy_fence::read_legacy_step(db, target, legacy_fence::LegacyAclStep::Home)
        .map_err(|error| fail("unbound HOME ACL step", error))?.is_some()
        || legacy_fence::read_legacy_step(db, target, legacy_fence::LegacyAclStep::Baseline)
            .map_err(|error| fail("unbound baseline step", error))?.is_some()
        || legacy_fence::legacy_fence_custody_present_for_source_migration(db, target)
            .map_err(|error| fail("unbound legacy ACL custody", error))? {
        return Err(denied());
    }
    Ok(())
}

fn qualify(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &LegacySourceRequest<'_>) -> Result<Qualification> {
    valid_request(input)?;
    exact_physical_authority(db, root, input)?;
    let legacy_fence_schema_present =
        legacy_fence::legacy_fence_schema_present_for_source_migration(db)
            .map_err(|error| fail("legacy ACL schema", error))?;
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
    let copy = rows(db,
        "SELECT state,COALESCE(version,''),COALESCE(stage_name,''),COALESCE(raw_error,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id='codex'", &[], 4)?;
    if copy.len()!=1 { return Err(denied()); }
    if copy[0][0]!="READY" {
        return if copy[0][3].is_empty() { Err(denied()) }
            else { Err(fail("READY copy original error", &copy[0][3])) };
    }
    if copy[0][1]!=pin.version || copy[0][2].is_empty() { return Err(denied()); }
    let stage = copy[0][2].clone();
    let path = managed_cli::locate_ready_managed_program_from_db(db, "codex", &digest, pin.version)
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
    let fence = if legacy_fence_schema_present {
        legacy_fence::read_legacy_fence(db, input.target_id)
            .map_err(|error| fail("prior legacy ACL fence", error))?
    } else { None };
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
        if legacy_fence_schema_present {
            qualified_unfenced_legacy_family(db, input.target_id)?;
        }
        LegacyAclInventory::capture(root, &home.path, &home.identity,
            &binding, &names).map_err(|error| fail("fresh physical historical HOME ACL", error))?
    };
    let mut evidence = Vec::new();
    let (historical_bindings,qualified_operations)=qualified_executions(db, &mut evidence)?;
    qualified_resources(db,&qualified_operations)?;
    qualified_owner_bindings(db,&historical_bindings)?;
    for value in [inventory.original_digest(), inventory.home_original_digest(),
        inventory.source_original_digest(), provenance,
        source_identity.opaque()] { frame(&mut evidence, value.as_bytes()); }
    let legacy_fence_schema_after =
        legacy_fence::legacy_fence_schema_present_for_source_migration(db)
            .map_err(|error| fail("legacy ACL schema recheck", error))?;
    if legacy_fence_schema_after != legacy_fence_schema_present { return Err(denied()); }
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

// The first reservation is the only point where an unknown COMMIT can leave
// no durable request to reconcile. Preserve SQLite's original error while
// making that admission boundary explicit to the caller.
fn admission_commit(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    #[cfg(all(test, windows))]
    if let Some(commit_first) = FIRST_ADMISSION_COMMIT_FAULT.with(|fault| fault.take()) {
        // These are distinct real SQLite transaction controls. The synthetic
        // error represents a lost/error return; readback asserts which control
        // actually reached the database rather than guessing from the return.
        db.execute(if commit_first { "COMMIT" } else { "ROLLBACK" })
            .map_err(|error| fail("admission test transaction control", error))?;
        return Err(fail("ADMISSION_COMMIT_UNKNOWN", SameOpenError::SqliteExec {
            code: 5, message: "controlled first COMMIT return error".to_owned(),
        }));
    }
    db.execute("COMMIT")
        .map_err(|error| fail("ADMISSION_COMMIT_UNKNOWN", error))
}

#[cfg(all(test, windows))]
std::thread_local! {
    static FIRST_ADMISSION_COMMIT_FAULT: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

#[cfg(all(test, windows))]
pub(crate) fn test_first_admission_commit_error_after(commit_first: bool) {
    FIRST_ADMISSION_COMMIT_FAULT.with(|fault| {
        assert!(fault.get().is_none(), "one admission fault at a time");
        fault.set(Some(commit_first));
    });
}

fn admission_transaction<T>(db: &mut VerifiedDatabaseConnection<'_>,
    work: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T>) -> Result<T> {
    db.execute("BEGIN IMMEDIATE").map_err(|error| fail("admission begin", error))?;
    match work(db) {
        Ok(value) => { admission_commit(db)?; Ok(value) },
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(fail("admission rollback unknown", (original, rollback))),
        },
    }
}

pub(crate) fn migrate_legacy_codex_source(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, input: &LegacySourceRequest<'_>,
    applied_receipt: &[u8]) -> Result<LegacySourceResult> {
    valid_request(input)?;
    exact_physical_authority(db, root, input)?;
    if let Some(prior) = read_journal(db, input.request_id)? {
        if !same_intent(db, input, &prior) { return Err(denied()); }
        return applied_readback(db, root, input, &prior, applied_receipt);
    }
    let initial = qualify(db, root, input)?;
    admission_transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        if read_journal(db, input.request_id)?.is_some() || any(db,
            "SELECT 1 FROM main.gogoke_v37_instance_legacy_source_migrations LIMIT 1", &[])? ||
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
    // A successful COMMIT return is necessary but the committed intent must
    // also be readable from this verified physical database before SOURCE.
    let committed = read_journal(db, input.request_id)?.ok_or_else(denied)?;
    if !same_intent(db, input, &committed) || committed.phase != "PREPARING" ||
        committed.qualification_digest != initial.qualification_digest ||
        !source_rows(db, input.target_id)?.is_empty() {
        return Err(denied());
    }
    // Only this uninterrupted invocation can enter the effect transaction.
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let prior = read_journal(db, input.request_id)?.ok_or_else(denied)?;
        if !same_intent(db, input, &prior) || prior.phase != "PREPARING" ||
            !source_rows(db, input.target_id)?.is_empty() ||
            any(db, "SELECT 1 FROM main.gogoke_v37_instance_legacy_source_migrations WHERE request_id<>?1 LIMIT 1",
                &[input.request_id])? { return Err(denied()); }
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

    fn confirm_durable_stop(custodian:&mut crate::process::ProcessCustodian,
        prepared:&crate::process::PreparedCustody,proof:&crate::process::NativeStopProof,
        revision:u64) {
        assert!(custodian.active(&prepared.ticket).is_some(),
            "native custody stays held until exact durable confirmation");
        let mut wrong_proof=proof.proof_hash();
        let replacement=if wrong_proof.as_bytes()[7]==b'0' {"1"} else {"0"};
        wrong_proof.replace_range(7..8,replacement);
        assert!(custodian.confirm_stop_durable(&crate::process::DurableStopConfirmation {
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:wrong_proof,
            durable_revision:revision,
        }).is_err(),"changed proof cannot release retained native custody");
        assert!(custodian.active(&prepared.ticket).is_some());
        assert!(custodian.confirm_stop_durable(&crate::process::DurableStopConfirmation {
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:proof.proof_hash(),
            durable_revision:0,
        }).is_err(),"zero is not a durable authority revision");
        assert!(custodian.active(&prepared.ticket).is_some());
        let confirmed=custodian.confirm_stop_durable(&crate::process::DurableStopConfirmation {
            ticket:prepared.ticket.clone(),custodian_nonce:prepared.custodian_nonce.clone(),
            identity:prepared.identity.clone(),proof_hash:proof.proof_hash(),
            durable_revision:revision,
        }).unwrap();
        assert_eq!(confirmed.proof_hash(),proof.proof_hash());
        assert!(custodian.active(&prepared.ticket).is_none(),
            "durably confirmed stop releases the exact active ticket");
    }

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

    const LEGACY_FENCE_TABLES: [&str; 3] = [
        "gogoke_v37_legacy_acl_fences",
        "gogoke_v37_legacy_acl_custody",
        "gogoke_v37_legacy_acl_steps",
    ];

    fn assert_legacy_schema_refused(db: &VerifiedDatabaseConnection<'_>) {
        assert!(matches!(
            legacy_fence::legacy_fence_schema_present_for_source_migration(db),
            Err(legacy_fence::LegacyFenceError::Schema)
        ));
    }

    #[test]
    pub(super) fn optional_legacy_fence_family_classifier_matrix() {
        fixture(|_, mut db| {
            assert!(!legacy_fence::legacy_fence_schema_present_for_source_migration(&db).unwrap());
            legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
            assert!(legacy_fence::legacy_fence_schema_present_for_source_migration(&db).unwrap());
            assert!(!legacy_fence::legacy_fence_custody_present_for_source_migration(&db, "oldA").unwrap());
            qualified_unfenced_legacy_family(&db, "oldA").unwrap();
            db.close_checked().unwrap();
        });

        for step in ["HOME", "BASELINE"] {
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute("PRAGMA foreign_keys=OFF").unwrap();
                db.execute(&format!(
                    "INSERT INTO main.gogoke_v37_legacy_acl_steps VALUES('oldA','{step}','APPLIED','requestA','bootA','a','b','c','d',2,3)"
                )).unwrap();
                assert!(qualified_unfenced_legacy_family(&db, "oldA").is_err(),
                    "a complete family cannot hide an orphan {step} step");
                db.close_checked().unwrap();
            });
        }
        fixture(|_, mut db| {
            legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
            db.execute("PRAGMA foreign_keys=OFF").unwrap();
            db.execute("INSERT INTO main.gogoke_v37_legacy_acl_custody VALUES('oldA','operationA','ticketA','nonceA','1','1','imageA','digestA','oldA','global','1','UNKNOWN',NULL)").unwrap();
            assert!(qualified_unfenced_legacy_family(&db, "oldA").is_err(),
                "a complete family cannot hide orphan captured custody");
            db.close_checked().unwrap();
        });

        for name in LEGACY_FENCE_TABLES {
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!("DROP TABLE main.{name}")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!("DROP TABLE main.{name}")).unwrap();
                db.execute(&format!("CREATE TABLE main.{name}(instance_id TEXT) STRICT")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!("DROP TABLE main.{name}")).unwrap();
                db.execute(&format!("CREATE VIEW main.{name} AS SELECT 'oldA' AS instance_id")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                db.execute(&format!("CREATE TEMP TABLE {name}(instance_id TEXT)")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                db.execute(&format!("CREATE TEMP VIEW {name} AS SELECT 'oldA' AS instance_id")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!("CREATE TEMP TABLE {name}(instance_id TEXT)")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!("CREATE TEMP VIEW {name} AS SELECT 'oldA' AS instance_id")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!(
                    "CREATE TRIGGER unexpected_{name} BEFORE UPDATE ON {name} BEGIN SELECT 1; END"
                )).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!(
                    "CREATE TEMP TRIGGER temp_unexpected_{name} BEFORE UPDATE ON main.{name} BEGIN SELECT 1; END"
                )).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                legacy_fence::initialize_legacy_fence_schema(&mut db).unwrap();
                db.execute(&format!("CREATE INDEX unexpected_{name} ON {name}(instance_id)")).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
            fixture(|_, mut db| {
                db.execute("CREATE TEMP TABLE unrelated_legacy_index_target(instance_id TEXT)").unwrap();
                db.execute(&format!(
                    "CREATE INDEX temp.gogoke_v37_legacy_acl_unexpected_{name} ON unrelated_legacy_index_target(instance_id)"
                )).unwrap();
                assert_legacy_schema_refused(&db);
                db.close_checked().unwrap();
            });
        }

        fixture(|_, mut db| {
            db.execute("CREATE TABLE main.GOGOKE_V37_LEGACY_ACL_EXTRA(instance_id TEXT) STRICT").unwrap();
            assert_legacy_schema_refused(&db);
            db.close_checked().unwrap();
        });
    }

    #[test]
    pub(super) fn plain_resume_accepts_only_direct_or_original_unknown_promotion_equation() {
        assert!(qualified_plain_resume_revisions(3,3,4),
            "direct promote keeps original previous and writes expected+1");
        assert!(qualified_plain_resume_revisions(3,4,5),
            "mark_resume_unknown then promote copies original result to previous and writes expected+2");
        for (previous,result) in [(2,3),(3,3),(3,5),(4,4),(4,6),(5,6)] {
            assert!(!qualified_plain_resume_revisions(3,previous,result),
                "no third revision shape may be inferred");
        }
    }

    #[test]
    fn preparing_is_durable_refusal_and_applied_without_source_is_not_a_receipt() {
        fixture(|root, mut db| {
            let owner = crate::store::authority::initialize_profile(&mut db, root).unwrap();
            let expected_root = db.root_identity().opaque();
            let expected_database = db.identity().opaque();
            let raw_frame = format!(r#"{{"schema":"gogoke.37.managed-cli.v1","command":"migrate-legacy-codex-source","driverId":"codex","requestId":"newA","targetId":"oldA","originalRequestId":"registerA","originalRequestFingerprint":"abcd","originalNativeReceiptHex":"686f6d65","originalHomeIdentity":"home","expectedRootIdentity":"{expected_root}","expectedDatabaseIdentity":"{expected_database}"}}"#);
            let raw = raw_frame.as_bytes();
            let input = LegacySourceRequest { raw, request_id:"newA", target_id:"oldA",
                original_request_id:"registerA", original_fingerprint:"abcd",
                original_receipt_hex:"686f6d65", original_home_identity:"home",
                expected_root_identity:&expected_root, expected_database_identity:&expected_database };
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
                original_home_identity:input.original_home_identity,
                expected_root_identity:input.expected_root_identity,
                expected_database_identity:input.expected_database_identity };
            assert!(!same_intent(&db, &changed, &prior));
            let changed_frame = raw_frame.replace(
                &format!("\"expectedDatabaseIdentity\":\"{expected_database}\""),
                "\"expectedDatabaseIdentity\":\"wrong-database\"");
            assert_ne!(changed_frame.as_bytes(), raw);
            let changed_anchor = LegacySourceRequest { raw: changed_frame.as_bytes(),
                request_id:input.request_id, target_id:input.target_id,
                original_request_id:input.original_request_id,
                original_fingerprint:input.original_fingerprint,
                original_receipt_hex:input.original_receipt_hex,
                original_home_identity:input.original_home_identity,
                expected_root_identity:input.expected_root_identity,
                expected_database_identity:"wrong-database" };
            assert_ne!(prior.raw_hex, hex(changed_anchor.raw),
                "the same request ID cannot reuse its original raw fingerprint after an anchor change");
            assert!(!same_intent(&db, &changed_anchor, &prior),
                "changing an expected anchor under the same ID changes the original raw intent");
            assert!(migrate_legacy_codex_source(&mut db, root, &owner,
                &changed_anchor, b"receipt").is_err());
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
    fn applied_replay_without_original_registration_refuses() {
        fixture(|root, mut db| {
            let home = super::super::home::prepare_persistent_home(root, "oldA").unwrap();
            let identity = home.identity.opaque();
            let expected_root = db.root_identity().opaque();
            let expected_database = db.identity().opaque();
            let raw_frame = format!(r#"{{"schema":"gogoke.37.managed-cli.v1","command":"migrate-legacy-codex-source","driverId":"codex","requestId":"newA","targetId":"oldA","originalRequestId":"registerA","originalRequestFingerprint":"abcd","originalNativeReceiptHex":"686f6d65","originalHomeIdentity":"{identity}","expectedRootIdentity":"{expected_root}","expectedDatabaseIdentity":"{expected_database}"}}"#);
            let raw = raw_frame.as_bytes();
            let receipt = b"exact original APPLIED receipt";
            let input = LegacySourceRequest { raw, request_id:"newA", target_id:"oldA",
                original_request_id:"registerA", original_fingerprint:"abcd",
                original_receipt_hex:"686f6d65", original_home_identity:&identity,
                expected_root_identity:&expected_root, expected_database_identity:&expected_database };
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
            assert!(applied_readback(&db, root, &input, &prior, receipt).is_err());
            std::fs::remove_file(home.path.join("gogoke-instance.marker")).unwrap();
            std::fs::remove_dir(home.path).unwrap();
            std::fs::remove_dir(root.canonical_root().canonical_path.join("v37-instances")).unwrap();
            db.close_checked().unwrap();
        });
    }

    #[test]
    pub(super) fn global_native_never_started_release_uses_original_empty_payload() {
        for (committed,payload,source,mutation,accepted) in [
            (false,"{}","native","none",true),
            (true,"{}","native","none",true),
            (false,r#"{"seatId":"seatA","generation":"1"}"#,"native","none",true),
            (false,r#"{"seatId":"wrongSeat","generation":"1"}"#,"native","none",false),
            (false,r#"{"seatId":"seatA","generation":"2"}"#,"native","none",false),
            (false,"{}","native","missing-secretary",false),
            (false,"{}","native","missing-incarnation",false),
            (false,r#"{"seatId":"seatA","generation":"1"}"#,"legacy","none",true),
            (false,"{}","legacy","none",false),
        ] {
            fixture(|root, mut db| {
                crate::store::orchestration::apply_orchestration_slice_schema(&mut db).unwrap();
                let owner=crate::store::authority::initialize_profile(&mut db,root).unwrap();
                super::super::initialize_credential_schema(&mut db).unwrap();
                crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
                crate::store::seat::initialize_schema(&mut db).unwrap();
                crate::store::session_transport::initialize_admission_schema(&mut db).unwrap();
                crate::store::session_transport::session_binding::initialize_schema(&mut db).unwrap();
                let persistent=super::super::home::prepare_persistent_home(root,"oldA").unwrap();
                let instance=Statement::prepare(db.as_ptr(),
                    "INSERT INTO main.gogoke_v37_instances VALUES('oldA','codex','instance-home-oldA',?1,'sha256:0000000000000000000000000000000000000000000000000000000000000000','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
                instance.bind_text(1,&persistent.identity.opaque()).unwrap();
                instance.step_done().unwrap();drop(instance);
                // These are the exact durable E rows produced by designation
                // and configuration. Admission, F HOME and release below use
                // their production writers and preserve the original raw.
                db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','seatA','incA','USER','LONG','oldA','BUSY',1,1)").unwrap();
                db.execute("INSERT INTO main.gogoke_v37_seat_secretary VALUES(1,'global','seatA','incA','designationA','original-secretary-fixture')").unwrap();
                let session=if committed {"committedRelease"} else {"directRelease"};
                let reserve_id=if committed {"commitReserve"} else {"directReserve"};
                let suffix=sha256_hex(format!("{}\nglobal\n{session}\nincA\n1",
                    root.canonical_root().identity.opaque()).as_bytes());
                let suffix=&suffix[..40];
                let home_id=format!("home-{suffix}");
                let binding_id=format!("binding-{suffix}");
                let profile=crate::process::AppContainerProfile::derive_for_revocation(
                    &format!("Gogoke37.Session.{suffix}")).unwrap();
                let reserve_raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-reserve\",\"requestId\":\"{reserve_id}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"0\",\"payload\":{payload}}}");
                let preparation=sha256_hex(format!("global\n{reserve_id}").as_bytes());
                let preparation_id=format!("homeprep-{}",&preparation[..40]);
                db.execute("BEGIN IMMEDIATE").unwrap();
                crate::store::session_transport::bind_owner_in_transaction(&mut db,
                    &crate::store::session_transport::OwnerBinding {binding_id:&binding_id,
                        instance_id:"oldA",domain_id:"global",kind:"SESSION",
                        owner_id:session,generation:"1"}).unwrap();
                db.execute("COMMIT").unwrap();
                super::super::temporary::create_temporary_home(&mut db,root,&profile,
                    &super::super::temporary::CreateTemporaryHome {request_id:&preparation_id,
                        request_bytes:reserve_raw.as_bytes(),home_id:&home_id,instance_id:"oldA",
                        domain_id:"global",kind:super::super::temporary::TemporaryKind::Session,
                        owner_id:session,generation:"1"}).unwrap();
                let selection=crate::store::session_transport::session_binding::SessionBinding {
                    domain_id:"global".into(),session_id:session.into(),seat_id:"seatA".into(),
                    seat_incarnation:"incA".into(),seat_authorization_generation:1,
                    selected_instance_id:"oldA".into(),
                    provenance:crate::store::session_transport::session_binding::Provenance::NativeV2,
                };
                if source=="legacy" {
                    db.execute(&format!("INSERT INTO main.gogoke_v37_h_seat_binding(domain_id,session_id,seat_id,seat_incarnation,generation) VALUES('global','{session}','seatA','incA','1')")).unwrap();
                }
                let reserve=crate::store::session_transport::AdmissionRequest {domain_id:"global",
                    session_id:session,request_id:reserve_id,raw_bytes:reserve_raw.as_bytes(),
                    instance_id:"oldA",home_id:&home_id,generation:"1",expected_revision:0};
                assert!(matches!(crate::store::session_transport::reserve_admission(&mut db,&reserve,
                    |db| {
                        if source=="native" {
                            crate::store::session_transport::session_binding::select_native_in_transaction(
                                db,&selection).map_err(crate::store::session_transport::AdmissionError::Relationship)?;
                        }
                        Ok(crate::store::session_transport::TrustedLimits {
                            project_parallel:4,instance_concurrency:4})
                    }).unwrap(),crate::store::session_transport::AdmissionResult::Applied(1)));
                let release_revision=if committed {
                    let raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-commit\",\"requestId\":\"commitA\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"1\",\"payload\":{payload}}}");
                    let input=crate::store::session_transport::AdmissionRequest {domain_id:"global",
                        session_id:session,request_id:"commitA",raw_bytes:raw.as_bytes(),
                        instance_id:"oldA",home_id:&home_id,generation:"1",expected_revision:1};
                    assert!(matches!(crate::store::session_transport::commit_admission(&mut db,&input,
                        |_|Ok(())).unwrap(),crate::store::session_transport::AdmissionResult::Applied(2)));
                    2
                } else {1};
                let release_id=if committed {"commitRelease"} else {"directReleaseA"};
                let raw=format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-SESSION\",\"operation\":\"admission-release\",\"requestId\":\"{release_id}\",\"targetId\":\"{session}\",\"domainId\":\"global\",\"expectedRevision\":\"{release_revision}\",\"payload\":{payload}}}");
                let input=crate::store::session_transport::AdmissionRequest {domain_id:"global",
                    session_id:session,request_id:release_id,raw_bytes:raw.as_bytes(),
                    instance_id:"oldA",home_id:&home_id,generation:"1",
                    expected_revision:release_revision};
                let released=if source=="native" {
                    crate::store::session_transport::runtime::release_native(&mut db,
                        &crate::store::seat::NativeOrigin::user(&owner),&input)
                } else {
                    crate::store::session_transport::release_admission(&mut db,&input,|_|Ok(()))
                };
                assert!(matches!(released.unwrap(),
                    crate::store::session_transport::AdmissionResult::Applied(_)));
                match mutation {
                    "missing-secretary"=>db.execute("DELETE FROM main.gogoke_v37_seat_secretary").unwrap(),
                    "missing-incarnation"=>db.execute(&format!("UPDATE main.gogoke_v37_native_selection SET seat_incarnation='missingIncarnation' WHERE domain_id='global' AND session_id='{session}'")).unwrap(),
                    "none"=>{},
                    _=>panic!("unknown fixture mutation"),
                }
                let (_,operations)=qualified_executions(&db,&mut Vec::new()).unwrap();
                assert_eq!(qualified_resources(&db,&operations).is_ok(),accepted,
                    "global historical two-field and current native Secretary sources stay distinct");
                if accepted && source=="native" && payload=="{}" && !committed {
                    db.execute(&format!("INSERT INTO main.gogoke_v37_h_seat_binding(domain_id,session_id,seat_id,seat_incarnation,generation) VALUES('global','{session}','seatA','incA','1')")).unwrap();
                    db.execute(&format!("UPDATE main.gogoke_v37_native_selection SET selected_instance_id='wrongInstance' WHERE domain_id='global' AND session_id='{session}'")).unwrap();
                    assert!(qualified_resources(&db,&operations).is_err(),
                        "a bad native Secretary selection cannot fall through to legacy identity");
                }
                db.close_checked().unwrap();
            });
        }
    }

    #[test]
    pub(super) fn stopped_h_requires_original_raw_home_binding_generation_and_c_proof() {
        fixture(|root, mut db| {
            // This fixture drives the real ProductDatabase H/C/ledger producers
            // on a create_new connection. Reproduce their complete schema
            // prerequisites with production initializers; no test DDL stands
            // in for the product opener.
            crate::store::orchestration::apply_orchestration_slice_schema(&mut db).unwrap();
            let owner=crate::store::authority::initialize_profile(&mut db,root).unwrap();
            super::super::initialize_credential_schema(&mut db).unwrap();
            crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
            crate::store::seat::initialize_schema(&mut db).unwrap();
            crate::store::session_transport::initialize_admission_schema(&mut db).unwrap();
            crate::store::session_transport::session_binding::initialize_schema(&mut db).unwrap();
            crate::store::ledger::initialize_schema(&mut db).unwrap();
            crate::store::session_transport::rpc_journal::initialize_schema(&mut db).unwrap();
            let command=std::path::PathBuf::from(std::env::var_os("SystemRoot").unwrap())
                .join("System32").join("cmd.exe");
            let command_digest=crate::store::digest::content_hash(&std::fs::read(&command).unwrap());
            let persistent=super::super::home::prepare_persistent_home(root,"oldA").unwrap();
            let persistent_identity=persistent.identity.opaque();
            let instance=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instances VALUES('oldA','codex','instance-home-oldA',?1,?2,'0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
            instance.bind_text(1,&persistent_identity).unwrap();
            instance.bind_text(2,&command_digest).unwrap();instance.step_done().unwrap();drop(instance);
            let incarnation="incA";
            db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','seatA','incA','USER','LONG','oldA','BUSY',1,1)").unwrap();
            db.execute("INSERT INTO main.gogoke_v37_seat_secretary VALUES(1,'global','seatA','incA','designationA','original-secretary-fixture')").unwrap();
            let suffix=sha256_hex(format!("{}\nglobal\nsessionA\n{incarnation}\n1",
                root.canonical_root().identity.opaque()).as_bytes());
            let suffix=&suffix[..40];
            let home_id=format!("home-{suffix}");
            let binding_id=format!("binding-{suffix}");
            let profile=crate::process::AppContainerProfile::derive_for_revocation(
                &format!("Gogoke37.Session.{suffix}")).unwrap();
            let reserve_raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"admission-reserve","requestId":"reserveA","targetId":"sessionA","domainId":"global","expectedRevision":"0","payload":{}}"#;
            let preparation=sha256_hex(b"global\nreserveA");
            let preparation_id=format!("homeprep-{}",&preparation[..40]);
            db.execute("BEGIN IMMEDIATE").unwrap();
            crate::store::session_transport::bind_owner_in_transaction(&mut db,
                &crate::store::session_transport::OwnerBinding {binding_id:&binding_id,
                    instance_id:"oldA",domain_id:"global",kind:"SESSION",
                    owner_id:"sessionA",generation:"1"}).unwrap();
            db.execute("COMMIT").unwrap();
            super::super::temporary::create_temporary_home(&mut db,root,&profile,
                &super::super::temporary::CreateTemporaryHome {request_id:&preparation_id,
                    request_bytes:reserve_raw,home_id:&home_id,instance_id:"oldA",
                    domain_id:"global",kind:super::super::temporary::TemporaryKind::Session,
                    owner_id:"sessionA",generation:"1"}).unwrap();
            let selection=crate::store::session_transport::session_binding::SessionBinding {
                domain_id:"global".into(),session_id:"sessionA".into(),
                seat_id:"seatA".into(),seat_incarnation:incarnation.into(),
                seat_authorization_generation:1,selected_instance_id:"oldA".into(),
                provenance:crate::store::session_transport::session_binding::Provenance::NativeV2,
            };
            let reserve=crate::store::session_transport::AdmissionRequest {domain_id:"global",
                session_id:"sessionA",request_id:"reserveA",raw_bytes:reserve_raw,
                instance_id:"oldA",home_id:&home_id,generation:"1",expected_revision:0};
            assert!(matches!(crate::store::session_transport::reserve_admission(&mut db,&reserve,
                |db|{
                    crate::store::session_transport::session_binding::select_native_in_transaction(
                        db,&selection).map_err(crate::store::session_transport::AdmissionError::Relationship)?;
                    Ok(crate::store::session_transport::TrustedLimits {
                        project_parallel:4,instance_concurrency:4})
                }).unwrap(),
                crate::store::session_transport::AdmissionResult::Applied(1)));
            let commit_raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"admission-commit","requestId":"commitA","targetId":"sessionA","domainId":"global","expectedRevision":"1","payload":{}}"#;
            let commit=crate::store::session_transport::AdmissionRequest {domain_id:"global",
                session_id:"sessionA",request_id:"commitA",raw_bytes:commit_raw,
                instance_id:"oldA",home_id:&home_id,generation:"1",expected_revision:1};
            assert!(matches!(crate::store::session_transport::commit_admission(&mut db,&commit,
                |_|Ok(())).unwrap(),crate::store::session_transport::AdmissionResult::Applied(2)));
            let raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"global","expectedRevision":"2","payload":{}}"#;
            let operation=format!("h-open-{}",&sha256_hex(raw)[..40]);
            let original_hex=hex(raw);
            let original_home_source=qualified_unknown_home_source(&db,&home_id,"oldA",
                "global","sessionA","1","openA",&original_hex).unwrap();
            db.execute("SAVEPOINT missing_original_f_home").unwrap();
            let missing=Statement::prepare(db.as_ptr(),
                "DELETE FROM main.gogoke_v37_instance_homes WHERE home_id=?1").unwrap();
            missing.bind_text(1,&home_id).unwrap();missing.step_done().unwrap();drop(missing);
            assert!(qualified_unknown_home_source(&db,&home_id,"oldA","global",
                "sessionA","1","openA",&original_hex).is_err(),
                "H raw cannot replace the missing original F HOME row");
            db.execute("ROLLBACK TO missing_original_f_home").unwrap();
            db.execute("RELEASE missing_original_f_home").unwrap();
            db.execute("SAVEPOINT changed_original_f_create").unwrap();
            let changed=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET request_hex='00' WHERE request_id=?1").unwrap();
            changed.bind_text(1,&preparation_id).unwrap();changed.step_done().unwrap();drop(changed);
            assert!(qualified_unknown_home_source(&db,&home_id,"oldA","global",
                "sessionA","1","openA",&original_hex).is_err(),
                "H raw cannot replace a damaged APPLIED F create request");
            db.execute("ROLLBACK TO changed_original_f_create").unwrap();
            db.execute("RELEASE changed_original_f_create").unwrap();
            db.execute("SAVEPOINT changed_original_f_receipt").unwrap();
            let changed=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id='wrong-home' WHERE request_id=?1").unwrap();
            changed.bind_text(1,&preparation_id).unwrap();changed.step_done().unwrap();drop(changed);
            assert!(qualified_unknown_home_source(&db,&home_id,"oldA","global",
                "sessionA","1","openA",&original_hex).is_err(),
                "H raw cannot replace a changed original F native HOME receipt");
            db.execute("ROLLBACK TO changed_original_f_receipt").unwrap();
            db.execute("RELEASE changed_original_f_receipt").unwrap();
            db.execute("SAVEPOINT missing_original_f_receipt").unwrap();
            let missing=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id=NULL WHERE request_id=?1").unwrap();
            missing.bind_text(1,&preparation_id).unwrap();missing.step_done().unwrap();drop(missing);
            assert!(qualified_unknown_home_source(&db,&home_id,"oldA","global",
                "sessionA","1","openA",&original_hex).is_err(),
                "H raw cannot replace a missing original F native HOME receipt");
            db.execute("ROLLBACK TO missing_original_f_receipt").unwrap();
            db.execute("RELEASE missing_original_f_receipt").unwrap();
            db.execute("SAVEPOINT changed_original_f_identity").unwrap();
            let changed=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_homes SET directory_identity='wrong-home' WHERE home_id=?1").unwrap();
            changed.bind_text(1,&home_id).unwrap();changed.step_done().unwrap();drop(changed);
            assert!(qualified_unknown_home_source(&db,&home_id,"oldA","global",
                "sessionA","1","openA",&original_hex).is_err(),
                "F HOME identity must still match its original native create receipt");
            db.execute("ROLLBACK TO changed_original_f_identity").unwrap();
            db.execute("RELEASE changed_original_f_identity").unwrap();
            assert_eq!(qualified_unknown_home_source(&db,&home_id,"oldA","global",
                "sessionA","1","openA",&original_hex).unwrap(),original_home_source,
                "restored original F provenance must reproduce its exact digest");
            let h_operation=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('global','openA',?1,'open','sessionA','UNKNOWN',2,2)").unwrap();
            h_operation.bind_text(1,&original_hex).unwrap();h_operation.step_done().unwrap();drop(h_operation);
            // CMD does not accept the extended-length spelling as its cwd.
            // Keep the same bound local directory in its DOS spelling.
            let fixture_cwd=root.canonical_root().canonical_path.to_str().unwrap()
                .strip_prefix(r"\\?\").expect("local extended-length fixture root").to_owned();
            let fixture_directory=std::path::PathBuf::from(&fixture_cwd);
            let cwd_json=crate::store::atomic::json_string(&fixture_cwd);
            let open_response=fixture_directory.join("open-response.jsonl");
            std::fs::write(&open_response,
                format!("{{\"id\":17,\"result\":{{\"thread\":{{\"id\":\"thread-a\",\"cwd\":{cwd_json}}}}}}}\n")).unwrap();
            let mut launch=crate::process::ProcessLaunch::new(command.clone());
            launch.arguments=vec!["/D".into(),"/C".into(),"type open-response.jsonl".into()];
            launch.current_directory=Some(fixture_directory.clone());
            launch.protocol_stdio=true;launch.persistent_protocol_stdio=true;
            let mut custodian=crate::process::ProcessCustodian::new().unwrap();
            let prepared=custodian.prepare(&crate::process::PrepareRequest {launch,
                binding:crate::process::NativeBinding {
                    binary_digest_sha256:command_digest.clone(),
                    profile_id:"oldA".into(),domain_id:"global".into(),generation:"1".into(),
                }}).unwrap();
            crate::store::authority::record_prepared_process(&mut db,&operation,&prepared).unwrap();
            db.execute("BEGIN IMMEDIATE").unwrap();
            crate::store::session_transport::bind_process_operation_in_transaction(
                &mut db,"global","sessionA",&operation).unwrap();
            crate::store::session_transport::record_initial(&db,"global","sessionA","openA",
                &operation).unwrap();
            crate::store::ledger::register_session(&mut db,
                &crate::store::ledger::SessionRegistration {domain_id:"global".into(),
                    seat_id:"seatA".into(),session_id:"sessionA".into(),
                    purpose:crate::store::ledger::SessionPurpose::Secretary,side_id:None}).unwrap();
            // The production LaunchEvidence guard owns this exact point. This
            // fixture cannot construct that private witness, so it calls the
            // same production writer only after its COMMITTED claim, PREPARED
            // C/H episode, pending selection and A registration all exist.
            crate::store::session_transport::session_binding::insert_native_in_transaction(
                &db,&selection).unwrap();
            db.execute("COMMIT").unwrap();
            let open_rpc_id=crate::store::session_transport::codex_rpc::RpcId::client(17).unwrap();
            let open_rpc_command=crate::store::session_transport::codex_rpc::Command::ThreadStartSecretaryTools {
                cwd:fixture_cwd.clone(),model:"m".into()};
            let open_step_id="thread-start";
            let open_step=crate::store::session_transport::rpc_journal::Step {
                domain_id:"global",session_id:"sessionA",open_request_id:"openA",
                open_request_bytes:raw,step_id:open_step_id,custody:&prepared,
                rpc_id:Some(&open_rpc_id),command:&open_rpc_command};
            crate::store::session_transport::rpc_journal::prepare(&mut db,&owner,&open_step).unwrap();
            custodian.activate(&prepared).unwrap();
            crate::store::authority::mark_process_active(&mut db,&operation,&prepared).unwrap();
            crate::store::session_transport::rpc_journal::mark_written(&mut db,&owner,&open_step).unwrap();
            let open_frame=custodian.read_persistent_child_frame(&prepared.ticket,
                std::time::Duration::from_secs(5)).unwrap();
            let open_source=crate::store::ledger::capture_raw_source(&mut db,&open_frame,
                &operation,"open-epoch","1").unwrap();
            crate::store::session_transport::rpc_journal::complete_response(&mut db,&owner,
                &open_step,&open_frame,&open_source.key).unwrap();
            db.execute("BEGIN IMMEDIATE").unwrap();
            db.execute("UPDATE main.gogoke_v37_h_operation SET status='APPLIED',revision=3 WHERE domain_id='global' AND request_id='openA' AND status='UNKNOWN' AND previous_revision=2 AND revision=2").unwrap();
            crate::store::session_transport::mark_active(&db,&operation).unwrap();
            db.execute("COMMIT").unwrap();
            let native_stop=custodian.stop(&prepared.ticket,crate::process::StopBudgets::production(),
                ||Ok(())).unwrap();
            let native_stop_revision=crate::store::authority::mark_process_stopped(
                &mut db,&operation,&native_stop).unwrap();
            db.execute("BEGIN IMMEDIATE").unwrap();
            let proof=crate::store::session_transport::record_session_stop_in_transaction(
                &mut db,"global","sessionA",&operation).unwrap();
            db.execute("COMMIT").unwrap();
            assert_eq!(proof,native_stop.proof_hash());
            // Match the session-stop producer: release native custody only
            // after H committed the exact C StopProof.
            confirm_durable_stop(&mut custodian,&prepared,&native_stop,native_stop_revision);
            db.execute("SAVEPOINT unknown_without_f_home").unwrap();
            let changed=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_coordination_process_custody SET state='UNKNOWN',stop_proof_hash=NULL WHERE operation_id=?1").unwrap();
            changed.bind_text(1,&operation).unwrap();changed.step_done().unwrap();drop(changed);
            let changed=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_h_process_episode SET phase='UNKNOWN',stop_fact_id=NULL WHERE process_operation_id=?1").unwrap();
            changed.bind_text(1,&operation).unwrap();changed.step_done().unwrap();drop(changed);
            let missing=Statement::prepare(db.as_ptr(),
                "DELETE FROM main.gogoke_v37_instance_homes WHERE home_id=?1").unwrap();
            missing.bind_text(1,&home_id).unwrap();missing.step_done().unwrap();drop(missing);
            let error=qualified_executions(&db,&mut Vec::new()).unwrap_err();
            assert!(format!("{error:?}").contains("original UNKNOWN F HOME create source"),
                "UNKNOWN C/H must refuse on absent original F source before H-only observation: {error:?}");
            db.execute("ROLLBACK TO unknown_without_f_home").unwrap();
            db.execute("RELEASE unknown_without_f_home").unwrap();
            // Build A through the existing historical admission/F producers.
            // B above is a real STOPPED execution with the same instance,
            // domain and numeric generation, but a different session.
            db.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,kind,instance_id,state,generation,revision) VALUES('global','siblingSeat','siblingInc','USER','LONG','oldA','BUSY',1,1)").unwrap();
            let sibling_session="releasedSibling";
            let sibling_suffix=sha256_hex(format!("{}\nglobal\n{sibling_session}\nsiblingInc\n1",
                root.canonical_root().identity.opaque()).as_bytes());
            let sibling_suffix=&sibling_suffix[..40];
            let sibling_home=format!("home-{sibling_suffix}");
            let sibling_binding=format!("binding-{sibling_suffix}");
            let sibling_profile=crate::process::AppContainerProfile::derive_for_revocation(
                &format!("Gogoke37.Session.{sibling_suffix}")).unwrap();
            let sibling_reserve=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"admission-reserve","requestId":"siblingReserve","targetId":"releasedSibling","domainId":"global","expectedRevision":"0","payload":{"seatId":"siblingSeat","generation":"1"}}"#;
            let sibling_preparation=sha256_hex(b"global\nsiblingReserve");
            let sibling_preparation=format!("homeprep-{}",&sibling_preparation[..40]);
            db.execute("BEGIN IMMEDIATE").unwrap();
            crate::store::session_transport::bind_owner_in_transaction(&mut db,
                &crate::store::session_transport::OwnerBinding {binding_id:&sibling_binding,
                    instance_id:"oldA",domain_id:"global",kind:"SESSION",
                    owner_id:sibling_session,generation:"1"}).unwrap();
            db.execute("COMMIT").unwrap();
            super::super::temporary::create_temporary_home(&mut db,root,&sibling_profile,
                &super::super::temporary::CreateTemporaryHome {request_id:&sibling_preparation,
                    request_bytes:sibling_reserve,home_id:&sibling_home,instance_id:"oldA",
                    domain_id:"global",kind:super::super::temporary::TemporaryKind::Session,
                    owner_id:sibling_session,generation:"1"}).unwrap();
            db.execute("INSERT INTO main.gogoke_v37_h_seat_binding(domain_id,session_id,seat_id,seat_incarnation,generation) VALUES('global','releasedSibling','siblingSeat','siblingInc','1')").unwrap();
            let sibling_reserve_input=crate::store::session_transport::AdmissionRequest {
                domain_id:"global",session_id:sibling_session,request_id:"siblingReserve",
                raw_bytes:sibling_reserve,instance_id:"oldA",home_id:&sibling_home,
                generation:"1",expected_revision:0};
            assert!(matches!(crate::store::session_transport::reserve_admission(&mut db,
                &sibling_reserve_input,|_|Ok(crate::store::session_transport::TrustedLimits {
                    project_parallel:4,instance_concurrency:4})).unwrap(),
                crate::store::session_transport::AdmissionResult::Applied(1)));
            let sibling_release=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"admission-release","requestId":"siblingRelease","targetId":"releasedSibling","domainId":"global","expectedRevision":"1","payload":{"seatId":"siblingSeat","generation":"1"}}"#;
            let sibling_release_input=crate::store::session_transport::AdmissionRequest {
                domain_id:"global",session_id:sibling_session,request_id:"siblingRelease",
                raw_bytes:sibling_release,instance_id:"oldA",home_id:&sibling_home,
                generation:"1",expected_revision:1};
            assert!(matches!(crate::store::session_transport::release_admission(&mut db,
                &sibling_release_input,|_|Ok(())).unwrap(),
                crate::store::session_transport::AdmissionResult::Applied(2)));
            let check=|db:&VerifiedDatabaseConnection<'_>| qualified_executions(db,&mut Vec::new());
            let (_,operations)=check(&db).unwrap();
            assert!(operations.contains(&operation));
            qualified_resources(&db,&operations).unwrap();
            assert!(qualified_custody_disjoint_from_session(&db,"oldA","global",
                "sessionA","1",&operations).is_err(),
                "the target session's own execution cannot be classified as a sibling");
            assert!(qualified_custody_disjoint_from_session(&db,"oldA","global",
                "neverStartedSibling","1",&BTreeSet::new()).is_err(),
                "an unqualified or orphan custody row cannot disappear through association");
            db.execute("INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('global','sessionA','2','orphanGeneration','orphanProcess')").unwrap();
            let (_,operations)=check(&db).unwrap();
            assert!(qualified_resources(&db,&operations).is_err(),
                "an orphan H generation cannot be hidden by the episode join");
            db.execute("DELETE FROM main.gogoke_v37_h_generation WHERE request_id='orphanGeneration'").unwrap();
            let (_,operations)=check(&db).unwrap();
            qualified_resources(&db,&operations).unwrap();

            // Drive the real UNKNOWN-resume producers. This narrow test seeds
            // the initial global seat/Secretary and OPEN journal row because
            // the ProductDatabase launcher is outside this module; F create,
            // admission, RPC intent/capture/observation, UNKNOWN promotion and
            // both native StopProofs use their production writers.
            let resume_raw=br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"resume","requestId":"resumeB","targetId":"sessionA","domainId":"global","expectedRevision":"3","payload":{"generation":"1"}}"#;
            let resume_suffix=sha256_hex(format!("{}\nglobal\nsessionA\n{incarnation}\n2",
                root.canonical_root().identity.opaque()).as_bytes());
            let resume_suffix=&resume_suffix[..40];
            let resume_home=format!("home-{resume_suffix}");
            let resume_binding=format!("binding-{resume_suffix}");
            let resume_profile=crate::process::AppContainerProfile::derive_for_revocation(
                &format!("Gogoke37.Session.{resume_suffix}")).unwrap();
            let resume_preparation=sha256_hex(b"global\nresumeB");
            let resume_home_request=format!("homeprep-{}",&resume_preparation[..40]);
            db.execute("BEGIN IMMEDIATE").unwrap();
            crate::store::session_transport::bind_owner_in_transaction(&mut db,
                &crate::store::session_transport::OwnerBinding {binding_id:&resume_binding,
                    instance_id:"oldA",domain_id:"global",kind:"SESSION",
                    owner_id:"sessionA",generation:"2"}).unwrap();
            db.execute("COMMIT").unwrap();
            super::super::temporary::create_temporary_home(&mut db,root,&resume_profile,
                &super::super::temporary::CreateTemporaryHome {request_id:&resume_home_request,
                    request_bytes:resume_raw,home_id:&resume_home,instance_id:"oldA",
                    domain_id:"global",kind:super::super::temporary::TemporaryKind::Session,
                    owner_id:"sessionA",generation:"2"}).unwrap();
            crate::store::session_transport::begin_resume(&db,"global","sessionA","resumeB",
                resume_raw,"1","2",3,&resume_home,&resume_binding).unwrap();
            let response_file=fixture_directory.join("resume-response.jsonl");
            std::fs::write(&response_file,
                format!("{{\"id\":19,\"result\":{{\"thread\":{{\"id\":\"thread-a\",\"cwd\":{cwd_json}}}}}}}\n")).unwrap();
            let mut resume_launch=crate::process::ProcessLaunch::new(command.clone());
            resume_launch.arguments=vec!["/D".into(),"/C".into(),"type resume-response.jsonl".into()];
            resume_launch.current_directory=Some(fixture_directory.clone());
            resume_launch.protocol_stdio=true;resume_launch.persistent_protocol_stdio=true;
            let mut resume_custodian=crate::process::ProcessCustodian::new().unwrap();
            let resume_prepared=resume_custodian.prepare(&crate::process::PrepareRequest {
                launch:resume_launch,binding:crate::process::NativeBinding {
                    binary_digest_sha256:command_digest.clone(),profile_id:"oldA".into(),
                    domain_id:"global".into(),generation:"2".into(),
                }}).unwrap();
            let resume_operation=format!("h-resume-{}",&sha256_hex(resume_raw)[..40]);
            crate::store::authority::record_prepared_process(&mut db,&resume_operation,
                &resume_prepared).unwrap();
            crate::store::session_transport::attach_resume_process(&db,"global","resumeB",
                &resume_operation).unwrap();
            let rpc_id=crate::store::session_transport::codex_rpc::RpcId::client(19).unwrap();
            let rpc_command=crate::store::session_transport::codex_rpc::Command::ThreadResume {
                thread_id:"thread-a".into(),cwd:fixture_cwd.clone(),model:"m".into()};
            let step_id=format!("{resume_operation}-thread-resume");
            let rpc_step=crate::store::session_transport::rpc_journal::Step {
                domain_id:"global",session_id:"sessionA",open_request_id:"resumeB",
                open_request_bytes:resume_raw,step_id:&step_id,custody:&resume_prepared,
                rpc_id:Some(&rpc_id),command:&rpc_command};
            crate::store::session_transport::rpc_journal::prepare(&mut db,&owner,&rpc_step).unwrap();
            resume_custodian.activate(&resume_prepared).unwrap();
            crate::store::authority::mark_process_active(&mut db,&resume_operation,
                &resume_prepared).unwrap();
            crate::store::session_transport::rpc_journal::mark_written(&mut db,&owner,&rpc_step).unwrap();
            let response=resume_custodian.read_persistent_child_frame(&resume_prepared.ticket,
                std::time::Duration::from_secs(5)).unwrap();
            let source=crate::store::ledger::capture_raw_source(&mut db,&response,
                &resume_operation,"resume-epoch","1").unwrap();
            crate::store::session_transport::rpc_journal::complete_response(&mut db,&owner,
                &rpc_step,&response,&source.key).unwrap();
            crate::store::authority::mark_process_unknown(&mut db,&resume_operation,
                &resume_prepared).unwrap();
            db.execute("BEGIN IMMEDIATE").unwrap();
            crate::store::session_transport::mark_resume_unknown(&db,"global","sessionA",
                "resumeB",&resume_operation).unwrap();
            db.execute("COMMIT").unwrap();
            assert_eq!(crate::store::session_transport::promote_resume(&db,"global","sessionA",
                "resumeB",&resume_operation,4).unwrap(),5);
            let resume_stop=resume_custodian.stop(&resume_prepared.ticket,
                crate::process::StopBudgets::production(),||Ok(())).unwrap();
            let resume_stop_revision=crate::store::authority::mark_process_stopped(
                &mut db,&resume_operation,&resume_stop).unwrap();
            db.execute("BEGIN IMMEDIATE").unwrap();
            let resume_proof=crate::store::session_transport::record_session_stop_in_transaction(
                &mut db,"global","sessionA",&resume_operation).unwrap();
            db.execute("COMMIT").unwrap();
            assert_eq!(resume_proof,resume_stop.proof_hash());
            confirm_durable_stop(&mut resume_custodian,&resume_prepared,&resume_stop,
                resume_stop_revision);
            let (historical,_)=check(&db).unwrap();
            qualified_owner_bindings(&db,&historical).unwrap();
            assert!(qualified_owner_bindings(&db,&BTreeSet::new()).is_err(),
                "a retained ACTIVE binding needs its independently qualified historical episode");
            db.execute("UPDATE main.gogoke_v37_native_selection SET seat_incarnation='other' WHERE domain_id='global' AND session_id='sessionA'").unwrap();
            assert!(check(&db).is_err(),"historical global generation still needs its original Secretary selection");
            db.execute("UPDATE main.gogoke_v37_native_selection SET seat_incarnation='incA' WHERE domain_id='global' AND session_id='sessionA'").unwrap();
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET previous_revision=3 WHERE request_id='resumeB'").unwrap();
            assert!(check(&db).is_err(),"UNKNOWN-promoted resume cannot be reclassified as direct");
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET previous_revision=4 WHERE request_id='resumeB'").unwrap();
            let original_create=Statement::prepare(db.as_ptr(),
                "SELECT request_hex FROM main.gogoke_v37_instance_operations WHERE request_id=?1").unwrap();
            original_create.bind_text(1,&preparation_id).unwrap();
            assert!(original_create.step_row().unwrap());
            let original_create_fingerprint=original_create.column_text(0).unwrap();
            assert!(!original_create.step_row().unwrap());drop(original_create);
            let corrupt_create=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET request_hex='zz' WHERE request_id=?1").unwrap();
            corrupt_create.bind_text(1,&preparation_id).unwrap();corrupt_create.step_done().unwrap();drop(corrupt_create);
            assert!(check(&db).is_err());
            let restore_create=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id=?2").unwrap();
            restore_create.bind_text(1,&original_create_fingerprint).unwrap();
            restore_create.bind_text(2,&preparation_id).unwrap();restore_create.step_done().unwrap();drop(restore_create);
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET raw_hex='zz' WHERE request_id='openA'").unwrap();
            assert!(check(&db).is_err());
            let restore=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_h_process_episode SET raw_hex=?1 WHERE request_id='openA'").unwrap();
            restore.bind_text(1,&original_hex).unwrap();restore.step_done().unwrap();drop(restore);
            db.execute("UPDATE main.gogoke_v37_h_process_episode SET home_id='otherHome' WHERE request_id='openA'").unwrap();
            assert!(check(&db).is_err());
            let restore_home=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_h_process_episode SET home_id=?1,binding_id='otherBinding' WHERE request_id='openA'").unwrap();
            restore_home.bind_text(1,&home_id).unwrap();restore_home.step_done().unwrap();drop(restore_home);
            assert!(check(&db).is_err());
            let restore_binding=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_h_process_episode SET binding_id=?1 WHERE request_id='openA'").unwrap();
            restore_binding.bind_text(1,&binding_id).unwrap();restore_binding.step_done().unwrap();drop(restore_binding);
            let corrupt_proof=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrong' WHERE operation_id=?1").unwrap();
            corrupt_proof.bind_text(1,&operation).unwrap();corrupt_proof.step_done().unwrap();drop(corrupt_proof);
            assert!(check(&db).is_err());
            db.close_checked().unwrap();
        });
    }
}
