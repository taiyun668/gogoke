//! Independent F metadata journal for an exact disappeared parent identity.
//! Root separately supplies retired-Job authority and the complete opaque
//! metadata/ACL capture. This module proves no descendants, StopFact, ACL
//! effect, H release, or permission to continue an old session. It opens no
//! credential file and writes only its own journal table.

use crate::process::{NativeLegacyHoldersGoneError, NativeProcessHoldersGone};
use crate::store::atomic::{AtomicError, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

const TABLE: &str = "gogoke_v37_holder_disappearance";
const SCHEMA: &str = "CREATE TABLE gogoke_v37_holder_disappearance(binding_id TEXT PRIMARY KEY REFERENCES gogoke_v37_credential_profiles(binding_id),instance_id TEXT NOT NULL REFERENCES gogoke_v37_credential_objects(instance_id),process_operation_id TEXT NOT NULL UNIQUE REFERENCES gogoke_coordination_process_custody(operation_id),request_id TEXT NOT NULL UNIQUE,pid TEXT NOT NULL,creation_time_100ns TEXT NOT NULL,snapshot_hex TEXT NOT NULL,snapshot_digest TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('PREPARING','REVOKED','APPLIED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision>=1)) STRICT";
const MAX_CAPTURE_BYTES: usize = 65_536;

#[derive(Debug)]
pub(crate) enum HolderDisappearanceError {
    Invalid(&'static str), Conflict, Unknown, Schema,
    Atomic(AtomicError), Sqlite(SameOpenError),
    Number(std::num::ParseIntError), NativeHolders(NativeLegacyHoldersGoneError),
    CommitUnknown(SameOpenError),
    RollbackUnknown { original: Box<HolderDisappearanceError>, rollback: SameOpenError },
}
impl From<AtomicError> for HolderDisappearanceError {
    fn from(value: AtomicError) -> Self { Self::Atomic(value) }
}
impl From<SameOpenError> for HolderDisappearanceError {
    fn from(value: SameOpenError) -> Self { Self::Sqlite(value) }
}
type Result<T> = std::result::Result<T, HolderDisappearanceError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HolderDisappearancePhase { Preparing, Revoked, Applied, Unknown }
impl HolderDisappearancePhase {
    pub(crate) fn text(self) -> &'static str {
        match self { Self::Preparing => "PREPARING", Self::Revoked => "REVOKED",
            Self::Applied => "APPLIED", Self::Unknown => "UNKNOWN" }
    }
    fn parse(value: &str) -> Result<Self> {
        match value { "PREPARING" => Ok(Self::Preparing), "REVOKED" => Ok(Self::Revoked),
            "APPLIED" => Ok(Self::Applied), "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(HolderDisappearanceError::Invalid("journal phase")) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HolderDisappearanceInput {
    pub(crate) binding_id: String,
    pub(crate) instance_id: String,
    pub(crate) process_operation_id: String,
    pub(crate) request_id: String,
    pub(crate) pid: String,
    pub(crate) creation_time_100ns: String,
    pub(crate) snapshot_hex: String,
    pub(crate) snapshot_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HolderDisappearanceRecord {
    pub(crate) input: HolderDisappearanceInput,
    pub(crate) phase: HolderDisappearancePhase,
    pub(crate) revision: i64,
}

unsafe extern "C" { fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> i32; }

fn require_transaction(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    if unsafe { sqlite3_get_autocommit(db.as_ptr()) } != 0 {
        return Err(HolderDisappearanceError::Invalid("open transaction required"));
    }
    Ok(())
}
fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>,
    work: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T>) -> Result<T> {
    if unsafe { sqlite3_get_autocommit(db.as_ptr()) } == 0 {
        return Err(HolderDisappearanceError::Invalid("journal requires its own transaction"));
    }
    db.execute("BEGIN IMMEDIATE")?;
    match work(db) {
        Ok(value) => { db.execute("COMMIT").map_err(HolderDisappearanceError::CommitUnknown)?; Ok(value) },
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(HolderDisappearanceError::RollbackUnknown {
                original: Box::new(original), rollback }),
        },
    }
}
fn bind(statement: &Statement, values: &[&str]) -> Result<()> {
    for (index, value) in values.iter().enumerate() {
        statement.bind_text((index + 1) as i32, value)?;
    }
    Ok(())
}
fn reject_side_effects(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(name)=?1 OR lower(tbl_name)=?1 LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(tbl_name)=?1 LIMIT 1",
    ] {
        let statement = Statement::prepare(db.as_ptr(), sql)?;
        statement.bind_text(1, TABLE)?;
        if statement.step_row()? { return Err(HolderDisappearanceError::Schema); }
    }
    Ok(())
}
fn has_schema(db: &VerifiedDatabaseConnection<'_>) -> Result<bool> {
    reject_side_effects(db)?;
    let statement = Statement::prepare(db.as_ptr(),
        "SELECT name,sql,type FROM main.sqlite_schema WHERE lower(name)=?1")?;
    statement.bind_text(1, TABLE)?;
    if !statement.step_row()? { return Ok(false); }
    if statement.column_text(0)? != TABLE || statement.column_text(1)? != SCHEMA ||
        statement.column_text(2)? != "table" || statement.step_row()? {
        return Err(HolderDisappearanceError::Schema);
    }
    Ok(true)
}
pub(crate) fn initialize_holder_disappearance_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    if has_schema(db)? { return Ok(()); }
    transaction(db, |db| {
        if has_schema(db)? { return Err(HolderDisappearanceError::Conflict); }
        db.execute(SCHEMA)?;
        if !has_schema(db)? { return Err(HolderDisappearanceError::Schema); }
        Ok(())
    })
}

fn atom(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}
fn pair(input: &HolderDisappearanceInput) -> Result<(u32, u64)> {
    let pid = input.pid.parse::<u32>().map_err(HolderDisappearanceError::Number)?;
    let creation = input.creation_time_100ns.parse::<u64>().map_err(HolderDisappearanceError::Number)?;
    if pid == 0 || creation == 0 || input.pid != pid.to_string() ||
        input.creation_time_100ns != creation.to_string() {
        return Err(HolderDisappearanceError::Invalid("canonical exact process pair"));
    }
    Ok((pid, creation))
}
fn validate_input(input: &HolderDisappearanceInput) -> Result<()> {
    if [&input.binding_id, &input.instance_id, &input.process_operation_id, &input.request_id]
        .iter().any(|value| !atom(value)) {
        return Err(HolderDisappearanceError::Invalid("journal identity"));
    }
    pair(input)?;
    let hex = input.snapshot_hex.as_bytes();
    if hex.is_empty() || hex.len() > MAX_CAPTURE_BYTES * 2 || hex.len() % 2 != 0 ||
        hex.iter().any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(byte)) {
        return Err(HolderDisappearanceError::Invalid("bounded canonical snapshot hex"));
    }
    let mut capture = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.chunks_exact(2) {
        let digit = |byte: u8| if byte.is_ascii_digit() { byte - b'0' } else { byte - b'a' + 10 };
        capture.push((digit(chunk[0]) << 4) | digit(chunk[1]));
    }
    if input.snapshot_digest != sha256_hex(&capture) {
        return Err(HolderDisappearanceError::Invalid("snapshot digest"));
    }
    Ok(())
}
fn validate_association(db: &VerifiedDatabaseConnection<'_>, input: &HolderDisappearanceInput) -> Result<()> {
    let statement = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_credential_objects o ON o.instance_id=p.instance_id JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id JOIN main.gogoke_v37_instance_history_generations g ON g.binding_id=p.binding_id AND g.history_id=p.history_id AND g.generation=p.generation JOIN main.gogoke_v37_instance_histories h ON h.history_id=p.history_id AND h.instance_id=p.instance_id JOIN main.gogoke_v37_credential_aliases a ON a.history_id=p.history_id AND a.instance_id=p.instance_id AND a.directory_identity=h.directory_identity AND a.source_file_identity=p.source_file_identity JOIN main.gogoke_coordination_process_custody c ON c.operation_id=g.process_operation_id AND c.ticket=g.ticket AND c.custodian_nonce=g.custodian_nonce AND c.profile_id=p.instance_id AND c.domain_id=h.domain_id AND c.generation=p.generation WHERE p.binding_id=?1 AND p.instance_id=?2 AND c.operation_id=?3 AND c.pid=?4 AND c.creation_time_100ns=?5 AND o.root_identity=?6 AND h.root_identity=?6 AND o.home_identity=i.home_identity AND h.home_identity=i.home_identity AND o.source_parent_identity=i.home_identity AND p.source_file_identity=o.file_identity")?;
    bind(&statement, &[&input.binding_id, &input.instance_id, &input.process_operation_id,
        &input.pid, &input.creation_time_100ns, &db.root_identity().opaque()])?;
    if !statement.step_row()? || statement.step_row()? { return Err(HolderDisappearanceError::Conflict); }
    Ok(())
}
fn validate_proof(input: &HolderDisappearanceInput, proof: &NativeProcessHoldersGone) -> Result<()> {
    proof.validate(&[pair(input)?]).map_err(HolderDisappearanceError::NativeHolders)
}
fn read_exact(db: &VerifiedDatabaseConnection<'_>, binding: &str) -> Result<Option<HolderDisappearanceRecord>> {
    let statement = Statement::prepare(db.as_ptr(),
        "SELECT binding_id,instance_id,process_operation_id,request_id,pid,creation_time_100ns,snapshot_hex,snapshot_digest,phase,revision FROM main.gogoke_v37_holder_disappearance WHERE binding_id=?1")?;
    statement.bind_text(1, binding)?;
    if !statement.step_row()? { return Ok(None); }
    let input = HolderDisappearanceInput {
        binding_id: statement.column_text(0)?, instance_id: statement.column_text(1)?,
        process_operation_id: statement.column_text(2)?, request_id: statement.column_text(3)?,
        pid: statement.column_text(4)?, creation_time_100ns: statement.column_text(5)?,
        snapshot_hex: statement.column_text(6)?, snapshot_digest: statement.column_text(7)?,
    };
    let phase = HolderDisappearancePhase::parse(&statement.column_text(8)?)?;
    let revision = statement.column_text(9)?.parse::<i64>().map_err(HolderDisappearanceError::Number)?;
    if revision < 1 || statement.step_row()? { return Err(HolderDisappearanceError::Conflict); }
    validate_input(&input)?;
    Ok(Some(HolderDisappearanceRecord { input, phase, revision }))
}
pub(crate) fn read_holder_disappearance(db: &VerifiedDatabaseConnection<'_>, binding: &str)
    -> Result<Option<HolderDisappearanceRecord>> {
    if !atom(binding) { return Err(HolderDisappearanceError::Invalid("binding id")); }
    if !has_schema(db)? { return Ok(None); }
    read_exact(db, binding)
}
pub(crate) fn begin_holder_disappearance(db: &mut VerifiedDatabaseConnection<'_>,
    input: &HolderDisappearanceInput, proof: &NativeProcessHoldersGone) -> Result<HolderDisappearanceRecord> {
    validate_input(input)?;
    validate_proof(input, proof)?;
    transaction(db, |db| {
        if !has_schema(db)? { return Err(HolderDisappearanceError::Schema); }
        validate_association(db, input)?;
        validate_proof(input, proof)?;
        if let Some(record) = read_exact(db, &input.binding_id)? {
            if record.input != *input { return Err(HolderDisappearanceError::Conflict); }
            if record.phase == HolderDisappearancePhase::Unknown { return Err(HolderDisappearanceError::Unknown); }
            validate_proof(input, proof)?;
            return Ok(record);
        }
        let occupied = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_holder_disappearance WHERE request_id=?1 OR process_operation_id=?2 LIMIT 1")?;
        bind(&occupied, &[&input.request_id, &input.process_operation_id])?;
        if occupied.step_row()? { return Err(HolderDisappearanceError::Conflict); }
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_holder_disappearance(binding_id,instance_id,process_operation_id,request_id,pid,creation_time_100ns,snapshot_hex,snapshot_digest,phase,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'PREPARING',1)")?;
        bind(&insert, &[&input.binding_id, &input.instance_id, &input.process_operation_id, &input.request_id,
            &input.pid, &input.creation_time_100ns, &input.snapshot_hex, &input.snapshot_digest])?;
        insert.step_done()?;
        validate_proof(input, proof)?;
        let record = read_exact(db, &input.binding_id)?.ok_or(HolderDisappearanceError::Conflict)?;
        if record.input != *input || record.phase != HolderDisappearancePhase::Preparing || record.revision != 1 {
            return Err(HolderDisappearanceError::Conflict);
        }
        validate_proof(input, proof)?;
        Ok(record)
    })
}

/// Root composes this CAS with its own H release transaction. A native proof
/// alone is never retired-Job/ACL authority. UNKNOWN cannot advance or be
/// overwritten by another request; an explicit terminal UNKNOWN stays a fence.
pub(crate) fn advance_in_transaction(db: &VerifiedDatabaseConnection<'_>, record: &HolderDisappearanceRecord,
    expected: HolderDisappearancePhase, next: HolderDisappearancePhase,
    proof: &NativeProcessHoldersGone) -> Result<HolderDisappearanceRecord> {
    require_transaction(db)?;
    if !has_schema(db)? { return Err(HolderDisappearanceError::Schema); }
    validate_input(&record.input)?;
    if record.phase != expected || record.revision < 1 {
        return Err(HolderDisappearanceError::Conflict);
    }
    let current = read_exact(db, &record.input.binding_id)?.ok_or(HolderDisappearanceError::Conflict)?;
    if current != *record { return Err(HolderDisappearanceError::Conflict); }
    if expected == HolderDisappearancePhase::Unknown && next != HolderDisappearancePhase::Unknown {
        return Err(HolderDisappearanceError::Unknown);
    }
    if next != HolderDisappearancePhase::Unknown && !matches!((expected, next),
        (HolderDisappearancePhase::Preparing, HolderDisappearancePhase::Revoked) |
        (HolderDisappearancePhase::Revoked, HolderDisappearancePhase::Applied)) {
        return Err(HolderDisappearanceError::Invalid("journal transition"));
    }
    validate_association(db, &record.input)?;
    validate_proof(&record.input, proof)?;
    if expected == HolderDisappearancePhase::Unknown { return Ok(current); }
    let revision = record.revision.checked_add(1).ok_or(HolderDisappearanceError::Invalid("revision overflow"))?;
    let update = Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_holder_disappearance SET phase=?9,revision=?10 WHERE binding_id=?1 AND instance_id=?2 AND process_operation_id=?3 AND request_id=?4 AND pid=?5 AND creation_time_100ns=?6 AND snapshot_hex=?7 AND snapshot_digest=?8 AND phase=?11 AND revision=?12")?;
    bind(&update, &[&record.input.binding_id, &record.input.instance_id, &record.input.process_operation_id,
        &record.input.request_id, &record.input.pid, &record.input.creation_time_100ns,
        &record.input.snapshot_hex, &record.input.snapshot_digest])?;
    update.bind_text(9, next.text())?; update.bind_i64(10, revision)?;
    update.bind_text(11, expected.text())?; update.bind_i64(12, record.revision)?;
    update.step_done()?;
    let changed = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !changed.step_row()? || changed.column_text(0)? != "1" { return Err(HolderDisappearanceError::Conflict); }
    validate_proof(&record.input, proof)?;
    let result = read_exact(db, &record.input.binding_id)?.ok_or(HolderDisappearanceError::Conflict)?;
    if result.input != record.input || result.phase != next || result.revision != revision {
        return Err(HolderDisappearanceError::Conflict);
    }
    validate_proof(&record.input, proof)?;
    Ok(result)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, HolderDisappearanceInput)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-holder-journal-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        super::super::initialize_credential_schema(&mut db).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances VALUES('instanceA','codex','homeA','homeIdentity','sha256:metadata','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
        let history = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_histories VALUES('historyA','instanceA',?1,'parentIdentity','homeIdentity','domainA','sessionA','seatA','incarnationA','bindingA','1','openA','history-historyA','directoryIdentity','READY',1)").unwrap();
        history.bind_text(1, &db.root_identity().opaque()).unwrap(); history.step_done().unwrap(); drop(history);
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('processA','ticketA','nonceA','4294967295','1','metadata-image','sha256:metadata','instanceA','domainA','1','UNKNOWN',NULL)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_history_generations VALUES('bindingA','historyA','1','openA',NULL,'processA','ticketA','nonceA')").unwrap();
        let object = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_credential_objects VALUES('instanceA',?1,'homeIdentity','sourceIdentity','homeIdentity','ACTIVE',1)").unwrap();
        object.bind_text(1, &db.root_identity().opaque()).unwrap(); object.step_done().unwrap(); drop(object);
        db.execute("INSERT INTO main.gogoke_v37_credential_aliases VALUES('historyA','instanceA','directoryIdentity','sourceIdentity','aliasOriginal','ACTIVE',2)").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_credential_profiles VALUES('bindingA','instanceA','historyA','1','profileSid','sourceIdentity','originalGrant','ACTIVE',2)").unwrap();
        initialize_holder_disappearance_schema(&mut db).unwrap();
        let capture = b"opaque complete Root metadata capture";
        run(&mut db, HolderDisappearanceInput { binding_id: "bindingA".into(), instance_id: "instanceA".into(),
            process_operation_id: "processA".into(), request_id: "goneA".into(), pid: "4294967295".into(),
            creation_time_100ns: "1".into(), snapshot_hex: capture.iter().map(|byte| format!("{byte:02x}")).collect(),
            snapshot_digest: sha256_hex(capture) });
        db.close_checked().unwrap(); drop(root);
        let actual = std::fs::canonicalize(&path).unwrap();
        assert!(actual.starts_with(std::fs::canonicalize(std::env::temp_dir()).unwrap()));
        assert!(actual.file_name().unwrap().to_string_lossy().starts_with("gogoke-holder-journal-"));
        std::fs::remove_dir_all(actual).unwrap();
    }
    fn gone(input: &HolderDisappearanceInput) -> NativeProcessHoldersGone {
        NativeProcessHoldersGone::observe(&[pair(input).unwrap()]).unwrap()
    }
    #[test]
    fn holder_journal_preserves_exact_capture_and_cas_without_other_effects() {
        fixture(|db, input| {
            let proof = gone(&input);
            let first = begin_holder_disappearance(db, &input, &proof).unwrap();
            assert_eq!(first.revision, 1);
            assert_eq!(begin_holder_disappearance(db, &input, &proof).unwrap(), first);
            let mut changed = input.clone(); changed.request_id = "goneB".into();
            assert!(matches!(begin_holder_disappearance(db, &changed, &proof), Err(HolderDisappearanceError::Conflict)));
            let mut changed = input.clone(); changed.snapshot_hex.push_str("00");
            let mut bytes = b"opaque complete Root metadata capture".to_vec(); bytes.push(0);
            changed.snapshot_digest = sha256_hex(&bytes);
            assert!(matches!(begin_holder_disappearance(db, &changed, &proof), Err(HolderDisappearanceError::Conflict)));
            assert!(advance_in_transaction(db, &first, HolderDisappearancePhase::Preparing,
                HolderDisappearancePhase::Revoked, &proof).is_err());
            let revoked = transaction(db, |db| advance_in_transaction(db, &first,
                HolderDisappearancePhase::Preparing, HolderDisappearancePhase::Revoked, &proof)).unwrap();
            assert_eq!(revoked.revision, 2);
            assert!(transaction(db, |db| advance_in_transaction(db, &first,
                HolderDisappearancePhase::Preparing, HolderDisappearancePhase::Revoked, &proof)).is_err());
            let applied = transaction(db, |db| advance_in_transaction(db, &revoked,
                HolderDisappearancePhase::Revoked, HolderDisappearancePhase::Applied, &proof)).unwrap();
            assert_eq!(applied.input, input); assert_eq!(applied.revision, 3);
            let custody = Statement::prepare(db.as_ptr(), "SELECT state,stop_proof_hash IS NULL FROM main.gogoke_coordination_process_custody WHERE operation_id='processA'").unwrap();
            assert!(custody.step_row().unwrap()); assert_eq!(custody.column_text(0).unwrap(), "UNKNOWN");
            assert_eq!(custody.column_text(1).unwrap(), "1");
            let profile = Statement::prepare(db.as_ptr(), "SELECT state,revision,intent_request FROM main.gogoke_v37_credential_profiles WHERE binding_id='bindingA'").unwrap();
            assert!(profile.step_row().unwrap()); assert_eq!(profile.column_text(0).unwrap(), "ACTIVE");
            assert_eq!(profile.column_text(1).unwrap(), "2"); assert_eq!(profile.column_text(2).unwrap(), "originalGrant");
        });
    }
    #[test]
    fn holder_journal_unknown_is_a_terminal_fence_and_snapshot_validation_is_bounded() {
        fixture(|db, input| {
            let proof = gone(&input);
            for malformed in ["", "0", "AA", "zz"] {
                let mut invalid = input.clone(); invalid.snapshot_hex = malformed.into();
                assert!(begin_holder_disappearance(db, &invalid, &proof).is_err());
            }
            let mut invalid = input.clone(); invalid.snapshot_digest = "0".repeat(64);
            assert!(begin_holder_disappearance(db, &invalid, &proof).is_err());
            invalid = input.clone(); invalid.snapshot_hex = "00".repeat(MAX_CAPTURE_BYTES + 1);
            assert!(begin_holder_disappearance(db, &invalid, &proof).is_err());
            invalid = input.clone(); invalid.pid = "04294967295".into();
            assert!(begin_holder_disappearance(db, &invalid, &proof).is_err());
            let first = begin_holder_disappearance(db, &input, &proof).unwrap();
            let unknown = transaction(db, |db| advance_in_transaction(db, &first,
                HolderDisappearancePhase::Preparing, HolderDisappearancePhase::Unknown, &proof)).unwrap();
            assert!(matches!(begin_holder_disappearance(db, &input, &proof), Err(HolderDisappearanceError::Unknown)));
            assert!(matches!(transaction(db, |db| advance_in_transaction(db, &unknown,
                HolderDisappearancePhase::Unknown, HolderDisappearancePhase::Applied, &proof)), Err(HolderDisappearanceError::Unknown)));
            let mut replaced = input.clone(); replaced.request_id = "newGoneRequest".into();
            assert!(matches!(begin_holder_disappearance(db, &replaced, &proof), Err(HolderDisappearanceError::Conflict)));
            assert_eq!(transaction(db, |db| advance_in_transaction(db, &unknown,
                HolderDisappearancePhase::Unknown, HolderDisappearancePhase::Unknown, &proof)).unwrap(), unknown);
            assert_eq!(read_holder_disappearance(db, "bindingA").unwrap().unwrap(), unknown);
        });
    }
    #[test]
    fn holder_journal_rejects_schema_shadow_trigger_index_and_mismatched_association() {
        fixture(|db, input| {
            let proof = gone(&input);
            for (change, restore) in [
                ("UPDATE main.gogoke_v37_instance_history_generations SET ticket='changed' WHERE binding_id='bindingA'",
                 "UPDATE main.gogoke_v37_instance_history_generations SET ticket='ticketA' WHERE binding_id='bindingA'"),
                ("UPDATE main.gogoke_v37_credential_profiles SET generation='2' WHERE binding_id='bindingA'",
                 "UPDATE main.gogoke_v37_credential_profiles SET generation='1' WHERE binding_id='bindingA'"),
                ("UPDATE main.gogoke_v37_credential_aliases SET source_file_identity='changed' WHERE history_id='historyA'",
                 "UPDATE main.gogoke_v37_credential_aliases SET source_file_identity='sourceIdentity' WHERE history_id='historyA'"),
                ("UPDATE main.gogoke_coordination_process_custody SET creation_time_100ns='2' WHERE operation_id='processA'",
                 "UPDATE main.gogoke_coordination_process_custody SET creation_time_100ns='1' WHERE operation_id='processA'"),
            ] {
                db.execute(change).unwrap();
                assert!(matches!(begin_holder_disappearance(db, &input, &proof), Err(HolderDisappearanceError::Conflict)));
                assert!(read_holder_disappearance(db, "bindingA").unwrap().is_none());
                db.execute(restore).unwrap();
            }
            for sql in ["CREATE TEMP TABLE gogoke_v37_holder_disappearance(value TEXT)",
                "CREATE TRIGGER bad_holder AFTER INSERT ON gogoke_v37_holder_disappearance BEGIN SELECT 1; END",
                "CREATE INDEX bad_holder ON gogoke_v37_holder_disappearance(phase)"] {
                db.execute(sql).unwrap();
                assert!(matches!(initialize_holder_disappearance_schema(db), Err(HolderDisappearanceError::Schema)));
                assert!(matches!(read_holder_disappearance(db, "bindingA"), Err(HolderDisappearanceError::Schema)));
                assert!(matches!(begin_holder_disappearance(db, &input, &proof), Err(HolderDisappearanceError::Schema)));
                let drop_sql = if sql.starts_with("CREATE TEMP") { "DROP TABLE temp.gogoke_v37_holder_disappearance" }
                    else if sql.starts_with("CREATE TRIGGER") { "DROP TRIGGER bad_holder" } else { "DROP INDEX bad_holder" };
                db.execute(drop_sql).unwrap();
            }
            db.execute("DROP TABLE main.gogoke_v37_holder_disappearance").unwrap();
            db.execute("CREATE TABLE gogoke_v37_holder_disappearance(binding_id TEXT PRIMARY KEY)").unwrap();
            assert!(matches!(initialize_holder_disappearance_schema(db), Err(HolderDisappearanceError::Schema)));
        });
    }
    #[test]
    fn holder_journal_cannot_use_an_unobserved_proof_for_the_exact_live_process() {
        #[repr(C)] struct FileTime { low: u32, high: u32 }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
            fn GetProcessTimes(process: *mut std::ffi::c_void, creation: *mut FileTime,
                exit: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        }
        let mut created = FileTime { low: 0, high: 0 }; let mut exited = FileTime { low: 0, high: 0 };
        let mut kernel = FileTime { low: 0, high: 0 }; let mut user = FileTime { low: 0, high: 0 };
        assert_ne!(unsafe { GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) }, 0);
        let pid = std::process::id(); let creation = (u64::from(created.high) << 32) | u64::from(created.low);
        fixture(|db, mut input| {
            input.pid = pid.to_string(); input.creation_time_100ns = creation.to_string();
            let update = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_coordination_process_custody SET pid=?1,creation_time_100ns=?2 WHERE operation_id='processA'").unwrap();
            bind(&update, &[&input.pid, &input.creation_time_100ns]).unwrap(); update.step_done().unwrap(); drop(update);
            let proof = NativeProcessHoldersGone::for_test(&[(pid, creation)]);
            assert!(matches!(begin_holder_disappearance(db, &input, &proof),
                Err(HolderDisappearanceError::NativeHolders(NativeLegacyHoldersGoneError::ExactHolderAlive { .. }))));
            assert!(read_holder_disappearance(db, "bindingA").unwrap().is_none());
        });
    }
}
