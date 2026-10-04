//! F owns physical private continuation directories. H owns stopped episodes
//! and original provider responses; A owns purpose. These native-only inputs
//! must come from those current facts, never from an IPC path or native UUID.

use super::registry::{resolve_registered_codex_home, RegistryError};
use super::resolver::ResolvedDirectory;
use crate::process::DirectoryRoots;
use crate::root::{inspect_root, RootIdentity, RootLock, RootLockError};
use crate::store::atomic::{AtomicError, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};
use std::fs;
use std::io;
use std::os::windows::fs::MetadataExt;
use std::path::Path;
use std::sync::Arc;

pub(super) const HISTORY_SCHEMA: &str = "CREATE TABLE gogoke_v37_instance_histories(history_id TEXT PRIMARY KEY,instance_id TEXT NOT NULL REFERENCES gogoke_v37_instances(instance_id),root_identity TEXT NOT NULL,parent_identity TEXT NOT NULL,home_identity TEXT NOT NULL,domain_id TEXT NOT NULL,session_id TEXT NOT NULL,seat_id TEXT NOT NULL,seat_incarnation TEXT NOT NULL,initial_binding_id TEXT NOT NULL UNIQUE,initial_generation TEXT NOT NULL,initial_request_id TEXT NOT NULL,directory_ref TEXT NOT NULL UNIQUE,directory_identity TEXT UNIQUE,state TEXT NOT NULL CHECK(state IN ('PREPARING','READY','UNKNOWN')),revision INTEGER NOT NULL CHECK(revision >= 0)) STRICT";
pub(super) const GENERATIONS_SCHEMA: &str = "CREATE TABLE gogoke_v37_instance_history_generations(binding_id TEXT PRIMARY KEY,history_id TEXT NOT NULL REFERENCES gogoke_v37_instance_histories(history_id),generation TEXT NOT NULL,request_id TEXT NOT NULL,predecessor_binding_id TEXT REFERENCES gogoke_v37_instance_history_generations(binding_id),process_operation_id TEXT UNIQUE,ticket TEXT,custodian_nonce TEXT,CHECK((process_operation_id IS NULL AND ticket IS NULL AND custodian_nonce IS NULL) OR (process_operation_id IS NOT NULL AND ticket IS NOT NULL AND custodian_nonce IS NOT NULL))) STRICT";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateHistoryLaunch<'a> {
    pub(crate) instance_id: &'a str,
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) seat_id: &'a str,
    pub(crate) seat_incarnation: &'a str,
    pub(crate) binding_id: &'a str,
    pub(crate) generation: &'a str,
    pub(crate) request_id: &'a str,
}

/// References to H's exact physical response source, not a second thread ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateHistorySource {
    pub(crate) process_operation_id: String,
    pub(crate) ticket: String,
    pub(crate) custodian_nonce: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PrivateHistoryGeneration {
    pub(crate) history_id: String,
    pub(crate) instance_id: String,
    pub(crate) domain_id: String,
    pub(crate) session_id: String,
    pub(crate) seat_id: String,
    pub(crate) seat_incarnation: String,
    pub(crate) binding_id: String,
    pub(crate) generation: String,
    pub(crate) request_id: String,
    pub(crate) predecessor_binding_id: Option<String>,
    pub(crate) source: Option<PrivateHistorySource>,
}

/// Root constructs this only after proving STOPPED and the same trusted
/// continuation from the original H episode/custody/RPC reply and current A.
pub(crate) struct StoppedPrivateHistory<'a> {
    pub(crate) history_id: &'a str,
    pub(crate) binding_id: &'a str,
    pub(crate) generation: &'a str,
    pub(crate) request_id: &'a str,
    pub(crate) source: &'a PrivateHistorySource,
}

pub(crate) struct PrivateHistoryReceipt {
    pub(crate) generation: PrivateHistoryGeneration,
    pub(crate) directory_ref: String,
    pub(crate) directory: ResolvedDirectory,
    // Retain this through the actual child stop. Only directory.path is an
    // execution root; custody's held ancestors are not model mappings.
    pub(crate) custody: Arc<DirectoryRoots>,
}

#[derive(Debug)]
pub(crate) enum PrivateHistoryError {
    Invalid(&'static str),
    GenerationParse(std::num::ParseIntError),
    Conflict,
    Unknown,
    IdentityChanged,
    Io(io::Error),
    Root(RootLockError),
    Registry(RegistryError),
    Atomic(AtomicError),
    Sqlite(SameOpenError),
    CommitUnknown(SameOpenError),
    RollbackUnknown { original: Box<PrivateHistoryError>, rollback: SameOpenError },
}
impl From<io::Error> for PrivateHistoryError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<RootLockError> for PrivateHistoryError {
    fn from(error: RootLockError) -> Self { Self::Root(error) }
}
impl From<RegistryError> for PrivateHistoryError {
    fn from(error: RegistryError) -> Self { Self::Registry(error) }
}
impl From<AtomicError> for PrivateHistoryError {
    fn from(error: AtomicError) -> Self { Self::Atomic(error) }
}
impl From<SameOpenError> for PrivateHistoryError {
    fn from(error: SameOpenError) -> Self { Self::Sqlite(error) }
}

fn atom(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
}
fn validate(input: &PrivateHistoryLaunch<'_>) -> Result<(), PrivateHistoryError> {
    for (name, value) in [("instance", input.instance_id), ("domain", input.domain_id),
        ("session", input.session_id), ("seat", input.seat_id),
        ("incarnation", input.seat_incarnation), ("binding", input.binding_id),
        ("request", input.request_id)] {
        if !atom(value) { return Err(PrivateHistoryError::Invalid(name)); }
    }
    if input.generation.is_empty() || input.generation.len() > 20
        || !input.generation.bytes().all(|byte| byte.is_ascii_digit())
        || (input.generation != "0" && input.generation.starts_with('0')) {
        return Err(PrivateHistoryError::Invalid("generation"));
    }
    Ok(())
}
fn validate_source(source: &PrivateHistorySource) -> Result<(), PrivateHistoryError> {
    if !atom(&source.process_operation_id) || !atom(&source.ticket) || !atom(&source.custodian_nonce) {
        return Err(PrivateHistoryError::Invalid("H source"));
    }
    Ok(())
}
fn framed(fields: &[&str]) -> String {
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn checked_directory(path: &Path) -> Result<RootIdentity, PrivateHistoryError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err(PrivateHistoryError::IdentityChanged);
    }
    Ok(inspect_root(path)?.identity)
}
fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>,
    action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, PrivateHistoryError>)
    -> Result<T, PrivateHistoryError> {
    db.execute("BEGIN IMMEDIATE")?;
    match action(db) {
        Ok(value) => { db.execute("COMMIT").map_err(PrivateHistoryError::CommitUnknown)?; Ok(value) },
        Err(original) => match db.execute("ROLLBACK") {
            Ok(()) => Err(original),
            Err(rollback) => Err(PrivateHistoryError::RollbackUnknown { original: Box::new(original), rollback }),
        },
    }
}
fn bind(statement: &Statement, values: &[&str]) -> Result<(), PrivateHistoryError> {
    for (index, value) in values.iter().enumerate() { statement.bind_text((index + 1) as i32, value)?; }
    Ok(())
}
fn changed(db: &VerifiedDatabaseConnection<'_>) -> Result<(), PrivateHistoryError> {
    let query = Statement::prepare(db.as_ptr(), "SELECT changes()")?;
    if !query.step_row()? || query.column_text(0)? != "1" { return Err(PrivateHistoryError::Conflict); }
    Ok(())
}

pub(crate) fn read_private_history_generation(db: &VerifiedDatabaseConnection<'_>,
    binding_id: &str, generation: &str) -> Result<Option<PrivateHistoryGeneration>, PrivateHistoryError> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT h.history_id,h.instance_id,h.domain_id,h.session_id,h.seat_id,h.seat_incarnation,
                g.request_id,COALESCE(g.predecessor_binding_id,''),COALESCE(g.process_operation_id,''),
                COALESCE(g.ticket,''),COALESCE(g.custodian_nonce,'')
           FROM main.gogoke_v37_instance_history_generations g
           JOIN main.gogoke_v37_instance_histories h ON h.history_id=g.history_id
          WHERE g.binding_id=?1 AND g.generation=?2")?;
    bind(&query, &[binding_id, generation])?;
    if !query.step_row()? { return Ok(None); }
    let predecessor = query.column_text(7)?;
    let operation = query.column_text(8)?;
    let ticket = query.column_text(9)?;
    let nonce = query.column_text(10)?;
    if operation.is_empty() != ticket.is_empty() || operation.is_empty() != nonce.is_empty() {
        return Err(PrivateHistoryError::Unknown);
    }
    let result = PrivateHistoryGeneration {
        history_id: query.column_text(0)?, instance_id: query.column_text(1)?,
        domain_id: query.column_text(2)?, session_id: query.column_text(3)?,
        seat_id: query.column_text(4)?, seat_incarnation: query.column_text(5)?,
        binding_id: binding_id.into(), generation: generation.into(), request_id: query.column_text(6)?,
        predecessor_binding_id: if predecessor.is_empty() { None } else { Some(predecessor) },
        source: if operation.is_empty() { None } else { Some(PrivateHistorySource {
            process_operation_id: operation, ticket, custodian_nonce: nonce }) },
    };
    if query.step_row()? { return Err(PrivateHistoryError::Conflict); }
    Ok(Some(result))
}
fn matches_launch(row: &PrivateHistoryGeneration, input: &PrivateHistoryLaunch<'_>) -> bool {
    row.instance_id == input.instance_id && row.domain_id == input.domain_id
        && row.session_id == input.session_id && row.seat_id == input.seat_id
        && row.seat_incarnation == input.seat_incarnation && row.binding_id == input.binding_id
        && row.generation == input.generation && row.request_id == input.request_id
}
fn same_generation(a: &PrivateHistoryGeneration, b: &PrivateHistoryGeneration) -> bool {
    let mut original = a.clone();
    original.source = b.source.clone();
    original == *b
}

struct HistoryRow {
    root: String, parent: String, home: String, initial_binding: String,
    initial_generation: String, initial_request: String, directory_ref: String,
    identity: String, state: String,
}
fn history(db: &VerifiedDatabaseConnection<'_>, id: &str) -> Result<HistoryRow, PrivateHistoryError> {
    let query = Statement::prepare(db.as_ptr(),
        "SELECT root_identity,parent_identity,home_identity,initial_binding_id,initial_generation,
                initial_request_id,directory_ref,COALESCE(directory_identity,''),state
           FROM main.gogoke_v37_instance_histories WHERE history_id=?1")?;
    query.bind_text(1, id)?;
    if !query.step_row()? { return Err(PrivateHistoryError::Unknown); }
    let row = HistoryRow { root: query.column_text(0)?, parent: query.column_text(1)?,
        home: query.column_text(2)?, initial_binding: query.column_text(3)?,
        initial_generation: query.column_text(4)?, initial_request: query.column_text(5)?,
        directory_ref: query.column_text(6)?, identity: query.column_text(7)?, state: query.column_text(8)? };
    if query.step_row()? { return Err(PrivateHistoryError::Conflict); }
    Ok(row)
}
fn physical_receipt(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    generation: PrivateHistoryGeneration) -> Result<PrivateHistoryReceipt, PrivateHistoryError> {
    let row = history(db, &generation.history_id)?;
    if row.state != "READY" || row.identity.is_empty() {
        return Err(PrivateHistoryError::Unknown);
    }
    let (home, identity) = resolve_registered_codex_home(db, root, &generation.instance_id)?;
    let canonical = root.canonical_root();
    if checked_directory(&canonical.canonical_path)? != canonical.identity
        || row.root != canonical.identity.opaque() || row.home != identity.opaque()
        || row.parent != checked_directory(&canonical.canonical_path.join("v37-instances"))?.opaque()
        || row.directory_ref != format!("history-{}", generation.history_id) {
        return Err(PrivateHistoryError::IdentityChanged);
    }
    let path = home.join(&row.directory_ref);
    let leaf = checked_directory(&path)?;
    if leaf.opaque() != row.identity { return Err(PrivateHistoryError::IdentityChanged); }
    let custody = Arc::new(DirectoryRoots::prepare(root, &[(path.clone(), leaf.clone())])?);
    custody.verify()?;
    if checked_directory(&home)? != identity
        || checked_directory(&canonical.canonical_path.join("v37-instances"))?.opaque() != row.parent {
        return Err(PrivateHistoryError::IdentityChanged);
    }
    Ok(PrivateHistoryReceipt { generation, directory_ref: row.directory_ref,
        directory: ResolvedDirectory { path, identity: leaf }, custody })
}
fn insert_generation(db: &VerifiedDatabaseConnection<'_>, id: &str,
    input: &PrivateHistoryLaunch<'_>, predecessor: Option<&str>) -> Result<(), PrivateHistoryError> {
    let statement = Statement::prepare(db.as_ptr(),
        "INSERT INTO main.gogoke_v37_instance_history_generations
         (binding_id,history_id,generation,request_id,predecessor_binding_id) VALUES(?1,?2,?3,?4,?5)")?;
    bind(&statement, &[input.binding_id, id, input.generation, input.request_id])?;
    if let Some(previous) = predecessor { statement.bind_text(5, previous)?; }
    statement.step_done()?;
    Ok(())
}

/// Reserve the exact host-derived name before touching the filesystem. An
/// interrupted create without a committed physical receipt is preserved and
/// refused on replay, even if the name now looks empty or absent.
pub(crate) fn create_initial_private_history(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, input: &PrivateHistoryLaunch<'_>) -> Result<PrivateHistoryReceipt, PrivateHistoryError> {
    validate(input)?;
    let (home, home_identity) = resolve_registered_codex_home(db, root, input.instance_id)?;
    let parents = DirectoryRoots::prepare(root, &[(home.clone(), home_identity.clone())])?;
    let canonical = root.canonical_root();
    let root_identity = canonical.identity.opaque();
    let parent_identity = checked_directory(&canonical.canonical_path.join("v37-instances"))?.opaque();
    let home_identity = home_identity.opaque();
    let fingerprint = framed(&["private-history-create-v1", &root_identity, &parent_identity,
        &home_identity, input.instance_id, input.domain_id, input.session_id, input.seat_id,
        input.seat_incarnation, input.binding_id, input.generation, input.request_id]);
    let id = sha256_hex(fingerprint.as_bytes());
    let directory_ref = format!("history-{id}");
    let operation = format!("private-history-create-{id}");
    let path = home.join(&directory_ref);
    let prior = transaction(db, |db| {
        parents.verify()?;
        let query = Statement::prepare(db.as_ptr(),
            "SELECT request_hex,target_id,phase,COALESCE(native_receipt_id,'')
               FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
        query.bind_text(1, &operation)?;
        if query.step_row()? {
            if query.column_text(0)? != fingerprint || query.column_text(1)? != id {
                return Err(PrivateHistoryError::Conflict);
            }
            let phase = query.column_text(2)?;
            let receipt = query.column_text(3)?;
            if query.step_row()? { return Err(PrivateHistoryError::Conflict); }
            return Ok(Some((phase, receipt)));
        }
        match fs::symlink_metadata(&path) {
            Ok(_) => return Err(PrivateHistoryError::Unknown),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error.into()),
        }
        let insert = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_histories
             (history_id,instance_id,root_identity,parent_identity,home_identity,domain_id,session_id,
              seat_id,seat_incarnation,initial_binding_id,initial_generation,initial_request_id,
              directory_ref,state,revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'PREPARING',0)")?;
        bind(&insert, &[&id, input.instance_id, &root_identity, &parent_identity, &home_identity,
            input.domain_id, input.session_id, input.seat_id, input.seat_incarnation,
            input.binding_id, input.generation, input.request_id, &directory_ref])?;
        insert.step_done()?;
        insert_generation(db, &id, input, None)?;
        let intent = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase)
             VALUES(?1,?2,?3,'PREPARING')")?;
        bind(&intent, &[&operation, &fingerprint, &id])?;
        intent.step_done()?;
        Ok(None)
    })?;
    let generation = read_private_history_generation(db, input.binding_id, input.generation)?
        .ok_or(PrivateHistoryError::Unknown)?;
    if generation.history_id != id || !matches_launch(&generation, input)
        || generation.predecessor_binding_id.is_some() { return Err(PrivateHistoryError::Conflict); }
    let row = history(db, &id)?;
    if row.initial_binding != input.binding_id || row.initial_generation != input.generation
        || row.initial_request != input.request_id || row.root != root_identity
        || row.parent != parent_identity || row.home != home_identity
        || row.directory_ref != directory_ref { return Err(PrivateHistoryError::Conflict); }
    let leaf_custody;
    if let Some((phase, receipt)) = &prior {
        if phase == "APPLIED" && row.state == "READY" && *receipt == row.identity && !receipt.is_empty() {
            return physical_receipt(db, root, generation);
        }
        if !matches!(phase.as_str(), "PREPARING" | "UNKNOWN") || receipt.is_empty()
            || *receipt != row.identity || !matches!(row.state.as_str(), "PREPARING" | "UNKNOWN") {
            return Err(PrivateHistoryError::Unknown);
        }
        if checked_directory(&path)?.opaque() != *receipt { return Err(PrivateHistoryError::IdentityChanged); }
        let leaf = checked_directory(&path)?;
        leaf_custody = DirectoryRoots::prepare(root, &[(path.clone(), leaf)])?;
    } else {
        parents.verify()?;
        fs::create_dir(&path)?; // create-new, no existing-name adoption
        let leaf = checked_directory(&path)?;
        let held = DirectoryRoots::prepare(root, &[(path.clone(), leaf.clone())])?;
        held.verify()?;
        transaction(db, |db| {
            parents.verify()?;
            held.verify()?;
            let update = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_histories SET directory_identity=?2
                  WHERE history_id=?1 AND state='PREPARING' AND directory_identity IS NULL")?;
            bind(&update, &[&id, &leaf.opaque()])?;
            update.step_done()?;
            changed(db)?;
            let receipt = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id=?2
                  WHERE request_id=?1 AND phase='PREPARING' AND native_receipt_id IS NULL")?;
            bind(&receipt, &[&operation, &leaf.opaque()])?;
            receipt.step_done()?;
            changed(db)
        })?;
        leaf_custody = held;
    }
    transaction(db, |db| {
        parents.verify()?;
        leaf_custody.verify()?;
        let row = history(db, &id)?;
        if row.identity.is_empty() || checked_directory(&path)?.opaque() != row.identity {
            return Err(PrivateHistoryError::IdentityChanged);
        }
        let finish = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_histories SET state='READY',revision=1
              WHERE history_id=?1 AND state IN ('PREPARING','UNKNOWN') AND revision=0")?;
        finish.bind_text(1, &id)?;
        finish.step_done()?;
        changed(db)?;
        let finish = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET phase='APPLIED'
              WHERE request_id=?1 AND phase IN ('PREPARING','UNKNOWN') AND native_receipt_id=?2")?;
        bind(&finish, &[&operation, &row.identity])?;
        finish.step_done()?;
        changed(db)
    })?;
    physical_receipt(db, root, generation)
}

/// This never creates a directory. A missing old binding/source is a refusal,
/// not compatibility permission and not an invitation to search native files.
pub(crate) fn resume_private_history(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &PrivateHistoryLaunch<'_>, old: &StoppedPrivateHistory<'_>)
    -> Result<PrivateHistoryReceipt, PrivateHistoryError> {
    validate(input)?;
    validate_source(old.source)?;
    transaction(db, |db| {
        let prior = read_private_history_generation(db, old.binding_id, old.generation)?
            .ok_or(PrivateHistoryError::Unknown)?;
        if prior.history_id != old.history_id || prior.request_id != old.request_id
            || prior.source.as_ref() != Some(old.source) || prior.instance_id != input.instance_id
            || prior.domain_id != input.domain_id || prior.session_id != input.session_id
            || prior.seat_id != input.seat_id || prior.seat_incarnation != input.seat_incarnation
            || prior.binding_id == input.binding_id || prior.request_id == input.request_id {
            return Err(PrivateHistoryError::Conflict);
        }
        let old_number = old.generation.parse::<u64>().map_err(PrivateHistoryError::GenerationParse)?;
        let new_number = input.generation.parse::<u64>().map_err(PrivateHistoryError::GenerationParse)?;
        if old_number >= new_number { return Err(PrivateHistoryError::Conflict); }
        let physical = physical_receipt(db, root, prior)?;
        if let Some(existing) = read_private_history_generation(db, input.binding_id, input.generation)? {
            if !matches_launch(&existing, input) || existing.history_id != old.history_id
                || existing.predecessor_binding_id.as_deref() != Some(old.binding_id) {
                return Err(PrivateHistoryError::Conflict);
            }
        } else { insert_generation(db, &physical.generation.history_id, input, Some(old.binding_id))?; }
        Ok(())
    })?;
    physical_receipt(db, root, read_private_history_generation(db, input.binding_id, input.generation)?
        .ok_or(PrivateHistoryError::Unknown)?)
}

/// Root calls within the original H response transaction, after checking the
/// original response source. The physical process reference is immutable.
pub(crate) fn bind_private_history_continuation_in_transaction(db: &VerifiedDatabaseConnection<'_>,
    receipt: &PrivateHistoryReceipt, source: &PrivateHistorySource) -> Result<(), PrivateHistoryError> {
    validate_source(source)?;
    unsafe extern "C" { fn sqlite3_get_autocommit(database: *mut std::ffi::c_void) -> i32; }
    if unsafe { sqlite3_get_autocommit(db.as_ptr()) } != 0 {
        return Err(PrivateHistoryError::Invalid("response transaction absent"));
    }
    let original = read_private_history_generation(db, &receipt.generation.binding_id, &receipt.generation.generation)?
        .ok_or(PrivateHistoryError::Unknown)?;
    if !same_generation(&original, &receipt.generation) { return Err(PrivateHistoryError::Conflict); }
    if history(db, &original.history_id)?.state != "READY" { return Err(PrivateHistoryError::Unknown); }
    if let Some(bound) = original.source { return if bound == *source { Ok(()) } else { Err(PrivateHistoryError::Conflict) }; }
    let statement = Statement::prepare(db.as_ptr(),
        "UPDATE main.gogoke_v37_instance_history_generations
            SET process_operation_id=?3,ticket=?4,custodian_nonce=?5
          WHERE binding_id=?1 AND generation=?2 AND process_operation_id IS NULL
            AND ticket IS NULL AND custodian_nonce IS NULL")?;
    bind(&statement, &[&receipt.generation.binding_id, &receipt.generation.generation,
        &source.process_operation_id, &source.ticket, &source.custodian_nonce])?;
    statement.step_done()?;
    changed(db)
}

/// Read-only physical and F binding recheck. Root also rechecks A registration
/// and current H/E authority at the actual pre-activation/active boundary.
pub(crate) fn verify_private_history(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    receipt: &PrivateHistoryReceipt, input: &PrivateHistoryLaunch<'_>) -> Result<(), PrivateHistoryError> {
    validate(input)?;
    let current = read_private_history_generation(db, input.binding_id, input.generation)?
        .ok_or(PrivateHistoryError::Unknown)?;
    if !matches_launch(&current, input) || !same_generation(&current, &receipt.generation) {
        return Err(PrivateHistoryError::Conflict);
    }
    receipt.custody.verify()?;
    let physical = physical_receipt(db, root, current)?;
    if physical.directory != receipt.directory || physical.directory_ref != receipt.directory_ref {
        return Err(PrivateHistoryError::IdentityChanged);
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use super::super::registry::{register_instance, ProgramObservation, Registration};
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &RootLock)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-private-history-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let mut db = create_new(&root, &path.join("state.sqlite")).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        let binary = path.join("fixture-program.bin");
        fs::write(&binary, b"non-executable fixture identity").unwrap();
        let program = ProgramObservation::observe(&binary, "0.160.0").unwrap();
        register_instance(&mut db, &root, &Registration { request_id: "registerA",
            request_bytes: b"register fixture instance", instance_id: "instanceA", driver_id: "codex",
            program: &program }).unwrap();
        run(&mut db, &root);
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }
    fn launch() -> PrivateHistoryLaunch<'static> {
        PrivateHistoryLaunch { instance_id: "instanceA", domain_id: "projectA", session_id: "sessionA",
            seat_id: "seatA", seat_incarnation: "1", binding_id: "bindingA", generation: "1", request_id: "openA" }
    }
    fn source() -> PrivateHistorySource {
        PrivateHistorySource { process_operation_id: "operationA".into(), ticket: "ticketA".into(),
            custodian_nonce: "nonceA".into() }
    }
    fn attach(db: &mut VerifiedDatabaseConnection<'_>, receipt: &PrivateHistoryReceipt) {
        db.execute("BEGIN IMMEDIATE").unwrap();
        bind_private_history_continuation_in_transaction(db, receipt, &source()).unwrap();
        db.execute("COMMIT").unwrap();
    }
    #[test]
    fn initial_is_fresh_resume_selects_exact_source_and_cross_scope_is_refused() {
        fixture(|db, root| {
            let first = create_initial_private_history(db, root, &launch()).unwrap();
            fs::write(first.directory.path.join("fixture-history"), b"preserve original history").unwrap();
            attach(db, &first);
            let replay = create_initial_private_history(db, root, &launch()).unwrap();
            assert_eq!(replay.directory, first.directory);
            let next = PrivateHistoryLaunch { binding_id: "bindingB", generation: "2", request_id: "resumeB", ..launch() };
            let source = source();
            let old = StoppedPrivateHistory { history_id: &first.generation.history_id,
                binding_id: "bindingA", generation: "1", request_id: "openA", source: &source };
            for forbidden in [PrivateHistoryLaunch { domain_id: "projectB", ..next.clone() },
                PrivateHistoryLaunch { session_id: "sessionB", ..next.clone() },
                PrivateHistoryLaunch { seat_id: "seatB", ..next.clone() },
                PrivateHistoryLaunch { seat_incarnation: "2", ..next.clone() }] {
                assert!(matches!(resume_private_history(db, root, &forbidden, &old), Err(PrivateHistoryError::Conflict)));
            }
            assert!(read_private_history_generation(db, "bindingB", "2").unwrap().is_none());
            let resumed = resume_private_history(db, root, &next, &old).unwrap();
            assert_eq!(resumed.directory, first.directory);
            verify_private_history(db, root, &resumed, &next).unwrap();
            let renewed = PrivateHistoryLaunch { binding_id: "bindingC", generation: "3", request_id: "newC", ..launch() };
            let fresh = create_initial_private_history(db, root, &renewed).unwrap();
            assert_ne!(fresh.directory, first.directory);
            assert_eq!(fs::read_dir(&fresh.directory.path).unwrap().count(), 0);
            assert_eq!(fs::read(first.directory.path.join("fixture-history")).unwrap(), b"preserve original history");
        });
    }
    #[test]
    fn absent_physical_commit_preserves_existing_or_missing_leaf_without_recreation() {
        for remove_leaf in [false, true] {
            fixture(|db, root| {
                let first = create_initial_private_history(db, root, &launch()).unwrap();
                let id = first.generation.history_id.clone();
                let path = first.directory.path.clone();
                drop(first);
                db.execute("BEGIN IMMEDIATE").unwrap();
                let reset = Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_instance_histories SET directory_identity=NULL,state='PREPARING',revision=0 WHERE history_id=?1").unwrap();
                reset.bind_text(1, &id).unwrap(); reset.step_done().unwrap();
                let reset = Statement::prepare(db.as_ptr(),
                    "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id=NULL,phase='PREPARING' WHERE target_id=?1").unwrap();
                reset.bind_text(1, &id).unwrap(); reset.step_done().unwrap();
                db.execute("COMMIT").unwrap();
                if remove_leaf { fs::remove_dir(&path).unwrap(); }
                assert!(matches!(create_initial_private_history(db, root, &launch()), Err(PrivateHistoryError::Unknown)));
                assert_eq!(path.exists(), !remove_leaf);
                let count = Statement::prepare(db.as_ptr(), "SELECT count(*) FROM main.gogoke_v37_instance_histories").unwrap();
                assert!(count.step_row().unwrap()); assert_eq!(count.column_text(0).unwrap(), "1");
            });
        }
    }
    #[test]
    fn replaced_leaf_is_refused_and_original_history_is_preserved() {
        fixture(|db, root| {
            let first = create_initial_private_history(db, root, &launch()).unwrap();
            let original = first.directory.path.clone();
            fs::write(original.join("fixture-history"), b"keep").unwrap();
            // Namespace pin prevents replacement while the actual witness lives.
            let preserved = original.with_file_name("preserved-original");
            assert!(fs::rename(&original, &preserved).is_err());
            drop(first);
            fs::rename(&original, &preserved).unwrap();
            fs::create_dir(&original).unwrap();
            assert!(matches!(create_initial_private_history(db, root, &launch()), Err(PrivateHistoryError::IdentityChanged)));
            assert_eq!(fs::read(preserved.join("fixture-history")).unwrap(), b"keep");
            assert_eq!(fs::read_dir(original).unwrap().count(), 0);
        });
    }
    #[test]
    fn h_source_attachment_requires_response_transaction_and_never_rebinds() {
        fixture(|db, root| {
            let first = create_initial_private_history(db, root, &launch()).unwrap();
            assert!(matches!(bind_private_history_continuation_in_transaction(db, &first, &source()),
                Err(PrivateHistoryError::Invalid("response transaction absent"))));
            attach(db, &first);
            attach(db, &first);
            db.execute("BEGIN IMMEDIATE").unwrap();
            let different = PrivateHistorySource { ticket: "another-ticket".into(), ..source() };
            assert!(matches!(bind_private_history_continuation_in_transaction(db, &first, &different),
                Err(PrivateHistoryError::Conflict)));
            db.execute("ROLLBACK").unwrap();
            assert_eq!(read_private_history_generation(db, "bindingA", "1").unwrap().unwrap().source, Some(source()));
            verify_private_history(db, root, &first, &launch()).unwrap();
        });
    }
}
