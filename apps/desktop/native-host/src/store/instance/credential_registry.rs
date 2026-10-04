//! Credential metadata and namespace/ACL intent only. This module never opens
//! credential files. Root/H supplies exact metadata receipts from held objects
//! and retains current A/H authority, startup-selector and quiescence proofs.

use super::private_history::read_private_history_generation;
use crate::root::RootIdentity;
use crate::store::atomic::{AtomicError, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};

const SCHEMA: [(&str, &str); 4] = [
    ("gogoke_v37_credential_backend_evidence", "CREATE TABLE gogoke_v37_credential_backend_evidence(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),home_identity TEXT NOT NULL,program_digest TEXT NOT NULL,version TEXT NOT NULL,backend TEXT NOT NULL CHECK(backend IN ('FILE','OTHER','UNKNOWN')),startup_selector TEXT NOT NULL CHECK(startup_selector IN ('FILE_BOUND','UNKNOWN')),request_id TEXT NOT NULL,operation_id TEXT NOT NULL,ticket TEXT NOT NULL,custodian_nonce TEXT NOT NULL,generation TEXT NOT NULL) STRICT"),
    ("gogoke_v37_credential_objects", "CREATE TABLE gogoke_v37_credential_objects(instance_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instances(instance_id),root_identity TEXT NOT NULL,home_identity TEXT NOT NULL,file_identity TEXT NOT NULL UNIQUE,source_parent_identity TEXT NOT NULL,phase TEXT NOT NULL CHECK(phase IN ('ACTIVE','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision >= 1)) STRICT"),
    ("gogoke_v37_credential_aliases", "CREATE TABLE gogoke_v37_credential_aliases(history_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instance_histories(history_id),instance_id TEXT NOT NULL REFERENCES gogoke_v37_credential_objects(instance_id),directory_identity TEXT NOT NULL,source_file_identity TEXT NOT NULL,intent_request TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('PREPARING','ACTIVE','DORMANT','REMOVE_PENDING','REMOVED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision >= 1)) STRICT"),
    ("gogoke_v37_credential_profiles", "CREATE TABLE gogoke_v37_credential_profiles(binding_id TEXT PRIMARY KEY REFERENCES gogoke_v37_instance_history_generations(binding_id),instance_id TEXT NOT NULL REFERENCES gogoke_v37_credential_objects(instance_id),history_id TEXT NOT NULL REFERENCES gogoke_v37_credential_aliases(history_id),generation TEXT NOT NULL,profile_sid TEXT NOT NULL UNIQUE,source_file_identity TEXT NOT NULL,intent_request TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('GRANT_PENDING','ACTIVE','REVOKE_PENDING','REVOKED','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision >= 1)) STRICT"),
];

#[derive(Debug)]
pub(crate) enum CredentialRegistryError {
    Invalid(&'static str), Conflict, Unusable, Unknown, Schema,
    Atomic(AtomicError), Sqlite(SameOpenError), History(super::private_history::PrivateHistoryError),
    IdentityParse(std::num::ParseIntError), CommitUnknown(SameOpenError),
    RollbackUnknown { original: Box<CredentialRegistryError>, rollback: SameOpenError },
}
impl From<AtomicError> for CredentialRegistryError { fn from(value: AtomicError) -> Self { Self::Atomic(value) } }
impl From<SameOpenError> for CredentialRegistryError { fn from(value: SameOpenError) -> Self { Self::Sqlite(value) } }
impl From<super::private_history::PrivateHistoryError> for CredentialRegistryError {
    fn from(value: super::private_history::PrivateHistoryError) -> Self { Self::History(value) }
}
type Result<T> = std::result::Result<T, CredentialRegistryError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialBackend { File, Other, Unknown }
impl CredentialBackend {
    fn text(self) -> &'static str { match self { Self::File => "FILE", Self::Other => "OTHER", Self::Unknown => "UNKNOWN" } }
    fn from_text(value: &str) -> Result<Self> {
        match value { "FILE" => Ok(Self::File), "OTHER" => Ok(Self::Other), "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(CredentialRegistryError::Unusable) }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialStartupSelector { FileBound, Unknown }
impl CredentialStartupSelector {
    fn text(self) -> &'static str { match self { Self::FileBound => "FILE_BOUND", Self::Unknown => "UNKNOWN" } }
    fn from_text(value: &str) -> Result<Self> {
        match value { "FILE_BOUND" => Ok(Self::FileBound), "UNKNOWN" => Ok(Self::Unknown),
            _ => Err(CredentialRegistryError::Unusable) }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendSource {
    pub(crate) request_id: String, pub(crate) instance_id: String,
    pub(crate) home_identity: RootIdentity, pub(crate) program_digest: String, pub(crate) version: String,
    pub(crate) backend: CredentialBackend, pub(crate) startup_selector: CredentialStartupSelector,
    pub(crate) operation_id: String, pub(crate) ticket: String,
    pub(crate) nonce: String, pub(crate) generation: String,
}
pub(crate) struct CredentialObjectInput {
    pub(crate) request_id: String, pub(crate) instance_id: String,
    pub(crate) root_identity: RootIdentity, pub(crate) home_identity: RootIdentity,
    pub(crate) file_identity: RootIdentity, pub(crate) source_parent_identity: RootIdentity,
    pub(crate) observed_nlink: u64, pub(crate) expected_revision: i64,
    pub(crate) rebind_from: Option<RootIdentity>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialObjectRecord {
    pub(crate) instance_id: String, pub(crate) root_identity: RootIdentity,
    pub(crate) home_identity: RootIdentity, pub(crate) file_identity: RootIdentity,
    pub(crate) source_parent_identity: RootIdentity, pub(crate) revision: i64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialIntentDisposition { New, Pending, Applied, Unknown }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialAliasAction { Create, Reactivate, Dormant, Remove }
impl CredentialAliasAction {
    fn text(self) -> &'static str { match self { Self::Create => "CREATE", Self::Reactivate => "REACTIVATE", Self::Dormant => "DORMANT", Self::Remove => "REMOVE" } }
    fn pending(self) -> &'static str { if self == Self::Remove { "REMOVE_PENDING" } else { "PREPARING" } }
    fn complete(self) -> &'static str { match self { Self::Create | Self::Reactivate => "ACTIVE", Self::Dormant => "DORMANT", Self::Remove => "REMOVED" } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialAliasResult { Active, Dormant, Removed, Unknown }
impl CredentialAliasResult {
    fn text(self) -> &'static str { match self { Self::Active => "ACTIVE", Self::Dormant => "DORMANT", Self::Removed => "REMOVED", Self::Unknown => "UNKNOWN" } }
}
pub(crate) struct CredentialAliasIntent {
    pub(crate) request_id: String, pub(crate) instance_id: String, pub(crate) history_id: String,
    pub(crate) directory_identity: RootIdentity, pub(crate) source_file_identity: RootIdentity,
    pub(crate) expected_revision: i64, pub(crate) action: CredentialAliasAction,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialAliasRecord {
    pub(crate) instance_id: String, pub(crate) history_id: String,
    pub(crate) directory_identity: RootIdentity, pub(crate) source_file_identity: RootIdentity,
    pub(crate) intent_request: String, pub(crate) state: String, pub(crate) revision: i64,
}
#[derive(Clone, Debug)]
pub(crate) struct CredentialAliasIntentReceipt {
    pub(crate) disposition: CredentialIntentDisposition, pub(crate) alias: CredentialAliasRecord,
    pub(crate) action: CredentialAliasAction,
    key: String, fingerprint: String, intent_revision: i64,
}
pub(crate) struct CredentialAliasPhysicalReceipt {
    pub(crate) source_file_identity: RootIdentity, pub(crate) directory_identity: RootIdentity,
    pub(crate) observed_nlink: u64, pub(crate) result: CredentialAliasResult,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialProfileAction { Grant, Revoke }
impl CredentialProfileAction {
    fn text(self) -> &'static str { match self { Self::Grant => "GRANT", Self::Revoke => "REVOKE" } }
    fn pending(self) -> &'static str { match self { Self::Grant => "GRANT_PENDING", Self::Revoke => "REVOKE_PENDING" } }
    fn complete(self) -> &'static str { match self { Self::Grant => "ACTIVE", Self::Revoke => "REVOKED" } }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CredentialProfileResult { Active, Revoked, Unknown }
impl CredentialProfileResult {
    fn text(self) -> &'static str { match self { Self::Active => "ACTIVE", Self::Revoked => "REVOKED", Self::Unknown => "UNKNOWN" } }
}
pub(crate) struct CredentialProfileIntent {
    pub(crate) request_id: String, pub(crate) instance_id: String, pub(crate) history_id: String,
    pub(crate) binding_id: String, pub(crate) generation: String, pub(crate) profile_sid: String,
    pub(crate) source_file_identity: RootIdentity, pub(crate) expected_revision: i64,
    pub(crate) action: CredentialProfileAction,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CredentialProfileRecord {
    pub(crate) instance_id: String, pub(crate) history_id: String, pub(crate) binding_id: String,
    pub(crate) generation: String, pub(crate) profile_sid: String,
    pub(crate) source_file_identity: RootIdentity, pub(crate) intent_request: String,
    pub(crate) state: String, pub(crate) revision: i64,
}
#[derive(Clone, Debug)]
pub(crate) struct CredentialProfileIntentReceipt {
    pub(crate) disposition: CredentialIntentDisposition, pub(crate) profile: CredentialProfileRecord,
    pub(crate) action: CredentialProfileAction,
    key: String, fingerprint: String, intent_revision: i64,
}

fn atom(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 256 || !value.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':')) {
        return Err(CredentialRegistryError::Invalid("metadata identifier"));
    }
    Ok(())
}
fn parse_identity(value: &str) -> Result<RootIdentity> {
    let Some((volume, file)) = value.strip_prefix("volume:").and_then(|value| value.split_once("/file:")) else {
        return Err(CredentialRegistryError::Invalid("identity"));
    };
    if volume.len() != 16 || file.len() != 32 || !volume.bytes().chain(file.bytes()).all(|byte| byte.is_ascii_hexdigit()) {
        return Err(CredentialRegistryError::Invalid("identity"));
    }
    let volume_serial = u64::from_str_radix(volume, 16).map_err(CredentialRegistryError::IdentityParse)?;
    let mut file_id = [0; 16];
    for (index, part) in file.as_bytes().chunks_exact(2).enumerate() {
        // All bytes have already been checked as ASCII hexadecimal.
        let text = std::str::from_utf8(part).map_err(|error| CredentialRegistryError::Atomic(
            AtomicError::DurabilityContractFailed(format!("identity encoding: {error}"))))?;
        file_id[index] = u8::from_str_radix(text, 16).map_err(CredentialRegistryError::IdentityParse)?;
    }
    let result = RootIdentity { volume_serial, file_id };
    if result.opaque() != value { return Err(CredentialRegistryError::Invalid("noncanonical identity")); }
    Ok(result)
}
fn number(value: &str) -> Result<i64> { value.parse().map_err(CredentialRegistryError::IdentityParse) }
fn framed(values: &[&str]) -> String {
    let mut bytes = Vec::new();
    for value in values { bytes.extend_from_slice(&(value.len() as u64).to_be_bytes()); bytes.extend_from_slice(value.as_bytes()); }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn bind(statement: &Statement, values: &[&str]) -> Result<()> {
    for (index, value) in values.iter().enumerate() { statement.bind_text((index + 1) as i32, value)?; }
    Ok(())
}
fn changed(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    let query = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !query.step_row()? || query.column_text(0)? != "1" { return Err(CredentialRegistryError::Conflict); }
    Ok(())
}
fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>, work: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<T>) -> Result<T> {
    db.execute("BEGIN IMMEDIATE")?;
    match work(db) {
        Ok(value) => { db.execute("COMMIT").map_err(CredentialRegistryError::CommitUnknown)?; Ok(value) },
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(CredentialRegistryError::RollbackUnknown { original: Box::new(original), rollback }),
        },
    }
}
fn observed_schema(db: &VerifiedDatabaseConnection<'_>) -> Result<Vec<(String, String)>> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,22))='gogoke_v37_credential_' ORDER BY name")?;
    let mut rows = Vec::new();
    while query.step_row()? { rows.push((query.column_text(0)?, query.column_text(1)?)); }
    Ok(rows)
}
fn expected_schema() -> Vec<(String, String)> {
    let mut entries: Vec<_> = SCHEMA.iter().map(|(name, sql)| ((*name).to_owned(), (*sql).to_owned())).collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0)); entries
}
fn reject_schema_side_effects(db: &VerifiedDatabaseConnection<'_>) -> Result<()> {
    for sql in [
        "SELECT 1 FROM temp.sqlite_schema WHERE lower(substr(name,1,22))='gogoke_v37_credential_' OR lower(substr(tbl_name,1,22))='gogoke_v37_credential_' LIMIT 1",
        "SELECT 1 FROM main.sqlite_schema WHERE type IN ('trigger','index') AND sql IS NOT NULL AND lower(substr(tbl_name,1,22))='gogoke_v37_credential_' LIMIT 1",
    ] { if Statement::prepare(db.as_ptr(), sql)?.step_row()? { return Err(CredentialRegistryError::Schema); } }
    Ok(())
}
/// Root calls after F's six-table initializer. This family has no unknown-
/// schema recreation or legacy migration; only exact absence or exact four.
pub(crate) fn initialize_credential_schema(db: &mut VerifiedDatabaseConnection<'_>) -> Result<()> {
    reject_schema_side_effects(db)?;
    let observed = observed_schema(db)?;
    if observed == expected_schema() { return Ok(()); }
    if !observed.is_empty() { return Err(CredentialRegistryError::Schema); }
    db.execute("BEGIN IMMEDIATE")?;
    let result = (|| {
        reject_schema_side_effects(db)?;
        if observed_schema(db)? != observed { return Err(CredentialRegistryError::Schema); }
        for (_, sql) in SCHEMA { db.execute(sql)?; }
        if observed_schema(db)? != expected_schema() { return Err(CredentialRegistryError::Schema); }
        Ok(())
    })();
    match result {
        Ok(()) => db.execute("COMMIT").map_err(CredentialRegistryError::CommitUnknown),
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(CredentialRegistryError::RollbackUnknown { original: Box::new(original), rollback }),
        },
    }
}
fn current_pin(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<(RootIdentity, String, String)> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT home_identity,program_digest,version FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id='codex'")?;
    query.bind_text(1, instance)?;
    if !query.step_row()? { return Err(CredentialRegistryError::Unusable); }
    let row = (parse_identity(&query.column_text(0)?)?, query.column_text(1)?, query.column_text(2)?);
    if query.step_row()? { return Err(CredentialRegistryError::Conflict); }
    Ok(row)
}
fn check_backend_source(db: &VerifiedDatabaseConnection<'_>, source: &BackendSource) -> Result<()> {
    for value in [&source.request_id, &source.instance_id, &source.operation_id, &source.ticket, &source.nonce, &source.generation] { atom(value)?; }
    let current = current_pin(db, &source.instance_id)?;
    if current != (source.home_identity.clone(), source.program_digest.clone(), source.version.clone()) {
        return Err(CredentialRegistryError::Unusable);
    }
    let query = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_coordination_process_custody
          WHERE operation_id=?1 AND ticket=?2 AND custodian_nonce=?3 AND generation=?4
            AND binary_digest_sha256=?5 AND state='STOPPED' AND stop_proof_hash IS NOT NULL AND stop_proof_hash<>''")?;
    bind(&query, &[&source.operation_id, &source.ticket, &source.nonce, &source.generation, &source.program_digest])?;
    if !query.step_row()? { return Err(CredentialRegistryError::Unusable); }
    if query.step_row()? { return Err(CredentialRegistryError::Conflict); }
    Ok(())
}
fn journal_target(instance: &str) -> String { format!("credential-instance-{}", sha256_hex(instance.as_bytes())) }
fn journal_key(kind: &str, instance: &str, request: &str) -> String {
    format!("credential-intent-{}", sha256_hex(framed(&[kind, instance, request]).as_bytes()))
}
fn journal(db: &VerifiedDatabaseConnection<'_>, key: &str, fingerprint: &str, target: &str) -> Result<Option<(CredentialIntentDisposition, String)>> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT request_hex,target_id,phase,COALESCE(receipt_json,'') FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    query.bind_text(1, key)?;
    if !query.step_row()? { return Ok(None); }
    if query.column_text(0)? != fingerprint || query.column_text(1)? != target { return Err(CredentialRegistryError::Conflict); }
    let phase = match query.column_text(2)?.as_str() {
        "PREPARING" => CredentialIntentDisposition::Pending, "APPLIED" => CredentialIntentDisposition::Applied,
        "UNKNOWN" => CredentialIntentDisposition::Unknown, _ => return Err(CredentialRegistryError::Unknown),
    };
    let result = query.column_text(3)?;
    if query.step_row()? { return Err(CredentialRegistryError::Conflict); }
    Ok(Some((phase, result)))
}
fn new_journal(db: &VerifiedDatabaseConnection<'_>, key: &str, fingerprint: &str, instance: &str) -> Result<()> {
    let statement = Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'PREPARING')")?;
    bind(&statement, &[key, fingerprint, &journal_target(instance)])?; statement.step_done()?; Ok(())
}
fn finish_journal(db: &VerifiedDatabaseConnection<'_>, key: &str, fingerprint: &str, result: &str, unknown: bool) -> Result<()> {
    let statement = Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_instance_operations SET phase=?3,receipt_json=?4,native_receipt_id=?5
          WHERE request_id=?1 AND request_hex=?2 AND phase IN ('PREPARING','UNKNOWN')")?;
    let metadata_receipt = format!("credential-receipt-{}", sha256_hex(framed(&[key, result]).as_bytes()));
    bind(&statement, &[key, fingerprint, if unknown { "UNKNOWN" } else { "APPLIED" }, result, &metadata_receipt])?;
    statement.step_done()?; changed(db)
}
pub(crate) fn record_credential_backend(db: &mut VerifiedDatabaseConnection<'_>, source: &BackendSource) -> Result<()> {
    transaction(db, |db| {
        check_backend_source(db, source)?;
        let fingerprint = framed(&["credential-backend-v1", &source.instance_id, &source.home_identity.opaque(),
            &source.program_digest, &source.version, source.backend.text(), source.startup_selector.text(),
            &source.request_id, &source.operation_id, &source.ticket, &source.nonce, &source.generation]);
        let key = journal_key("backend", &source.instance_id, &source.request_id);
        if let Some((disposition, _)) = journal(db, &key, &fingerprint, &journal_target(&source.instance_id))? {
            return if disposition == CredentialIntentDisposition::Applied { Ok(()) } else { Err(CredentialRegistryError::Unknown) };
        }
        new_journal(db, &key, &fingerprint, &source.instance_id)?;
        let statement = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_credential_backend_evidence
             (instance_id,home_identity,program_digest,version,backend,startup_selector,request_id,operation_id,ticket,custodian_nonce,generation)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
             ON CONFLICT(instance_id) DO UPDATE SET home_identity=excluded.home_identity,program_digest=excluded.program_digest,
             version=excluded.version,backend=excluded.backend,startup_selector=excluded.startup_selector,request_id=excluded.request_id,
             operation_id=excluded.operation_id,ticket=excluded.ticket,custodian_nonce=excluded.custodian_nonce,generation=excluded.generation")?;
        bind(&statement, &[&source.instance_id, &source.home_identity.opaque(), &source.program_digest, &source.version,
            source.backend.text(), source.startup_selector.text(), &source.request_id, &source.operation_id,
            &source.ticket, &source.nonce, &source.generation])?;
        statement.step_done()?;
        finish_journal(db, &key, &fingerprint, "BACKEND_METADATA_RECORDED", false)
    })
}
/// Original configured enum is evidence, never startup/use qualification.
pub(crate) fn read_configured_credential_backend(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<BackendSource> {
    atom(instance)?;
    let query = Statement::prepare(db.as_ptr(),
        "SELECT home_identity,program_digest,version,request_id,operation_id,ticket,custodian_nonce,generation,backend,startup_selector
           FROM main.gogoke_v37_credential_backend_evidence WHERE instance_id=?1")?;
    query.bind_text(1, instance)?;
    if !query.step_row()? { return Err(CredentialRegistryError::Unusable); }
    let source = BackendSource { instance_id: instance.into(), home_identity: parse_identity(&query.column_text(0)?)?,
        program_digest: query.column_text(1)?, version: query.column_text(2)?, request_id: query.column_text(3)?,
        operation_id: query.column_text(4)?, ticket: query.column_text(5)?, nonce: query.column_text(6)?, generation: query.column_text(7)?,
        backend: CredentialBackend::from_text(&query.column_text(8)?)?,
        startup_selector: CredentialStartupSelector::from_text(&query.column_text(9)?)? };
    if query.step_row()? { return Err(CredentialRegistryError::Conflict); }
    check_backend_source(db, &source)?; Ok(source)
}
pub(crate) fn read_usable_credential_backend(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<BackendSource> {
    let source = read_configured_credential_backend(db, instance)?;
    if source.backend != CredentialBackend::File || source.startup_selector != CredentialStartupSelector::FileBound {
        return Err(CredentialRegistryError::Unusable);
    }
    Ok(source)
}
pub(crate) fn read_credential_object(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<Option<CredentialObjectRecord>> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT root_identity,home_identity,file_identity,source_parent_identity,CAST(revision AS TEXT),phase
           FROM main.gogoke_v37_credential_objects WHERE instance_id=?1")?;
    query.bind_text(1, instance)?;
    if !query.step_row()? { return Ok(None); }
    if query.column_text(5)? != "ACTIVE" { return Err(CredentialRegistryError::Unknown); }
    let row = CredentialObjectRecord { instance_id: instance.into(), root_identity: parse_identity(&query.column_text(0)?)?,
        home_identity: parse_identity(&query.column_text(1)?)?, file_identity: parse_identity(&query.column_text(2)?)?,
        source_parent_identity: parse_identity(&query.column_text(3)?)?, revision: number(&query.column_text(4)?)? };
    if query.step_row()? { return Err(CredentialRegistryError::Conflict); }
    if &row.root_identity != db.root_identity() || row.source_parent_identity != row.home_identity {
        return Err(CredentialRegistryError::Conflict);
    }
    Ok(Some(row))
}
fn all_quiescent_metadata(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<()> {
    let target = journal_target(instance);
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 AND state<>'REMOVED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 AND state<>'REVOKED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN') LIMIT 1",
    ] {
        let query = Statement::prepare(db.as_ptr(), sql)?;
        query.bind_text(1, if sql.contains("target_id") { target.as_str() } else { instance })?;
        if query.step_row()? { return Err(CredentialRegistryError::Unknown); }
    }
    Ok(())
}
pub(crate) fn bind_credential_object(db: &mut VerifiedDatabaseConnection<'_>, input: &CredentialObjectInput) -> Result<CredentialObjectRecord> {
    atom(&input.request_id)?; atom(&input.instance_id)?;
    transaction(db, |db| {
        let backend = read_usable_credential_backend(db, &input.instance_id)?;
        if input.root_identity != *db.root_identity() || input.home_identity != backend.home_identity
            || input.source_parent_identity != input.home_identity || input.observed_nlink == 0 {
            return Err(CredentialRegistryError::Conflict);
        }
        let prior = read_credential_object(db, &input.instance_id)?;
        let expected = prior.as_ref().map_or(0, |row| row.revision);
        let fingerprint = framed(&["credential-object-v1", &input.request_id, &input.instance_id,
            &input.root_identity.opaque(), &input.home_identity.opaque(), &input.file_identity.opaque(),
            &input.source_parent_identity.opaque(), &input.observed_nlink.to_string(), &input.expected_revision.to_string(),
            &input.rebind_from.as_ref().map_or(String::new(), RootIdentity::opaque)]);
        let key = journal_key("object", &input.instance_id, &input.request_id);
        if let Some((disposition, _)) = journal(db, &key, &fingerprint, &journal_target(&input.instance_id))? {
            if disposition == CredentialIntentDisposition::Applied {
                return prior.filter(|row| row.file_identity == input.file_identity).ok_or(CredentialRegistryError::Conflict);
            }
            return Err(CredentialRegistryError::Unknown);
        }
        if expected != input.expected_revision { return Err(CredentialRegistryError::Conflict); }
        if let Some(row) = &prior {
            if row.root_identity != input.root_identity || row.home_identity != input.home_identity
                || row.source_parent_identity != input.source_parent_identity { return Err(CredentialRegistryError::Conflict); }
            if row.file_identity != input.file_identity {
                if input.rebind_from.as_ref() != Some(&row.file_identity) || input.observed_nlink != 1 { return Err(CredentialRegistryError::Conflict); }
                all_quiescent_metadata(db, &input.instance_id)?;
            } else if input.rebind_from.is_some() { return Err(CredentialRegistryError::Conflict); }
        } else if input.rebind_from.is_some() || input.observed_nlink != 1 { return Err(CredentialRegistryError::Conflict); }
        new_journal(db, &key, &fingerprint, &input.instance_id)?;
        let update = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_credential_objects(instance_id,root_identity,home_identity,file_identity,source_parent_identity,phase,revision)
             VALUES(?1,?2,?3,?4,?5,'ACTIVE',?6) ON CONFLICT(instance_id) DO UPDATE SET file_identity=excluded.file_identity,revision=excluded.revision")?;
        bind(&update, &[&input.instance_id, &input.root_identity.opaque(), &input.home_identity.opaque(),
            &input.file_identity.opaque(), &input.source_parent_identity.opaque()])?;
        update.bind_i64(6, expected.checked_add(1).ok_or(CredentialRegistryError::Invalid("revision overflow"))?)?;
        update.step_done()?;
        finish_journal(db, &key, &fingerprint, "OBJECT_METADATA_RECORDED", false)?;
        read_credential_object(db, &input.instance_id)?.ok_or(CredentialRegistryError::Unknown)
    })
}
fn current_history(db: &VerifiedDatabaseConnection<'_>, instance: &str, history: &str, identity: &RootIdentity) -> Result<()> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instance_histories h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id
          WHERE h.history_id=?1 AND h.instance_id=?2 AND h.directory_identity=?3 AND h.state='READY'
            AND h.root_identity=?4 AND h.home_identity=i.home_identity")?;
    bind(&query, &[history, instance, &identity.opaque(), &db.root_identity().opaque()])?;
    if !query.step_row()? { return Err(CredentialRegistryError::Conflict); } Ok(())
}
pub(crate) fn read_credential_aliases(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<Vec<CredentialAliasRecord>> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT history_id,directory_identity,source_file_identity,intent_request,state,CAST(revision AS TEXT)
           FROM main.gogoke_v37_credential_aliases WHERE instance_id=?1 ORDER BY history_id")?;
    query.bind_text(1, instance)?;
    let mut rows = Vec::new();
    while query.step_row()? { rows.push(CredentialAliasRecord { instance_id: instance.into(), history_id: query.column_text(0)?,
        directory_identity: parse_identity(&query.column_text(1)?)?, source_file_identity: parse_identity(&query.column_text(2)?)?,
        intent_request: query.column_text(3)?, state: query.column_text(4)?, revision: number(&query.column_text(5)?)? }); }
    Ok(rows)
}
fn alias(db: &VerifiedDatabaseConnection<'_>, instance: &str, history: &str) -> Result<Option<CredentialAliasRecord>> {
    Ok(read_credential_aliases(db, instance)?.into_iter().find(|row| row.history_id == history))
}
fn check_alias_source(db: &VerifiedDatabaseConnection<'_>, row: &CredentialAliasRecord, use_object: bool) -> Result<()> {
    if use_object {
        read_usable_credential_backend(db, &row.instance_id)?;
        current_history(db, &row.instance_id, &row.history_id, &row.directory_identity)?;
    }
    let object = read_credential_object(db, &row.instance_id)?.ok_or(CredentialRegistryError::Unknown)?;
    if object.file_identity != row.source_file_identity { return Err(CredentialRegistryError::Conflict); } Ok(())
}
fn no_live_profiles(db: &VerifiedDatabaseConnection<'_>, history: &str) -> Result<()> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_credential_profiles WHERE history_id=?1 AND state<>'REVOKED' LIMIT 1")?;
    query.bind_text(1, history)?;
    if query.step_row()? { return Err(CredentialRegistryError::Unknown); } Ok(())
}
pub(crate) fn begin_credential_alias(db: &mut VerifiedDatabaseConnection<'_>, input: &CredentialAliasIntent) -> Result<CredentialAliasIntentReceipt> {
    atom(&input.request_id)?; atom(&input.instance_id)?; atom(&input.history_id)?;
    transaction(db, |db| {
        let revision = input.expected_revision.checked_add(1).ok_or(CredentialRegistryError::Invalid("revision overflow"))?;
        let fingerprint = framed(&["credential-alias-v1", &input.instance_id, &input.history_id, &input.request_id,
            &input.directory_identity.opaque(), &input.source_file_identity.opaque(), input.action.text(), &input.expected_revision.to_string()]);
        let key = journal_key("alias", &input.instance_id, &input.request_id);
        let prior = alias(db, &input.instance_id, &input.history_id)?;
        let row = CredentialAliasRecord { instance_id: input.instance_id.clone(), history_id: input.history_id.clone(),
            directory_identity: input.directory_identity.clone(), source_file_identity: input.source_file_identity.clone(),
            intent_request: key.clone(), state: input.action.pending().into(), revision };
        check_alias_source(db, &row, matches!(input.action, CredentialAliasAction::Create | CredentialAliasAction::Reactivate))?;
        if let Some((disposition, _)) = journal(db, &key, &fingerprint, &journal_target(&input.instance_id))? {
            let stored = prior.ok_or(CredentialRegistryError::Unknown)?;
            if stored.intent_request != key || stored.directory_identity != row.directory_identity
                || stored.source_file_identity != row.source_file_identity { return Err(CredentialRegistryError::Conflict); }
            return Ok(CredentialAliasIntentReceipt { disposition, alias: stored, action: input.action, key, fingerprint, intent_revision: revision });
        }
        if input.expected_revision < 0 || prior.as_ref().map_or(0, |row| row.revision) != input.expected_revision {
            return Err(CredentialRegistryError::Conflict);
        }
        let state = prior.as_ref().map(|row| row.state.as_str());
        let legal = match input.action {
            CredentialAliasAction::Create => matches!(state, None | Some("REMOVED")),
            CredentialAliasAction::Reactivate => state == Some("DORMANT"),
            CredentialAliasAction::Dormant => state == Some("ACTIVE"),
            CredentialAliasAction::Remove => state == Some("DORMANT"),
        };
        if !legal { return Err(CredentialRegistryError::Unknown); }
        if let Some(previous) = &prior {
            if previous.directory_identity != row.directory_identity
                || (previous.source_file_identity != row.source_file_identity && previous.state != "REMOVED") {
                return Err(CredentialRegistryError::Conflict);
            }
        }
        if matches!(input.action, CredentialAliasAction::Dormant | CredentialAliasAction::Remove) { no_live_profiles(db, &input.history_id)?; }
        new_journal(db, &key, &fingerprint, &input.instance_id)?;
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_credential_aliases(history_id,instance_id,directory_identity,source_file_identity,intent_request,state,revision)
             VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(history_id) DO UPDATE SET source_file_identity=excluded.source_file_identity,
             intent_request=excluded.intent_request,state=excluded.state,revision=excluded.revision")?;
        bind(&write, &[&row.history_id, &row.instance_id, &row.directory_identity.opaque(), &row.source_file_identity.opaque(), &key, &row.state])?;
        write.bind_i64(7, revision)?; write.step_done()?;
        Ok(CredentialAliasIntentReceipt { disposition: CredentialIntentDisposition::New, alias: row,
            action: input.action, key, fingerprint, intent_revision: revision })
    })
}
pub(crate) fn complete_credential_alias(db: &mut VerifiedDatabaseConnection<'_>, receipt: &CredentialAliasIntentReceipt,
    physical: &CredentialAliasPhysicalReceipt) -> Result<CredentialAliasRecord> {
    transaction(db, |db| {
        let (disposition, old_result) = journal(db, &receipt.key, &receipt.fingerprint, &journal_target(&receipt.alias.instance_id))?
            .ok_or(CredentialRegistryError::Unknown)?;
        let row = alias(db, &receipt.alias.instance_id, &receipt.alias.history_id)?.ok_or(CredentialRegistryError::Unknown)?;
        check_alias_source(db, &row, false)?;
        if row.intent_request != receipt.key || row.directory_identity != receipt.alias.directory_identity
            || row.source_file_identity != receipt.alias.source_file_identity
            || physical.source_file_identity != row.source_file_identity || physical.directory_identity != row.directory_identity {
            return Err(CredentialRegistryError::Conflict);
        }
        let result = framed(&[physical.result.text(), &physical.source_file_identity.opaque(),
            &physical.directory_identity.opaque(), &physical.observed_nlink.to_string()]);
        if disposition == CredentialIntentDisposition::Applied {
            return if old_result == result && row.state == physical.result.text() { Ok(row) } else { Err(CredentialRegistryError::Conflict) };
        }
        if !matches!(row.state.as_str(), "PREPARING" | "REMOVE_PENDING" | "UNKNOWN")
            || row.revision != receipt.intent_revision { return Err(CredentialRegistryError::Conflict); }
        let unknown = physical.result == CredentialAliasResult::Unknown;
        if !unknown {
            if physical.result.text() != receipt.action.complete() { return Err(CredentialRegistryError::Conflict); }
            let mut expected = 1_u64;
            for registered in read_credential_aliases(db, &row.instance_id)? {
                if registered.history_id != row.history_id && registered.state == "UNKNOWN" { return Err(CredentialRegistryError::Unknown); }
                if registered.state != "REMOVED" && !(registered.history_id == row.history_id && physical.result == CredentialAliasResult::Removed) {
                    if registered.source_file_identity != row.source_file_identity { return Err(CredentialRegistryError::Conflict); }
                    expected = expected.checked_add(1).ok_or(CredentialRegistryError::Invalid("link count overflow"))?;
                }
            }
            if physical.observed_nlink != expected { return Err(CredentialRegistryError::Conflict); }
            if matches!(physical.result, CredentialAliasResult::Dormant | CredentialAliasResult::Removed) { no_live_profiles(db, &row.history_id)?; }
        }
        let write = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_credential_aliases SET state=?3,revision=?4 WHERE history_id=?1 AND intent_request=?2 AND revision=?5")?;
        bind(&write, &[&row.history_id, &receipt.key, physical.result.text()])?;
        let next_revision = if unknown { row.revision } else { row.revision.checked_add(1).ok_or(CredentialRegistryError::Invalid("revision overflow"))? };
        write.bind_i64(4, next_revision)?; write.bind_i64(5, row.revision)?; write.step_done()?; changed(db)?;
        finish_journal(db, &receipt.key, &receipt.fingerprint, &result, unknown)?;
        alias(db, &row.instance_id, &row.history_id)?.ok_or(CredentialRegistryError::Unknown)
    })
}
pub(crate) fn read_credential_profiles(db: &VerifiedDatabaseConnection<'_>, instance: &str) -> Result<Vec<CredentialProfileRecord>> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT binding_id,history_id,generation,profile_sid,source_file_identity,intent_request,state,CAST(revision AS TEXT)
           FROM main.gogoke_v37_credential_profiles WHERE instance_id=?1 ORDER BY binding_id")?;
    query.bind_text(1, instance)?;
    let mut rows = Vec::new();
    while query.step_row()? { rows.push(CredentialProfileRecord { instance_id: instance.into(), binding_id: query.column_text(0)?,
        history_id: query.column_text(1)?, generation: query.column_text(2)?, profile_sid: query.column_text(3)?,
        source_file_identity: parse_identity(&query.column_text(4)?)?, intent_request: query.column_text(5)?,
        state: query.column_text(6)?, revision: number(&query.column_text(7)?)? }); } Ok(rows)
}
fn profile(db: &VerifiedDatabaseConnection<'_>, instance: &str, binding: &str) -> Result<Option<CredentialProfileRecord>> {
    Ok(read_credential_profiles(db, instance)?.into_iter().find(|row| row.binding_id == binding))
}
fn check_profile_source(db: &VerifiedDatabaseConnection<'_>, row: &CredentialProfileRecord, grant: bool) -> Result<()> {
    let registered = alias(db, &row.instance_id, &row.history_id)?.ok_or(CredentialRegistryError::Unknown)?;
    check_alias_source(db, &registered, grant)?;
    if registered.source_file_identity != row.source_file_identity
        || (grant && !matches!(registered.state.as_str(), "ACTIVE" | "DORMANT")) { return Err(CredentialRegistryError::Conflict); }
    let generation = read_private_history_generation(db, &row.binding_id, &row.generation)?.ok_or(CredentialRegistryError::Unknown)?;
    if generation.instance_id != row.instance_id || generation.history_id != row.history_id { return Err(CredentialRegistryError::Conflict); }
    Ok(())
}
pub(crate) fn begin_credential_profile(db: &mut VerifiedDatabaseConnection<'_>, input: &CredentialProfileIntent) -> Result<CredentialProfileIntentReceipt> {
    for value in [&input.request_id, &input.instance_id, &input.history_id, &input.binding_id, &input.generation, &input.profile_sid] { atom(value)?; }
    transaction(db, |db| {
        let revision = input.expected_revision.checked_add(1).ok_or(CredentialRegistryError::Invalid("revision overflow"))?;
        let fingerprint = framed(&["credential-profile-v1", &input.instance_id, &input.history_id, &input.binding_id,
            &input.generation, &input.profile_sid, &input.source_file_identity.opaque(), &input.request_id,
            input.action.text(), &input.expected_revision.to_string()]);
        let key = journal_key("profile", &input.instance_id, &input.request_id);
        let row = CredentialProfileRecord { instance_id: input.instance_id.clone(), history_id: input.history_id.clone(),
            binding_id: input.binding_id.clone(), generation: input.generation.clone(), profile_sid: input.profile_sid.clone(),
            source_file_identity: input.source_file_identity.clone(), intent_request: key.clone(), state: input.action.pending().into(), revision };
        check_profile_source(db, &row, input.action == CredentialProfileAction::Grant)?;
        let prior = profile(db, &input.instance_id, &input.binding_id)?;
        if let Some((disposition, _)) = journal(db, &key, &fingerprint, &journal_target(&input.instance_id))? {
            let stored = prior.ok_or(CredentialRegistryError::Unknown)?;
            if stored.intent_request != key || stored.history_id != row.history_id || stored.generation != row.generation
                || stored.profile_sid != row.profile_sid || stored.source_file_identity != row.source_file_identity {
                return Err(CredentialRegistryError::Conflict);
            }
            return Ok(CredentialProfileIntentReceipt { disposition, profile: stored, action: input.action, key, fingerprint, intent_revision: revision });
        }
        if input.expected_revision < 0 || prior.as_ref().map_or(0, |row| row.revision) != input.expected_revision { return Err(CredentialRegistryError::Conflict); }
        let state = prior.as_ref().map(|row| row.state.as_str());
        let legal = match input.action { CredentialProfileAction::Grant => matches!(state, None | Some("REVOKED")), CredentialProfileAction::Revoke => state == Some("ACTIVE") };
        if !legal { return Err(CredentialRegistryError::Unknown); }
        if let Some(previous) = &prior {
            if previous.history_id != row.history_id || previous.generation != row.generation
                || previous.profile_sid != row.profile_sid || previous.source_file_identity != row.source_file_identity {
                return Err(CredentialRegistryError::Conflict);
            }
        }
        new_journal(db, &key, &fingerprint, &input.instance_id)?;
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_credential_profiles(binding_id,instance_id,history_id,generation,profile_sid,source_file_identity,intent_request,state,revision)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(binding_id) DO UPDATE SET intent_request=excluded.intent_request,state=excluded.state,revision=excluded.revision")?;
        bind(&write, &[&row.binding_id, &row.instance_id, &row.history_id, &row.generation, &row.profile_sid,
            &row.source_file_identity.opaque(), &key, &row.state])?;
        write.bind_i64(9, revision)?; write.step_done()?;
        Ok(CredentialProfileIntentReceipt { disposition: CredentialIntentDisposition::New, profile: row,
            action: input.action, key, fingerprint, intent_revision: revision })
    })
}
pub(crate) fn complete_credential_profile(db: &mut VerifiedDatabaseConnection<'_>, receipt: &CredentialProfileIntentReceipt,
    result: CredentialProfileResult) -> Result<CredentialProfileRecord> {
    transaction(db, |db| {
        let (disposition, old_result) = journal(db, &receipt.key, &receipt.fingerprint, &journal_target(&receipt.profile.instance_id))?
            .ok_or(CredentialRegistryError::Unknown)?;
        let row = profile(db, &receipt.profile.instance_id, &receipt.profile.binding_id)?.ok_or(CredentialRegistryError::Unknown)?;
        check_profile_source(db, &row, false)?;
        if row.intent_request != receipt.key || row.history_id != receipt.profile.history_id || row.generation != receipt.profile.generation
            || row.profile_sid != receipt.profile.profile_sid || row.source_file_identity != receipt.profile.source_file_identity {
            return Err(CredentialRegistryError::Conflict);
        }
        if disposition == CredentialIntentDisposition::Applied {
            return if old_result == result.text() && row.state == result.text() { Ok(row) } else { Err(CredentialRegistryError::Conflict) };
        }
        if !matches!(row.state.as_str(), "GRANT_PENDING" | "REVOKE_PENDING" | "UNKNOWN") || row.revision != receipt.intent_revision {
            return Err(CredentialRegistryError::Conflict);
        }
        let unknown = result == CredentialProfileResult::Unknown;
        if !unknown && result.text() != receipt.action.complete() { return Err(CredentialRegistryError::Conflict); }
        let write = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_credential_profiles SET state=?3,revision=?4 WHERE binding_id=?1 AND intent_request=?2 AND revision=?5")?;
        bind(&write, &[&row.binding_id, &receipt.key, result.text()])?;
        write.bind_i64(4, if unknown { row.revision } else { row.revision.checked_add(1).ok_or(CredentialRegistryError::Invalid("revision overflow"))? })?;
        write.bind_i64(5, row.revision)?; write.step_done()?; changed(db)?;
        finish_journal(db, &receipt.key, &receipt.fingerprint, result.text(), unknown)?;
        profile(db, &row.instance_id, &row.binding_id)?.ok_or(CredentialRegistryError::Unknown)
    })
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    // Only the owned test database is opened. No auth file/path is created,
    // opened, copied or hashed; metadata receipts are caller witness inputs.
    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-credential-registry-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        let instance = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instances VALUES('instanceA','codex','homeA',?1,'sha256:fixture','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
        instance.bind_text(1, &identity(1).opaque()).unwrap(); instance.step_done().unwrap();
        db.execute("INSERT INTO main.gogoke_coordination_process_custody VALUES('accountRead','ticketA','nonceA','1','1','fixture-image','sha256:fixture','ordinary','domainA','1','STOPPED','fixture-stop-proof')").unwrap();
        let history = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_histories VALUES('historyA','instanceA',?1,?2,?2,'domainA','sessionA','seatA','1','bindingA','1','openA','history-historyA',?3,'READY',1)").unwrap();
        bind(&history, &[&db.root_identity().opaque(), &identity(1).opaque(), &identity(2).opaque()]).unwrap(); history.step_done().unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_history_generations(binding_id,history_id,generation,request_id) VALUES('bindingA','historyA','1','openA')").unwrap();
        drop(history); drop(instance);
        run(&mut db);
        db.close_checked().unwrap(); drop(root); fs::remove_dir_all(path).unwrap();
    }
    fn identity(byte: u8) -> RootIdentity { RootIdentity { volume_serial: 1, file_id: [byte; 16] } }
    fn backend() -> BackendSource {
        BackendSource { request_id: "backendA".into(), instance_id: "instanceA".into(), home_identity: identity(1),
            program_digest: "sha256:fixture".into(), version: "0.160.0".into(), backend: CredentialBackend::File,
            startup_selector: CredentialStartupSelector::FileBound, operation_id: "accountRead".into(),
            ticket: "ticketA".into(), nonce: "nonceA".into(), generation: "1".into() }
    }
    fn object(db: &VerifiedDatabaseConnection<'_>) -> CredentialObjectInput {
        CredentialObjectInput { request_id: "objectA".into(), instance_id: "instanceA".into(),
            root_identity: db.root_identity().clone(), home_identity: identity(1), file_identity: identity(3),
            source_parent_identity: identity(1), observed_nlink: 1, expected_revision: 0, rebind_from: None }
    }
    fn ready_object(db: &mut VerifiedDatabaseConnection<'_>) {
        initialize_credential_schema(db).unwrap(); record_credential_backend(db, &backend()).unwrap();
        let input = object(db);
        bind_credential_object(db, &input).unwrap();
    }
    fn alias_intent() -> CredentialAliasIntent {
        CredentialAliasIntent { request_id: "aliasA".into(), instance_id: "instanceA".into(), history_id: "historyA".into(),
            directory_identity: identity(2), source_file_identity: identity(3), expected_revision: 0, action: CredentialAliasAction::Create }
    }
    fn physical(result: CredentialAliasResult, links: u64) -> CredentialAliasPhysicalReceipt {
        CredentialAliasPhysicalReceipt { source_file_identity: identity(3), directory_identity: identity(2), observed_nlink: links, result }
    }
    #[test]
    fn credential_backend_requires_startup_selector_current_pin_and_original_stopped_custody() {
        fixture(|db| {
            initialize_credential_schema(db).unwrap();
            let config_only = BackendSource { startup_selector: CredentialStartupSelector::Unknown, ..backend() };
            record_credential_backend(db, &config_only).unwrap();
            assert_eq!(read_configured_credential_backend(db, "instanceA").unwrap(), config_only);
            assert!(matches!(read_usable_credential_backend(db, "instanceA"), Err(CredentialRegistryError::Unusable)));
            let actual = BackendSource { request_id: "backendB".into(), ..backend() };
            record_credential_backend(db, &actual).unwrap();
            assert_eq!(read_usable_credential_backend(db, "instanceA").unwrap(), actual);
            let unregistered_links = CredentialObjectInput { observed_nlink: 2, ..object(db) };
            assert!(matches!(bind_credential_object(db, &unregistered_links), Err(CredentialRegistryError::Conflict)));
            assert!(read_credential_object(db, "instanceA").unwrap().is_none());
            db.execute("UPDATE main.gogoke_coordination_process_custody SET state='UNKNOWN'").unwrap();
            assert!(matches!(read_usable_credential_backend(db, "instanceA"), Err(CredentialRegistryError::Unusable)));
            db.execute("UPDATE main.gogoke_coordination_process_custody SET state='STOPPED',custodian_nonce='other'").unwrap();
            assert!(matches!(read_usable_credential_backend(db, "instanceA"), Err(CredentialRegistryError::Unusable)));
            db.execute("UPDATE main.gogoke_coordination_process_custody SET custodian_nonce='nonceA'").unwrap();
            db.execute("UPDATE main.gogoke_v37_instances SET version='changed'").unwrap();
            assert!(matches!(read_usable_credential_backend(db, "instanceA"), Err(CredentialRegistryError::Unusable)));
        });
    }
    #[test]
    fn credential_alias_unknown_intent_preserves_identity_and_complete_registry_link_count() {
        fixture(|db| {
            ready_object(db);
            let intent = begin_credential_alias(db, &alias_intent()).unwrap();
            assert_eq!(intent.disposition, CredentialIntentDisposition::New);
            assert_eq!(read_credential_aliases(db, "instanceA").unwrap()[0].state, "PREPARING");
            assert_eq!(begin_credential_alias(db, &alias_intent()).unwrap().disposition, CredentialIntentDisposition::Pending);
            assert!(matches!(complete_credential_alias(db, &intent, &physical(CredentialAliasResult::Active, 1)), Err(CredentialRegistryError::Conflict)));
            let wrong = CredentialAliasPhysicalReceipt { source_file_identity: identity(8), ..physical(CredentialAliasResult::Active, 2) };
            assert!(matches!(complete_credential_alias(db, &intent, &wrong), Err(CredentialRegistryError::Conflict)));
            complete_credential_alias(db, &intent, &physical(CredentialAliasResult::Unknown, 0)).unwrap();
            let existing = begin_credential_alias(db, &alias_intent()).unwrap();
            assert_eq!(existing.disposition, CredentialIntentDisposition::Unknown);
            let another = CredentialAliasIntent { request_id: "another-create".into(), expected_revision: 1, ..alias_intent() };
            assert!(matches!(begin_credential_alias(db, &another), Err(CredentialRegistryError::Unknown)));
            let original = complete_credential_alias(db, &existing, &physical(CredentialAliasResult::Active, 2)).unwrap();
            assert_eq!(original.state, "ACTIVE"); assert_eq!(original.revision, 2);
            assert_eq!(begin_credential_alias(db, &alias_intent()).unwrap().disposition, CredentialIntentDisposition::Applied);
            assert_eq!(read_credential_aliases(db, "instanceA").unwrap().len(), 1);
            let another_history = Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_histories VALUES('historyB','instanceA',?1,?2,?2,'domainB','sessionB','seatB','1','bindingB','1','openB','history-historyB',?3,'READY',1)").unwrap();
            bind(&another_history, &[&db.root_identity().opaque(), &identity(1).opaque(), &identity(4).opaque()]).unwrap();
            another_history.step_done().unwrap();
            let additional = CredentialAliasIntent { request_id: "aliasB".into(), history_id: "historyB".into(),
                directory_identity: identity(4), ..alias_intent() };
            let additional = begin_credential_alias(db, &additional).unwrap();
            let second_receipt = CredentialAliasPhysicalReceipt { directory_identity: identity(4), ..physical(CredentialAliasResult::Active, 2) };
            assert!(matches!(complete_credential_alias(db, &additional, &second_receipt), Err(CredentialRegistryError::Conflict)));
            let full_set = CredentialAliasPhysicalReceipt { observed_nlink: 3, ..second_receipt };
            complete_credential_alias(db, &additional, &full_set).unwrap();
            assert_eq!(read_credential_aliases(db, "instanceA").unwrap().len(), 2);
        });
    }
    #[test]
    fn credential_profile_original_identity_and_quiescent_source_rebind_are_enforced() {
        fixture(|db| {
            ready_object(db);
            let alias = begin_credential_alias(db, &alias_intent()).unwrap();
            complete_credential_alias(db, &alias, &physical(CredentialAliasResult::Active, 2)).unwrap();
            let input = CredentialProfileIntent { request_id: "grantA".into(), instance_id: "instanceA".into(), history_id: "historyA".into(),
                binding_id: "bindingA".into(), generation: "1".into(), profile_sid: "S-1-15-2-7".into(),
                source_file_identity: identity(3), expected_revision: 0, action: CredentialProfileAction::Grant };
            let wrong = CredentialProfileIntent { source_file_identity: identity(8), request_id: "wrongSource".into(),
                instance_id: input.instance_id.clone(), history_id: input.history_id.clone(), binding_id: input.binding_id.clone(),
                generation: input.generation.clone(), profile_sid: input.profile_sid.clone(), expected_revision: 0, action: CredentialProfileAction::Grant };
            assert!(matches!(begin_credential_profile(db, &wrong), Err(CredentialRegistryError::Conflict)));
            let grant = begin_credential_profile(db, &input).unwrap();
            complete_credential_profile(db, &grant, CredentialProfileResult::Unknown).unwrap();
            let rebind = CredentialObjectInput { request_id: "objectB".into(), file_identity: identity(8), expected_revision: 1,
                rebind_from: Some(identity(3)), ..object(db) };
            assert!(matches!(bind_credential_object(db, &rebind), Err(CredentialRegistryError::Unknown)));
            // Backend changes prohibit new grants, but cannot block exact
            // original ACL receipt reconciliation and subsequent withdrawal.
            let unknown_backend = BackendSource { request_id: "backendUnknown".into(), backend: CredentialBackend::Other,
                startup_selector: CredentialStartupSelector::Unknown, ..backend() };
            record_credential_backend(db, &unknown_backend).unwrap();
            assert_eq!(read_configured_credential_backend(db, "instanceA").unwrap().backend, CredentialBackend::Other);
            assert!(matches!(read_usable_credential_backend(db, "instanceA"), Err(CredentialRegistryError::Unusable)));
            complete_credential_profile(db, &grant, CredentialProfileResult::Active).unwrap();
            let revoke = CredentialProfileIntent { request_id: "revokeA".into(), expected_revision: 2,
                action: CredentialProfileAction::Revoke, ..input };
            let revoke = begin_credential_profile(db, &revoke).unwrap();
            complete_credential_profile(db, &revoke, CredentialProfileResult::Revoked).unwrap();
            let dormant = CredentialAliasIntent { request_id: "dormantA".into(), expected_revision: 2, action: CredentialAliasAction::Dormant, ..alias_intent() };
            let dormant = begin_credential_alias(db, &dormant).unwrap();
            complete_credential_alias(db, &dormant, &physical(CredentialAliasResult::Dormant, 2)).unwrap();
            let remove = CredentialAliasIntent { request_id: "removeA".into(), expected_revision: 4, action: CredentialAliasAction::Remove, ..alias_intent() };
            let remove = begin_credential_alias(db, &remove).unwrap();
            complete_credential_alias(db, &remove, &physical(CredentialAliasResult::Removed, 1)).unwrap();
            let actual_backend = BackendSource { request_id: "backendAgain".into(), ..backend() };
            record_credential_backend(db, &actual_backend).unwrap();
            let rebound = bind_credential_object(db, &rebind).unwrap();
            assert_eq!(rebound.file_identity, identity(8)); assert_eq!(rebound.revision, 2);
            assert_eq!(read_credential_profiles(db, "instanceA").unwrap()[0].state, "REVOKED");
        });
    }
    #[test]
    fn credential_schema_exact_family_reopen_preserves_rows_and_rejects_unknown_shadow_trigger() {
        fixture(|db| {
            ready_object(db);
            initialize_credential_schema(db).unwrap();
            assert_eq!(read_credential_object(db, "instanceA").unwrap().unwrap().file_identity, identity(3));
            db.execute("CREATE TEMP TABLE gogoke_v37_credential_aliases(value TEXT)").unwrap();
            assert!(matches!(initialize_credential_schema(db), Err(CredentialRegistryError::Schema)));
            db.execute("DROP TABLE temp.gogoke_v37_credential_aliases").unwrap();
            db.execute("CREATE TRIGGER unrelated_credential_side_effect AFTER INSERT ON gogoke_v37_credential_objects BEGIN SELECT 1; END").unwrap();
            assert!(matches!(initialize_credential_schema(db), Err(CredentialRegistryError::Schema)));
            db.execute("DROP TRIGGER unrelated_credential_side_effect").unwrap();
            db.execute("DROP TABLE main.gogoke_v37_credential_profiles").unwrap();
            assert!(matches!(initialize_credential_schema(db), Err(CredentialRegistryError::Schema)));
            assert_eq!(observed_schema(db).unwrap().len(), 3);
            assert_eq!(read_credential_object(db, "instanceA").unwrap().unwrap().file_identity, identity(3));
        });
    }
}
