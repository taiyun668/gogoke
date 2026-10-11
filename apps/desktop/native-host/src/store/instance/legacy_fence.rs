//! Metadata-only, one-instance boot fence for two exact legacy ACL repairs.
//! Native callers own kernel boot observations and sealed ACL readback. This
//! module neither opens credential content nor changes permissions or custody.

use crate::root::RootIdentity;
use crate::store::atomic::{AtomicError, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

const FENCE_SQL: &str = "CREATE TABLE gogoke_v37_legacy_acl_fences(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),boot_identity TEXT NOT NULL,database_identity TEXT NOT NULL,root_identity TEXT NOT NULL,home_identity TEXT NOT NULL,source_identity TEXT NOT NULL,source_parent_identity TEXT NOT NULL,source_revision INTEGER CHECK(source_revision>=1),custody_digest TEXT NOT NULL,home_acl_digest TEXT NOT NULL,source_acl_digest TEXT NOT NULL,acl_provenance_digest TEXT NOT NULL,native_snapshot TEXT NOT NULL,native_snapshot_digest TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>=1)) STRICT";
const CUSTODY_SQL: &str = "CREATE TABLE gogoke_v37_legacy_acl_custody(instance_id TEXT NOT NULL REFERENCES gogoke_v37_legacy_acl_fences(instance_id),operation_id TEXT NOT NULL,ticket TEXT NOT NULL,custodian_nonce TEXT NOT NULL,pid TEXT NOT NULL,creation_time_100ns TEXT NOT NULL,image_path TEXT NOT NULL,binary_digest_sha256 TEXT NOT NULL,profile_id TEXT NOT NULL,domain_id TEXT NOT NULL,generation TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('STOPPED','UNKNOWN')),stop_proof_hash TEXT,PRIMARY KEY(instance_id,operation_id)) STRICT";
const STEP_SQL: &str = "CREATE TABLE gogoke_v37_legacy_acl_steps(instance_id TEXT NOT NULL REFERENCES gogoke_v37_legacy_acl_fences(instance_id),step TEXT NOT NULL CHECK(step IN ('HOME','BASELINE')),phase TEXT NOT NULL CHECK(phase IN ('PENDING','APPLIED')),request_id TEXT NOT NULL,boot_identity TEXT NOT NULL,before_home_acl_digest TEXT NOT NULL,before_source_acl_digest TEXT NOT NULL,target_home_acl_digest TEXT NOT NULL,target_source_acl_digest TEXT NOT NULL,intent_revision INTEGER NOT NULL CHECK(intent_revision>=2),applied_revision INTEGER,PRIMARY KEY(instance_id,step),CHECK((phase='PENDING' AND applied_revision IS NULL) OR (phase='APPLIED' AND applied_revision>intent_revision))) STRICT";
const SCHEMA: [(&str, &str); 3] = [
    ("gogoke_v37_legacy_acl_fences", FENCE_SQL),
    ("gogoke_v37_legacy_acl_custody", CUSTODY_SQL),
    ("gogoke_v37_legacy_acl_steps", STEP_SQL),
];

#[derive(Debug)]
pub(crate) enum LegacyFenceError {
    Invalid(&'static str), Conflict, Unsafe, Schema,
    IdentityParse(String), NativeHolders(crate::process::NativeLegacyHoldersGoneError),
    Atomic(AtomicError), Sqlite(SameOpenError), CommitUnknown(SameOpenError),
    RollbackUnknown { original: Box<LegacyFenceError>, rollback: SameOpenError },
}
impl From<AtomicError> for LegacyFenceError { fn from(value: AtomicError) -> Self { Self::Atomic(value) } }
impl From<SameOpenError> for LegacyFenceError { fn from(value: SameOpenError) -> Self { Self::Sqlite(value) } }
type Result<T> = std::result::Result<T, LegacyFenceError>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyCustodyRow {
    pub(crate) operation_id: String, pub(crate) ticket: String, pub(crate) custodian_nonce: String,
    pub(crate) pid: String, pub(crate) creation_time_100ns: String, pub(crate) image_path: String,
    pub(crate) binary_digest_sha256: String, pub(crate) profile_id: String,
    pub(crate) domain_id: String, pub(crate) generation: String,
    pub(crate) state: String, pub(crate) stop_proof_hash: Option<String>,
}
impl LegacyCustodyRow {
    pub(crate) fn native_pair(&self) -> Result<(u32, u64)> {
        let pid = self.pid.parse::<u32>().map_err(|error|
            LegacyFenceError::IdentityParse(format!("legacy custody pid: {error}")))?;
        let created = self.creation_time_100ns.parse::<u64>().map_err(|error|
            LegacyFenceError::IdentityParse(format!("legacy custody creation time: {error}")))?;
        if pid == 0 || created == 0 { return Err(LegacyFenceError::Invalid("zero original process identity")); }
        Ok((pid, created))
    }
    fn fields(&self) -> [&str; 12] {
        [&self.operation_id, &self.ticket, &self.custodian_nonce, &self.pid,
            &self.creation_time_100ns, &self.image_path, &self.binary_digest_sha256,
            &self.profile_id, &self.domain_id, &self.generation, &self.state,
            self.stop_proof_hash.as_deref().unwrap_or("")]
    }
}

/// The caller records kernel BootTime metadata and ACL digests from
/// metadata-only, held physical objects. No wire constructor is provided.
#[derive(Clone, Debug)]
pub(crate) struct LegacyFenceCapture {
    pub(crate) instance_id: String, pub(crate) original_boot: String,
    pub(crate) database_identity: RootIdentity, pub(crate) root_identity: RootIdentity,
    pub(crate) home_identity: RootIdentity, pub(crate) source_identity: RootIdentity,
    pub(crate) source_parent_identity: RootIdentity, pub(crate) source_revision: Option<i64>,
    pub(crate) source_link_count: u64, pub(crate) registered_alias_count: usize,
    pub(crate) custody: Vec<LegacyCustodyRow>,
    pub(crate) home_acl_digest: String, pub(crate) source_acl_digest: String,
    pub(crate) acl_provenance_digest: String,
    pub(crate) native_snapshot: String, pub(crate) native_snapshot_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyFenceRecord {
    pub(crate) instance_id: String, pub(crate) original_boot: String,
    pub(crate) database_identity: String, pub(crate) root_identity: String,
    pub(crate) home_identity: String, pub(crate) source_identity: String,
    pub(crate) source_parent_identity: String, pub(crate) source_revision: Option<i64>,
    pub(crate) custody_digest: String, pub(crate) custody: Vec<LegacyCustodyRow>,
    pub(crate) home_acl_digest: String, pub(crate) source_acl_digest: String,
    pub(crate) acl_provenance_digest: String, pub(crate) native_snapshot: String,
    pub(crate) native_snapshot_digest: String, pub(crate) revision: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyAclStep { Home, Baseline }
impl LegacyAclStep { fn text(self) -> &'static str { match self { Self::Home => "HOME", Self::Baseline => "BASELINE" } } }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LegacyStepPhase { Pending, Applied }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LegacyStepIntent {
    pub(crate) instance_id: String, pub(crate) step: LegacyAclStep,
    pub(crate) request_id: String, pub(crate) current_boot: String,
    pub(crate) before_home_acl_digest: String, pub(crate) before_source_acl_digest: String,
    pub(crate) target_home_acl_digest: String, pub(crate) target_source_acl_digest: String,
    pub(crate) intent_revision: i64, pub(crate) applied_revision: Option<i64>,
    pub(crate) phase: LegacyStepPhase,
}

/// Current physical metadata and actual ACL readback. The caller must acquire
/// the original root exclusion and seal this proof from native handles.
pub(crate) struct LegacyPhysicalProof {
    pub(crate) holders_gone: crate::process::NativeLegacyHoldersGone,
    pub(crate) database_identity: RootIdentity, pub(crate) root_identity: RootIdentity,
    pub(crate) home_identity: RootIdentity, pub(crate) source_identity: RootIdentity,
    pub(crate) source_parent_identity: RootIdentity, pub(crate) source_revision: Option<i64>,
    pub(crate) source_link_count: u64, pub(crate) registered_alias_count: usize,
    pub(crate) acl_provenance_digest: String,
    pub(crate) actual_home_acl_digest: String, pub(crate) actual_source_acl_digest: String,
}
pub(crate) struct LegacyStepRequest<'a> {
    pub(crate) instance_id: &'a str, pub(crate) step: LegacyAclStep,
    pub(crate) request_id: &'a str,
    pub(crate) expected_revision: i64,
    pub(crate) target_home_acl_digest: &'a str, pub(crate) target_source_acl_digest: &'a str,
    pub(crate) proof: &'a LegacyPhysicalProof,
}
pub(crate) struct LegacyStepFinish<'a> {
    pub(crate) instance_id: &'a str, pub(crate) step: LegacyAclStep,
    pub(crate) request_id: &'a str,
    pub(crate) expected_intent_revision: i64, pub(crate) proof: &'a LegacyPhysicalProof,
}

fn atom(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 256 || !value.bytes().all(|b|
        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':')) {
        return Err(LegacyFenceError::Invalid("metadata identifier"));
    }
    Ok(())
}
fn digest(value: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(LegacyFenceError::Invalid("metadata digest"));
    }
    Ok(())
}
fn snapshot_digest(payload: &str) -> Result<String> {
    if payload.is_empty() || payload.len() % 2 != 0 ||
        !payload.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(LegacyFenceError::Invalid("native snapshot encoding"));
    }
    let mut decoded = Vec::with_capacity(payload.len() / 2);
    for pair in payload.as_bytes().chunks_exact(2) {
        let part = std::str::from_utf8(pair).map_err(|_| LegacyFenceError::Invalid("native snapshot utf8"))?;
        decoded.push(u8::from_str_radix(part, 16).map_err(|_| LegacyFenceError::Invalid("native snapshot hex"))?);
    }
    Ok(sha256_hex(&decoded))
}
fn bind(statement: &Statement, values: &[&str]) -> Result<()> {
    for (i, value) in values.iter().enumerate() { statement.bind_text((i + 1) as i32, value)?; }
    Ok(())
}
fn one_change(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    let q = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !q.step_row()? || q.column_text(0)? != "1" { return Err(LegacyFenceError::Conflict); }
    Ok(())
}
fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>, work: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T>) -> Result<T> {
    db.execute("BEGIN IMMEDIATE")?;
    match work(db) {
        Ok(value) => { db.execute("COMMIT").map_err(LegacyFenceError::CommitUnknown)?; Ok(value) },
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(LegacyFenceError::RollbackUnknown { original: Box::new(original), rollback }),
        },
    }
}
fn observed_schema(db: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>> {
    let q = Statement::prepare(db.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,22))='gogoke_v37_legacy_acl_' ORDER BY name")?;
    let mut rows = Vec::new();
    while q.step_row()? { rows.push((q.column_text(0)?, q.column_text(1)?)); }
    Ok(rows)
}
fn expected_schema() -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = SCHEMA.iter().map(|(n, s)| ((*n).to_owned(), (*s).to_owned())).collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0)); rows
}
fn reject_schema_side_effects(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for schema in ["temp", "main"] {
        let sql = if schema == "temp" {
            "SELECT 1 FROM temp.sqlite_schema WHERE lower(substr(name,1,22))='gogoke_v37_legacy_acl_' OR lower(substr(tbl_name,1,22))='gogoke_v37_legacy_acl_' LIMIT 1"
        } else {
            "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(substr(tbl_name,1,22))='gogoke_v37_legacy_acl_' LIMIT 1"
        };
        if Statement::prepare(db.as_ptr(), sql)?.step_row()? { return Err(LegacyFenceError::Schema); }
    }
    Ok(())
}
pub(crate) fn initialize_legacy_fence_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    reject_schema_side_effects(db)?;
    let before = observed_schema(db)?;
    if before == expected_schema() { return Ok(()); }
    if !before.is_empty() { return Err(LegacyFenceError::Schema); }
    transaction(db, |db| {
        reject_schema_side_effects(db)?;
        if observed_schema(db)? != before { return Err(LegacyFenceError::Schema); }
        for (_, sql) in SCHEMA { db.execute(sql)?; }
        if observed_schema(db)? != expected_schema() { return Err(LegacyFenceError::Schema); }
        Ok(())
    })
}

fn live_custody(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<Vec<LegacyCustodyRow>> {
    let q = Statement::prepare(db.as_ptr(), "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,''),stop_proof_hash IS NULL FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 ORDER BY operation_id")?;
    q.bind_text(1, instance)?;
    let mut rows = Vec::new();
    while q.step_row()? {
        rows.push(LegacyCustodyRow {
            operation_id: q.column_text(0)?, ticket: q.column_text(1)?, custodian_nonce: q.column_text(2)?,
            pid: q.column_text(3)?, creation_time_100ns: q.column_text(4)?, image_path: q.column_text(5)?,
            binary_digest_sha256: q.column_text(6)?, profile_id: q.column_text(7)?,
            domain_id: q.column_text(8)?, generation: q.column_text(9)?, state: q.column_text(10)?,
            stop_proof_hash: if q.column_text(12)? == "1" { None } else { Some(q.column_text(11)?) },
        });
    }
    Ok(rows)
}
fn custody_digest(rows: &[LegacyCustodyRow]) -> String {
    let mut bytes = Vec::new();
    for row in rows {
        for field in row.fields() {
            bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes.push(u8::from(row.stop_proof_hash.is_some()));
    }
    sha256_hex(&bytes)
}
fn captured_custody(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<Vec<LegacyCustodyRow>> {
    let q = Statement::prepare(db.as_ptr(), "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,''),stop_proof_hash IS NULL FROM main.gogoke_v37_legacy_acl_custody WHERE instance_id=?1 ORDER BY operation_id")?;
    q.bind_text(1, instance)?;
    let mut rows = Vec::new();
    while q.step_row()? {
        rows.push(LegacyCustodyRow {
            operation_id: q.column_text(0)?, ticket: q.column_text(1)?, custodian_nonce: q.column_text(2)?,
            pid: q.column_text(3)?, creation_time_100ns: q.column_text(4)?, image_path: q.column_text(5)?,
            binary_digest_sha256: q.column_text(6)?, profile_id: q.column_text(7)?, domain_id: q.column_text(8)?,
            generation: q.column_text(9)?, state: q.column_text(10)?,
            stop_proof_hash: if q.column_text(12)? == "1" { None } else { Some(q.column_text(11)?) },
        });
    }
    Ok(rows)
}
fn check_custody(db: &VerifiedDatabaseConnection<'_>, fence: &LegacyFenceRecord) -> Result<()> {
    if custody_digest(&fence.custody) != fence.custody_digest { return Err(LegacyFenceError::Conflict); }
    let live = live_custody(db, &fence.instance_id)?;
    for old in &fence.custody {
        if !live.contains(old) { return Err(LegacyFenceError::Conflict); }
    }
    for row in live.iter().filter(|row| !fence.custody.iter().any(|old| old.operation_id == row.operation_id)) {
        // New completed episodes do not enlarge the original fence. An
        // unlisted UNKNOWN or live holder cannot be migrated through it.
        if row.state != "STOPPED" || row.stop_proof_hash.as_deref().map_or(true, str::is_empty) {
            return Err(LegacyFenceError::Unsafe);
        }
    }
    Ok(())
}
fn check_binding(db: &VerifiedDatabaseConnection<'_>, instance: &str, database: &str, root: &str,
    home: &str, source: &str, parent: &str, revision: Option<i64>) -> Result<()> {
    if db.identity().opaque() != database || db.root_identity().opaque() != root || parent != home ||
        source == home || revision.is_some_and(|value| value < 1) { return Err(LegacyFenceError::Conflict); }
    // The source may predate F's credential registration. Its original
    // physical identity remains fenced independently and cannot be inferred
    // from a newly fabricated credential registry row.
    let home_volume = home.strip_prefix("volume:").and_then(|value| value.split_once("/file:")).map(|value| value.0);
    let source_volume = source.strip_prefix("volume:").and_then(|value| value.split_once("/file:")).map(|value| value.0);
    if home_volume.is_none() || home_volume != source_volume { return Err(LegacyFenceError::Conflict); }
    let q = Statement::prepare(db.as_ptr(), "SELECT home_identity FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id='codex'")?;
    q.bind_text(1, instance)?;
    if !q.step_row()? || q.column_text(0)? != home || q.step_row()? { return Err(LegacyFenceError::Conflict); }
    let q = Statement::prepare(db.as_ptr(), "SELECT root_identity,home_identity,file_identity,source_parent_identity,CAST(revision AS TEXT),phase FROM main.gogoke_v37_credential_objects WHERE instance_id=?1")?;
    q.bind_text(1, instance)?;
    if q.step_row()? {
        let expected = revision.ok_or(LegacyFenceError::Conflict)?;
        if q.column_text(0)? != root || q.column_text(1)? != home || q.column_text(2)? != source ||
            q.column_text(3)? != parent || q.column_text(4)? != expected.to_string() || q.column_text(5)? != "ACTIVE" ||
            q.step_row()? { return Err(LegacyFenceError::Conflict); }
    } else if revision.is_some() { return Err(LegacyFenceError::Conflict); }
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 AND state<>'REMOVED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 AND state<>'REVOKED' LIMIT 1",
    ] {
        let q = Statement::prepare(db.as_ptr(), sql)?; q.bind_text(1, instance)?;
        if q.step_row()? { return Err(LegacyFenceError::Unsafe); }
    }
    let target = format!("credential-instance-{}", sha256_hex(instance.as_bytes()));
    let q = Statement::prepare(db.as_ptr(), "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN') LIMIT 1")?;
    q.bind_text(1, &target)?;
    if q.step_row()? { return Err(LegacyFenceError::Unsafe); }
    Ok(())
}

pub(crate) fn read_legacy_fence(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<Option<LegacyFenceRecord>> {
    atom(instance)?;
    let q = Statement::prepare(db.as_ptr(), "SELECT boot_identity,database_identity,root_identity,home_identity,source_identity,source_parent_identity,COALESCE(CAST(source_revision AS TEXT),''),source_revision IS NULL,custody_digest,home_acl_digest,source_acl_digest,acl_provenance_digest,native_snapshot,native_snapshot_digest,CAST(revision AS TEXT) FROM main.gogoke_v37_legacy_acl_fences WHERE instance_id=?1")?;
    q.bind_text(1, instance)?;
    if !q.step_row()? { return Ok(None); }
    let row = LegacyFenceRecord {
        instance_id: instance.into(), original_boot: q.column_text(0)?, database_identity: q.column_text(1)?,
        root_identity: q.column_text(2)?, home_identity: q.column_text(3)?, source_identity: q.column_text(4)?,
        source_parent_identity: q.column_text(5)?, source_revision: if q.column_text(7)? == "1" { None }
            else { Some(q.column_text(6)?.parse().map_err(|_| LegacyFenceError::Invalid("revision"))?) },
        custody_digest: q.column_text(8)?, home_acl_digest: q.column_text(9)?, source_acl_digest: q.column_text(10)?,
        acl_provenance_digest: q.column_text(11)?, native_snapshot: q.column_text(12)?,
        native_snapshot_digest: q.column_text(13)?,
        revision: q.column_text(14)?.parse().map_err(|_| LegacyFenceError::Invalid("revision"))?,
        custody: captured_custody(db, instance)?,
    };
    if q.step_row()? { return Err(LegacyFenceError::Conflict); }
    if custody_digest(&row.custody) != row.custody_digest { return Err(LegacyFenceError::Conflict); }
    if snapshot_digest(&row.native_snapshot)? != row.native_snapshot_digest { return Err(LegacyFenceError::Conflict); }
    Ok(Some(row))
}

pub(crate) fn capture_legacy_fence(db: &mut VerifiedDatabaseConnection<'_>, input: &LegacyFenceCapture) -> Result<LegacyFenceRecord> {
    atom(&input.instance_id)?; atom(&input.original_boot)?;
    if input.original_boot.eq_ignore_ascii_case("UNKNOWN") { return Err(LegacyFenceError::Unsafe); }
    for value in [&input.home_acl_digest, &input.source_acl_digest, &input.acl_provenance_digest] { digest(value)?; }
    digest(&input.native_snapshot_digest)?;
    if snapshot_digest(&input.native_snapshot)? != input.native_snapshot_digest { return Err(LegacyFenceError::Conflict); }
    if input.source_link_count != 1 || input.registered_alias_count != 0 || input.custody.is_empty() { return Err(LegacyFenceError::Unsafe); }
    transaction(db, |db| {
        if read_legacy_fence(db, &input.instance_id)?.is_some() { return Err(LegacyFenceError::Conflict); }
        check_binding(db, &input.instance_id, &input.database_identity.opaque(), &input.root_identity.opaque(),
            &input.home_identity.opaque(), &input.source_identity.opaque(), &input.source_parent_identity.opaque(), input.source_revision)?;
        let live = live_custody(db, &input.instance_id)?;
        if live != input.custody || live.iter().any(|row| row.state != "UNKNOWN" &&
            (row.state != "STOPPED" || row.stop_proof_hash.as_deref().map_or(true, str::is_empty))) {
            return Err(LegacyFenceError::Unsafe);
        }
        let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_legacy_acl_fences VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,1)")?;
        let captured_hash = custody_digest(&live);
        bind(&q, &[&input.instance_id, &input.original_boot, &input.database_identity.opaque(), &input.root_identity.opaque(),
            &input.home_identity.opaque(), &input.source_identity.opaque(), &input.source_parent_identity.opaque()])?;
        if let Some(revision) = input.source_revision { q.bind_i64(8, revision)?; }
        for (index, value) in [&captured_hash, &input.home_acl_digest, &input.source_acl_digest,
            &input.acl_provenance_digest, &input.native_snapshot, &input.native_snapshot_digest].iter().enumerate() {
            q.bind_text((index + 9) as i32, value)?;
        }
        q.step_done()?;
        for row in &live {
            let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_legacy_acl_custody VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")?;
            bind(&q, &[&input.instance_id, &row.operation_id, &row.ticket, &row.custodian_nonce, &row.pid,
                &row.creation_time_100ns, &row.image_path, &row.binary_digest_sha256, &row.profile_id,
                &row.domain_id, &row.generation, &row.state])?;
            if let Some(hash) = &row.stop_proof_hash { q.bind_text(13, hash)?; }
            q.step_done()?;
        }
        read_legacy_fence(db, &input.instance_id)?.ok_or(LegacyFenceError::Conflict)
    })
}

fn check_proof(db: &VerifiedDatabaseConnection<'_>, fence: &LegacyFenceRecord, proof: &LegacyPhysicalProof) -> Result<()> {
    if proof.source_link_count != 1 || proof.registered_alias_count != 0 ||
        proof.database_identity.opaque() != fence.database_identity || proof.root_identity.opaque() != fence.root_identity ||
        proof.home_identity.opaque() != fence.home_identity || proof.source_identity.opaque() != fence.source_identity ||
        proof.source_parent_identity.opaque() != fence.source_parent_identity || proof.source_revision != fence.source_revision ||
        proof.acl_provenance_digest != fence.acl_provenance_digest { return Err(LegacyFenceError::Conflict); }
    check_binding(db, &fence.instance_id, &fence.database_identity, &fence.root_identity,
        &fence.home_identity, &fence.source_identity, &fence.source_parent_identity, fence.source_revision)?;
    check_custody(db, fence)
}
pub(crate) fn read_legacy_step(db: &VerifiedDatabaseConnection<'_>, instance: &str, step: LegacyAclStep) -> Result<Option<LegacyStepIntent>> {
    let q = Statement::prepare(db.as_ptr(), "SELECT phase,request_id,boot_identity,before_home_acl_digest,before_source_acl_digest,target_home_acl_digest,target_source_acl_digest,CAST(intent_revision AS TEXT),COALESCE(CAST(applied_revision AS TEXT),''),applied_revision IS NULL FROM main.gogoke_v37_legacy_acl_steps WHERE instance_id=?1 AND step=?2")?;
    bind(&q, &[instance, step.text()])?;
    if !q.step_row()? { return Ok(None); }
    let phase = match q.column_text(0)?.as_str() { "PENDING" => LegacyStepPhase::Pending, "APPLIED" => LegacyStepPhase::Applied, _ => return Err(LegacyFenceError::Schema) };
    let row = LegacyStepIntent { instance_id: instance.into(), step, phase,
        request_id: q.column_text(1)?, current_boot: q.column_text(2)?,
        before_home_acl_digest: q.column_text(3)?, before_source_acl_digest: q.column_text(4)?,
        target_home_acl_digest: q.column_text(5)?, target_source_acl_digest: q.column_text(6)?,
        intent_revision: q.column_text(7)?.parse().map_err(|_| LegacyFenceError::Schema)?,
        applied_revision: if q.column_text(9)? == "1" { None } else { Some(q.column_text(8)?.parse().map_err(|_| LegacyFenceError::Schema)?) },
    };
    if q.step_row()? { return Err(LegacyFenceError::Conflict); }
    Ok(Some(row))
}

/// Read-only reuse of the already applied exact legacy ACL baseline. This
/// proves the original UNKNOWN custody was fenced and physically retired at
/// completion; callers still make a fresh kernel holder-gone observation.
pub(crate) fn read_applied_legacy_retirement(
    db:&VerifiedDatabaseConnection<'_>,instance:&str)->Result<Option<LegacyFenceRecord>> {
    let Some(fence)=read_legacy_fence(db,instance)? else{return Ok(None)};
    check_binding(db,instance,&fence.database_identity,&fence.root_identity,
        &fence.home_identity,&fence.source_identity,&fence.source_parent_identity,
        fence.source_revision)?;
    check_custody(db,&fence)?;
    let baseline=read_legacy_step(db,instance,LegacyAclStep::Baseline)?
        .ok_or(LegacyFenceError::Unsafe)?;
    if baseline.phase!=LegacyStepPhase::Applied {return Err(LegacyFenceError::Unsafe)}
    Ok(Some(fence))
}
fn eligible_holders(fence: &LegacyFenceRecord, proof: &LegacyPhysicalProof) -> Result<()> {
    let pairs = fence.custody.iter().map(LegacyCustodyRow::native_pair).collect::<Result<Vec<_>>>()?;
    proof.holders_gone.validate(&pairs).map_err(LegacyFenceError::NativeHolders)
}
/// Read-only qualification immediately before a native ACL write. A pending
/// aggregate ACL may be between endpoints; the native sealed snapshot must
/// reconcile each object. This returns only this exact journaled action, not
/// instance quiescence or any authority to clean later grants.
pub(crate) fn validate_legacy_acl_write(db: &VerifiedDatabaseConnection<'_>, expected: &LegacyStepIntent,
    proof: &LegacyPhysicalProof) -> Result<LegacyStepIntent> {
    atom(&expected.instance_id)?;
    let fence = read_legacy_fence(db, &expected.instance_id)?.ok_or(LegacyFenceError::Unsafe)?;
    eligible_holders(&fence, proof)?;
    check_proof(db, &fence, proof)?;
    let recorded = read_legacy_step(db, &expected.instance_id, expected.step)?.ok_or(LegacyFenceError::Unsafe)?;
    if recorded != *expected || recorded.phase != LegacyStepPhase::Pending ||
        recorded.applied_revision.is_some() || recorded.intent_revision != fence.revision {
        return Err(LegacyFenceError::Conflict);
    }
    for value in [&recorded.before_home_acl_digest, &recorded.before_source_acl_digest,
        &recorded.target_home_acl_digest, &recorded.target_source_acl_digest] { digest(value)?; }
    match recorded.step {
        LegacyAclStep::Home => {
            if read_legacy_step(db, &recorded.instance_id, LegacyAclStep::Baseline)?.is_some() ||
                recorded.before_home_acl_digest != fence.home_acl_digest ||
                recorded.before_source_acl_digest != fence.source_acl_digest ||
                (recorded.target_home_acl_digest == recorded.before_home_acl_digest &&
                    recorded.target_source_acl_digest == recorded.before_source_acl_digest) {
                return Err(LegacyFenceError::Unsafe);
            }
        },
        LegacyAclStep::Baseline => {
            let home = read_legacy_step(db, &recorded.instance_id, LegacyAclStep::Home)?.ok_or(LegacyFenceError::Unsafe)?;
            if home.phase != LegacyStepPhase::Applied ||
                home.target_home_acl_digest != recorded.before_home_acl_digest ||
                home.target_source_acl_digest != recorded.before_source_acl_digest ||
                home.target_home_acl_digest != recorded.target_home_acl_digest ||
                recorded.target_source_acl_digest == recorded.before_source_acl_digest {
                return Err(LegacyFenceError::Unsafe);
            }
        },
    }
    Ok(recorded)
}
pub(crate) fn begin_legacy_acl_step(db: &mut VerifiedDatabaseConnection<'_>, input: &LegacyStepRequest<'_>) -> Result<LegacyStepIntent> {
    atom(input.instance_id)?; atom(input.request_id)?;
    digest(input.target_home_acl_digest)?; digest(input.target_source_acl_digest)?;
    transaction(db, |db| {
        let fence = read_legacy_fence(db, input.instance_id)?.ok_or(LegacyFenceError::Unsafe)?;
        eligible_holders(&fence, input.proof)?;
        check_proof(db, &fence, input.proof)?;
        // PENDING is read back through read_legacy_step. Native restore must
        // reconcile each captured ACL before finish; an aggregate digest can
        // be neither endpoint during a partial tree migration.
        if read_legacy_step(db, input.instance_id, input.step)?.is_some() { return Err(LegacyFenceError::Conflict); }
        if fence.revision != input.expected_revision { return Err(LegacyFenceError::Conflict); }
        let before = (&input.proof.actual_home_acl_digest, &input.proof.actual_source_acl_digest);
        match input.step {
            LegacyAclStep::Home => {
                if read_legacy_step(db, input.instance_id, LegacyAclStep::Baseline)?.is_some() ||
                    before.0 != &fence.home_acl_digest || before.1 != &fence.source_acl_digest ||
                    (input.target_home_acl_digest == before.0 && input.target_source_acl_digest == before.1) {
                    return Err(LegacyFenceError::Unsafe);
                }
            },
            LegacyAclStep::Baseline => {
                let home = read_legacy_step(db, input.instance_id, LegacyAclStep::Home)?.ok_or(LegacyFenceError::Unsafe)?;
                if home.phase != LegacyStepPhase::Applied ||
                    before.0 != &home.target_home_acl_digest || before.1 != &home.target_source_acl_digest ||
                    input.target_home_acl_digest != home.target_home_acl_digest ||
                    input.target_source_acl_digest == before.1 { return Err(LegacyFenceError::Unsafe); }
            },
        }
        let next = fence.revision.checked_add(1).ok_or(LegacyFenceError::Invalid("revision overflow"))?;
        let q = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_legacy_acl_steps VALUES(?1,?2,'PENDING',?3,?4,?5,?6,?7,?8,?9,NULL)")?;
        let observed_start = format!("boot-start:{:016x}", input.proof.holders_gone.boot_start());
        bind(&q, &[input.instance_id, input.step.text(), input.request_id, &observed_start,
            before.0, before.1, input.target_home_acl_digest, input.target_source_acl_digest])?;
        q.bind_i64(9, next)?; q.step_done()?;
        let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_legacy_acl_fences SET revision=?3 WHERE instance_id=?1 AND revision=?2")?;
        q.bind_text(1, input.instance_id)?; q.bind_i64(2, fence.revision)?; q.bind_i64(3, next)?; q.step_done()?; one_change(db)?;
        read_legacy_step(db, input.instance_id, input.step)?.ok_or(LegacyFenceError::Conflict)
    })
}
pub(crate) fn finish_legacy_acl_step(db: &mut VerifiedDatabaseConnection<'_>, input: &LegacyStepFinish<'_>) -> Result<LegacyFenceRecord> {
    transaction(db, |db| {
        let fence = read_legacy_fence(db, input.instance_id)?.ok_or(LegacyFenceError::Unsafe)?;
        eligible_holders(&fence, input.proof)?;
        check_proof(db, &fence, input.proof)?;
        let step = read_legacy_step(db, input.instance_id, input.step)?.ok_or(LegacyFenceError::Unsafe)?;
        if step.phase != LegacyStepPhase::Pending || step.request_id != input.request_id ||
            step.intent_revision != input.expected_intent_revision ||
            fence.revision != step.intent_revision { return Err(LegacyFenceError::Conflict); }
        match input.step {
            LegacyAclStep::Home => {},
            LegacyAclStep::Baseline => {
                let home = read_legacy_step(db, input.instance_id, LegacyAclStep::Home)?.ok_or(LegacyFenceError::Unsafe)?;
                if home.phase != LegacyStepPhase::Applied ||
                    step.before_home_acl_digest != home.target_home_acl_digest ||
                    step.before_source_acl_digest != home.target_source_acl_digest ||
                    step.target_home_acl_digest != home.target_home_acl_digest { return Err(LegacyFenceError::Unsafe); }
            },
        }
        if input.proof.actual_home_acl_digest != step.target_home_acl_digest ||
            input.proof.actual_source_acl_digest != step.target_source_acl_digest {
            return Err(LegacyFenceError::Unsafe);
        }
        let next = fence.revision.checked_add(1).ok_or(LegacyFenceError::Invalid("revision overflow"))?;
        let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_legacy_acl_steps SET phase='APPLIED',applied_revision=?5 WHERE instance_id=?1 AND step=?2 AND request_id=?3 AND phase='PENDING' AND intent_revision=?4")?;
        bind(&q, &[input.instance_id, input.step.text(), input.request_id])?;
        q.bind_i64(4, step.intent_revision)?; q.bind_i64(5, next)?; q.step_done()?; one_change(db)?;
        let q = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_legacy_acl_fences SET revision=?3 WHERE instance_id=?1 AND revision=?2")?;
        q.bind_text(1, input.instance_id)?; q.bind_i64(2, fence.revision)?; q.bind_i64(3, next)?; q.step_done()?; one_change(db)?;
        read_legacy_fence(db, input.instance_id)?.ok_or(LegacyFenceError::Conflict)
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn id(byte: u8) -> RootIdentity { RootIdentity { volume_serial: 1, file_id: [byte; 16] } }
    fn hash(byte: char) -> String { byte.to_string().repeat(64) }
    fn fixture(run: impl for<'a> FnOnce(&'a RootLock, &std::path::Path, VerifiedDatabaseConnection<'a>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-legacy-fence-{}-{nonce}", std::process::id()));
        fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let path = root_path.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        super::super::initialize_credential_schema(&mut db).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        initialize_legacy_fence_schema(&mut db).unwrap();
        let instance = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_instances VALUES('instanceA','codex','homeA',?1,'sha256:fixture','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
        instance.bind_text(1, &id(1).opaque()).unwrap(); instance.step_done().unwrap(); drop(instance);
        let source = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_credential_objects VALUES('instanceA',?1,?2,?3,?2,'ACTIVE',1)").unwrap();
        bind(&source, &[&db.root_identity().opaque(), &id(1).opaque(), &id(3).opaque()]).unwrap();
        source.step_done().unwrap(); drop(source);
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('old','ticketA','nonceA','5','7','old-image','sha256:old','instanceA','global','1','UNKNOWN',NULL)").unwrap();
        run(&root, &path, db);
        drop(root);
        fs::remove_dir_all(root_path).unwrap();
    }
    fn capture(db: &VerifiedDatabaseConnection<'_>) -> LegacyFenceCapture {
        LegacyFenceCapture {
            instance_id: "instanceA".into(), original_boot: "bootA".into(),
            database_identity: db.identity().clone(), root_identity: db.root_identity().clone(),
            home_identity: id(1), source_identity: id(3), source_parent_identity: id(1), source_revision: Some(1),
            source_link_count: 1, registered_alias_count: 0, custody: live_custody(db, "instanceA").unwrap(),
            home_acl_digest: hash('a'), source_acl_digest: hash('b'), acl_provenance_digest: hash('c'),
            native_snapshot: "000102".into(), native_snapshot_digest: sha256_hex(&[0, 1, 2]),
        }
    }
    fn proof(db: &VerifiedDatabaseConnection<'_>, home: char, source: char) -> LegacyPhysicalProof {
        LegacyPhysicalProof {
            database_identity: db.identity().clone(), root_identity: db.root_identity().clone(),
            home_identity: id(1), source_identity: id(3), source_parent_identity: id(1), source_revision: Some(1),
            source_link_count: 1, registered_alias_count: 0, acl_provenance_digest: hash('c'),
            actual_home_acl_digest: hash(home), actual_source_acl_digest: hash(source),
            holders_gone: crate::process::NativeLegacyHoldersGone::for_test(&[(5, 7)], 100),
        }
    }

    #[test]
    fn exact_schema_reopen_and_two_persistent_intents() {
        fixture(|root, path, mut db| {
            let captured = capture(&db);
            let fence = capture_legacy_fence(&mut db, &captured).unwrap();
            assert_eq!(fence.custody[0].state, "UNKNOWN");
            assert_eq!(fence.custody[0].stop_proof_hash, None);
            assert!(matches!(capture_legacy_fence(&mut db, &captured), Err(LegacyFenceError::Conflict)));
            let initial = proof(&db, 'a', 'b');

            let home = begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_revision: 1, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &initial,
            }).unwrap();
            assert_eq!(home.phase, LegacyStepPhase::Pending);
            assert_eq!(home.before_source_acl_digest, hash('b'));
            assert_eq!(home.target_source_acl_digest, hash('f'));
            db.close_checked().unwrap();
            let mut db = open_existing(root, path).unwrap();
            db.execute("PRAGMA foreign_keys=ON").unwrap();
            initialize_legacy_fence_schema(&mut db).unwrap();
            assert_eq!(read_legacy_step(&db, "instanceA", LegacyAclStep::Home).unwrap(), Some(home.clone()));
            let partial = proof(&db, 'd', 'b');
            assert_eq!(validate_legacy_acl_write(&db, &home, &partial).unwrap(), home);
            let mut mismatched = proof(&db, 'd', 'b');
            mismatched.holders_gone = crate::process::NativeLegacyHoldersGone::for_test(&[(6, 7)], 100);
            assert!(matches!(validate_legacy_acl_write(&db, &home, &mismatched),
                Err(LegacyFenceError::NativeHolders(_))));
            assert!(matches!(begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_revision: 2, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &partial,
            }), Err(LegacyFenceError::Conflict)));
            assert!(matches!(finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &partial,
            }), Err(LegacyFenceError::Unsafe)));
            assert!(matches!(finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &partial,
            }), Err(LegacyFenceError::Unsafe)));
            let home_done = proof(&db, 'd', 'f');
            assert!(matches!(begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_revision: 2, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('e'), proof: &home_done,
            }), Err(LegacyFenceError::Unsafe)));
            finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &home_done,
            }).unwrap();
            assert_eq!(read_legacy_step(&db, "instanceA", LegacyAclStep::Home).unwrap().unwrap().current_boot, "boot-start:0000000000000064");
            let stale_source = proof(&db, 'd', 'b');
            assert!(matches!(begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_revision: 3, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('e'), proof: &stale_source,
            }), Err(LegacyFenceError::Unsafe)));
            assert!(matches!(begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeAgain", expected_revision: 3, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &home_done,
            }), Err(LegacyFenceError::Conflict)));
            let baseline = begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_revision: 3, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('e'), proof: &home_done,
            }).unwrap();
            assert_eq!(baseline.phase, LegacyStepPhase::Pending);
            assert_eq!(baseline.before_source_acl_digest, home.target_source_acl_digest);
            assert!(matches!(finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_intent_revision: 4, proof: &home_done,
            }), Err(LegacyFenceError::Unsafe)));
            let baseline_done = proof(&db, 'd', 'e');
            finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_intent_revision: 4, proof: &baseline_done,
            }).unwrap();
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn immutable_old_rows_and_new_unknown_refuse_but_new_stopped_does_not_join_fence() {
        fixture(|_, _, mut db| {
            let captured = capture(&db);
            capture_legacy_fence(&mut db, &captured).unwrap();
            db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('new','ticketB','nonceB','8','9','new-image','sha256:new','instanceA','global','2','STOPPED','proofB')").unwrap();
            let before = proof(&db, 'a', 'b');
            let home_target = hash('d');
            let source_target = hash('f');
            let request = LegacyStepRequest { instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA",
                expected_revision: 1,
                target_home_acl_digest: &home_target, target_source_acl_digest: &source_target, proof: &before };
            begin_legacy_acl_step(&mut db, &request).unwrap();
            db.execute("UPDATE main.gogoke_coordination_process_custody SET state='UNKNOWN',stop_proof_hash=NULL WHERE operation_id='new'").unwrap();
            let after = proof(&db, 'd', 'f');
            assert!(matches!(finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &after,
            }), Err(LegacyFenceError::Unsafe)));
            db.execute("UPDATE main.gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='proofB' WHERE operation_id='new'").unwrap();
            db.execute("UPDATE main.gogoke_coordination_process_custody SET ticket='altered' WHERE operation_id='old'").unwrap();
            assert!(matches!(finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &after,
            }), Err(LegacyFenceError::Conflict)));
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn home_applied_allows_only_same_fence_baseline_with_fresh_holder_proof() {
        fixture(|_, _, mut db| {
            let original = capture(&db);
            capture_legacy_fence(&mut db, &original).unwrap();
            let before = proof(&db, 'a', 'b');
            begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_revision: 1, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &before,
            }).unwrap();
            let after_home = proof(&db, 'd', 'f');
            finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &after_home,
            }).unwrap();


            let baseline = begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_revision: 3, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('e'), proof: &after_home,
            }).unwrap();
            assert_eq!(baseline.current_boot, "boot-start:0000000000000064");
            let after_baseline = proof(&db, 'd', 'e');
            finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_intent_revision: 4, proof: &after_baseline,
            }).unwrap();
            assert_eq!(read_legacy_step(&db, "instanceA", LegacyAclStep::Home).unwrap().unwrap().current_boot, "boot-start:0000000000000064");
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn unregistered_original_source_is_fenced_without_fabricating_registry_row() {
        fixture(|_, _, mut db| {
            db.execute("DELETE FROM main.gogoke_v37_credential_objects WHERE instance_id='instanceA'").unwrap();
            let mut input = capture(&db);
            input.source_revision = None;
            let fence = capture_legacy_fence(&mut db, &input).unwrap();
            assert_eq!(fence.source_revision, None);
            let q = Statement::prepare(db.as_ptr(), "SELECT 1 FROM main.gogoke_v37_credential_objects WHERE instance_id='instanceA'").unwrap();
            assert!(!q.step_row().unwrap()); drop(q);
            let mut current = proof(&db, 'a', 'b');
            current.source_revision = None;
            begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_revision: 1, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &current,
            }).unwrap();
            current.actual_home_acl_digest = hash('d');
            current.actual_source_acl_digest = hash('f');
            finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_intent_revision: 2, proof: &current,
            }).unwrap();
            begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_revision: 3, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('e'), proof: &current,
            }).unwrap();
            current.actual_source_acl_digest = hash('e');
            finish_legacy_acl_step(&mut db, &LegacyStepFinish {
                instance_id: "instanceA", step: LegacyAclStep::Baseline, request_id: "baselineA", expected_intent_revision: 4, proof: &current,
            }).unwrap();
            let source = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_credential_objects VALUES('instanceA',?1,?2,?3,?2,'ACTIVE',1)").unwrap();
            bind(&source, &[&db.root_identity().opaque(), &id(1).opaque(), &id(3).opaque()]).unwrap();
            source.step_done().unwrap(); drop(source);
            current.source_revision = Some(1);
            assert_eq!(read_legacy_fence(&db, "instanceA").unwrap().unwrap().source_revision, None);
            assert_eq!(read_legacy_step(&db, "instanceA", LegacyAclStep::Baseline).unwrap().unwrap().phase,
                LegacyStepPhase::Applied);
            assert!(matches!(begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeAgain", expected_revision: 5, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &current,
            }), Err(LegacyFenceError::Conflict)));
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn schema_shadow_trigger_and_snapshot_mutation_refuse() {
        fixture(|_, _, mut db| {
            db.execute("CREATE TEMP TABLE gogoke_v37_legacy_acl_fences(instance_id TEXT)").unwrap();
            assert!(matches!(initialize_legacy_fence_schema(&mut db), Err(LegacyFenceError::Schema)));
            db.execute("DROP TABLE temp.gogoke_v37_legacy_acl_fences").unwrap();
            db.execute("CREATE TRIGGER unexpected BEFORE UPDATE ON gogoke_v37_legacy_acl_fences BEGIN SELECT RAISE(ABORT,'blocked'); END").unwrap();
            assert!(matches!(initialize_legacy_fence_schema(&mut db), Err(LegacyFenceError::Schema)));
            db.execute("DROP TRIGGER unexpected").unwrap();
            let input = capture(&db);
            capture_legacy_fence(&mut db, &input).unwrap();
            db.execute("UPDATE main.gogoke_v37_legacy_acl_fences SET native_snapshot='abcd' WHERE instance_id='instanceA'").unwrap();
            assert!(matches!(read_legacy_fence(&db, "instanceA"), Err(LegacyFenceError::Conflict)));
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn prewrite_validation_refuses_late_source_registration() {
        fixture(|_, _, mut db| {
            db.execute("DELETE FROM main.gogoke_v37_credential_objects WHERE instance_id='instanceA'").unwrap();
            let mut input = capture(&db);
            input.source_revision = None;
            capture_legacy_fence(&mut db, &input).unwrap();
            let mut physical = proof(&db, 'a', 'b');
            physical.source_revision = None;
            let pending = begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_revision: 1, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &physical,
            }).unwrap();
            assert!(validate_legacy_acl_write(&db, &pending, &physical).is_ok());
            let source = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_credential_objects VALUES('instanceA',?1,?2,?3,?2,'ACTIVE',1)").unwrap();
            bind(&source, &[&db.root_identity().opaque(), &id(1).opaque(), &id(3).opaque()]).unwrap();
            source.step_done().unwrap(); drop(source);
            assert!(matches!(validate_legacy_acl_write(&db, &pending, &physical),
                Err(LegacyFenceError::Conflict)));
            db.close_checked().unwrap();
        });
    }

    #[test]
    fn prewrite_validation_checks_same_open_database_and_credential_journal() {
        fixture(|_, _, mut db| {
            let input = capture(&db);
            capture_legacy_fence(&mut db, &input).unwrap();
            let mut physical = proof(&db, 'a', 'b');
            let pending = begin_legacy_acl_step(&mut db, &LegacyStepRequest {
                instance_id: "instanceA", step: LegacyAclStep::Home, request_id: "homeA", expected_revision: 1, target_home_acl_digest: &hash('d'), target_source_acl_digest: &hash('f'), proof: &physical,
            }).unwrap();
            let original_db = db.identity().opaque();
            let changed = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_legacy_acl_fences SET database_identity=?1 WHERE instance_id='instanceA'").unwrap();
            changed.bind_text(1, &id(9).opaque()).unwrap(); changed.step_done().unwrap(); drop(changed);
            physical.database_identity = id(9);
            assert!(matches!(validate_legacy_acl_write(&db, &pending, &physical),
                Err(LegacyFenceError::Conflict)));
            let restore = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_legacy_acl_fences SET database_identity=?1 WHERE instance_id='instanceA'").unwrap();
            restore.bind_text(1, &original_db).unwrap(); restore.step_done().unwrap(); drop(restore);
            physical.database_identity = db.identity().clone();
            assert!(validate_legacy_acl_write(&db, &pending, &physical).is_ok());
            let target = format!("credential-instance-{}", sha256_hex(b"instanceA"));
            let journal = Statement::prepare(db.as_ptr(), "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES('newIntent','newIntent',?1,'PREPARING')").unwrap();
            journal.bind_text(1, &target).unwrap(); journal.step_done().unwrap(); drop(journal);
            assert!(matches!(validate_legacy_acl_write(&db, &pending, &physical),
                Err(LegacyFenceError::Unsafe)));
            db.close_checked().unwrap();
        });
    }
}
