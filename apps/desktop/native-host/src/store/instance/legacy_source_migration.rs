//! One authenticated, original-intent migration of a proven legacy Codex SOURCE.
//! This F journal never advances an old instance operation or H/C outcome.

use super::{credential_registry, legacy_fence, managed_cli, private_history, registry};
use crate::process::{CredentialAliasScope, CredentialBinding, NativeLegacyHoldersGone};
use crate::root::RootLock;
use crate::store::atomic::Statement;
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

/// All affected original C rows are read. STOPPED requires its original proof;
/// an unsettled H row needs H's unique original-source kernel fact; a global
/// historical UNKNOWN needs the already applied original legacy fence.
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
        let fence = legacy_fence::read_applied_legacy_retirement(db, &instance)
            .map_err(|error| fail("original global fence", error))?.ok_or_else(denied)?;
        let unknown: Vec<_> = custody.iter().filter(|c| c[1] == instance && c[2] == "global" && c[4] == "UNKNOWN").collect();
        if unknown.is_empty() || unknown.iter().any(|c|
            !fence.custody.iter().any(|original| original.operation_id == c[0] &&
                original.pid == c[6] && original.creation_time_100ns == c[7])) {
            return Err(denied());
        }
        let pairs = fence.custody.iter().map(legacy_fence::LegacyCustodyRow::native_pair)
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|error| fail("global exact pairs", error))?;
        let fact = NativeLegacyHoldersGone::observe(&pairs)
            .map_err(|error| fail("global kernel fact", error))?;
        fact.validate(&pairs).map_err(|error| fail("global kernel recheck", error))?;
        frame(evidence, fence.custody_digest.as_bytes());
        frame(evidence, &fact.boot_start().to_be_bytes());
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

fn qualified_resources(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for (sql, values) in [
        ("SELECT 1 FROM main.gogoke_v37_instance_operations o JOIN main.gogoke_v37_instances i ON i.instance_id=o.target_id WHERE i.driver_id='codex' AND o.phase!='APPLIED' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE open_driver_id='codex' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id='managed-cli-codex' AND (state!='STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_operation WHERE status!='APPLIED' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_generation_change WHERE stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_stdin_journal WHERE phase!='RECEIPTED' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id='codex' AND (a.cleanup_status IN ('NOT_REQUESTED','UNKNOWN') OR a.phase IN ('INTENT','PREPARE_STARTED','UNKNOWN')) LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id='codex' AND h.state IN ('PREPARING','CLOSE_UNKNOWN','CLEANUP_UNKNOWN','UNKNOWN') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_objects o JOIN main.gogoke_v37_instances i ON i.instance_id=o.instance_id WHERE i.driver_id='codex' AND o.phase!='ACTIVE' LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_aliases a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id='codex' AND a.state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1", vec![]),
        ("SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id WHERE i.driver_id='codex' AND p.state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1", vec![]),
    ] {
        if any(db, sql, &values)? { return Err(denied()); }
    }
    let claims = rows(db,
        "SELECT c.instance_id,c.domain_id,c.session_id,c.state,COALESCE(c.process_operation_id,''),COALESCE(c.stop_fact_id,''),c.generation,c.binding_id FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id='codex'", &[], 8)?;
    for c in &claims {
        if c[4].is_empty() {
            if c[3] != "RELEASED" || !c[5].is_empty() || any(db,
                "SELECT 1 FROM main.gogoke_v37_h_process_episode WHERE domain_id=?1 AND session_id=?2 LIMIT 1", &[&c[1], &c[2]])? {
                return Err(denied());
            }
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
            "SELECT e.process_operation_id,c.state FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id AND c.profile_id=e.instance_id AND c.domain_id=e.domain_id AND c.generation=e.generation WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3 AND e.session_id=?4 AND e.generation=?5", &[&h[0], &h[1], &h[2], &h[3], &h[4]], 2)?;
        if execution.len() != 1 || execution[0][1] != "UNKNOWN" { return Err(denied()); }
    }
    Ok(())
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
        "SELECT i.instance_id,i.home_identity,i.program_digest,i.version,COALESCE(CAST(p.enabled AS TEXT),''),COALESCE(CAST(p.tombstoned AS TEXT),''),i.login_state FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id LEFT JOIN main.gogoke_v37_instance_program_sources s ON s.instance_id=i.instance_id WHERE i.driver_id='codex' AND (p.tombstoned IS NULL OR p.tombstoned=0) AND s.instance_id IS NULL ORDER BY i.instance_id", &[], 7)?;
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
    let object = credential_registry::read_credential_object(db, input.target_id)
        .map_err(|error| fail("credential object", error))?;
    if let Some(object) = object {
        let backend = credential_registry::read_usable_credential_backend(db, input.target_id)
            .map_err(|error| fail("credential source", error))?;
        if object.home_identity.opaque() != input.original_home_identity ||
            backend.home_identity.opaque() != input.original_home_identity { return Err(denied()); }
        let source = root.canonical_root().canonical_path.join("v37-instances")
            .join(input.target_id).join("auth.json");
        let aliases = credential_registry::read_credential_aliases(db, input.target_id)
            .map_err(|error| fail("credential aliases", error))?;
        let mut scopes = Vec::new();
        for alias in aliases {
            match alias.state.as_str() {
                "ACTIVE" | "DORMANT" => {
                    if alias.source_file_identity != object.file_identity { return Err(denied()); }
                    let directory = private_history::resolve_private_history_directory(db, root,
                        &alias.history_id).map_err(|error| fail("alias directory", error))?;
                    if directory.identity != alias.directory_identity { return Err(denied()); }
                    scopes.push(CredentialAliasScope { root: directory.path,
                        root_identity: directory.identity });
                },
                "REMOVED" => (),
                _ => return Err(denied()),
            }
        }
        CredentialBinding::open_registered(root, &source, &object.home_identity,
            &object.file_identity, &scopes).map_err(|error| fail("original credential metadata", error))?;
    } else if candidates[0][6] == "LOGGED_IN" {
        return Err(denied());
    }
    qualified_resources(db)?;
    let mut evidence = Vec::new();
    qualified_executions(db, &mut evidence)?;
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
        fixture(|_, db| {
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
        fixture(|root, db| {
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
}
