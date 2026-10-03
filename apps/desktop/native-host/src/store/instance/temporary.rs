//! F.1 temporary SESSION/CALL home creation. H's durable stop and admission
//! fence are not yet wired: close and cleanup fail closed and never delete.

use super::registry::{observed_home, RegistryError};
use crate::process::{AppContainerProfile, IsolationError};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::{AtomicError, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};
use crate::store::session_transport::{verify_home_owner_in_transaction,
    verify_home_stop_in_transaction, fence_home_admission_in_transaction, AdmissionError};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::MetadataExt;
use std::path::Path;

const CONTAINER: &str = "v37-instances";
const TEMPORARY_CONTAINER: &str = "v37-temporary-homes";
const MARKER: &str = "gogoke-temporary-home.marker";
const REPARSE_POINT: u32 = 0x400;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TemporaryKind { Session, Call }
impl TemporaryKind {
    fn as_str(self) -> &'static str {
        match self { Self::Session => "SESSION", Self::Call => "CALL" }
    }
}

pub(crate) struct CreateTemporaryHome<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) home_id: &'a str,
    pub(crate) instance_id: &'a str,
    pub(crate) domain_id: &'a str,
    pub(crate) kind: TemporaryKind,
    pub(crate) owner_id: &'a str,
    pub(crate) generation: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TemporaryHomeReceipt {
    pub(crate) disposition: &'static str,
    pub(crate) directory_ref: String,
    pub(crate) native_receipt_id: String,
}

#[derive(Debug)]
pub(crate) enum TemporaryHomeError {
    Invalid(&'static str),
    RequestConflict,
    HomeConflict,
    StaleRevision,
    InstanceChanged,
    IdentityChanged,
    Unknown,
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
    StopFactUnavailable,
    Io(io::Error),
    Atomic(AtomicError),
    Sqlite(SameOpenError),
    Registry(RegistryError),
    Admission(AdmissionError),
    Isolation(IsolationError),
}
impl From<io::Error> for TemporaryHomeError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}
impl From<AtomicError> for TemporaryHomeError {
    fn from(error: AtomicError) -> Self { Self::Atomic(error) }
}
impl From<SameOpenError> for TemporaryHomeError {
    fn from(error: SameOpenError) -> Self { Self::Sqlite(error) }
}
impl From<RegistryError> for TemporaryHomeError {
    fn from(error: RegistryError) -> Self { Self::Registry(error) }
}
impl From<AdmissionError> for TemporaryHomeError {
    fn from(error: AdmissionError) -> Self { Self::Admission(error) }
}
impl From<IsolationError> for TemporaryHomeError {
    fn from(error: IsolationError) -> Self { Self::Isolation(error) }
}

fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty() && bytes.len() <= 128 && bytes[0].is_ascii_alphabetic()
        && bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
}
fn valid_generation(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}
fn validate(input: &CreateTemporaryHome<'_>) -> Result<(), TemporaryHomeError> {
    for (name, value) in [("request_id", input.request_id), ("home_id", input.home_id),
        ("instance_id", input.instance_id), ("domain_id", input.domain_id),
        ("owner_id", input.owner_id)] {
        if !valid_id(value) { return Err(TemporaryHomeError::Invalid(name)); }
    }
    if !valid_generation(input.generation) { return Err(TemporaryHomeError::Invalid("generation")); }
    if input.request_bytes.is_empty() || input.request_bytes.len() > 65_536 {
        return Err(TemporaryHomeError::Invalid("request_bytes"));
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes { result.push(DIGITS[(byte >> 4) as usize] as char);
        result.push(DIGITS[(byte & 15) as usize] as char); }
    result
}
fn fingerprint(input: &CreateTemporaryHome<'_>, sid_identity: &str) -> String {
    let mut framed = Vec::new();
    for value in [&b"temporary-create-detached"[..], input.request_bytes, input.home_id.as_bytes(),
        input.instance_id.as_bytes(), input.domain_id.as_bytes(), input.kind.as_str().as_bytes(),
        input.owner_id.as_bytes(), input.generation.as_bytes(), sid_identity.as_bytes()] {
        framed.extend_from_slice(&(value.len() as u64).to_be_bytes());
        framed.extend_from_slice(value);
    }
    hex(&framed)
}
fn directory_ref(home_id: &str) -> String {
    format!("temp-home-{}", sha256_hex(home_id.as_bytes()))
}
fn directory(root: &RootLock, input: &CreateTemporaryHome<'_>) -> std::path::PathBuf {
    root.canonical_root().canonical_path.join(TEMPORARY_CONTAINER).join(input.instance_id)
        .join(directory_ref(input.home_id))
}
fn checked_dir(path: &Path) -> Result<RootIdentity, TemporaryHomeError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    Ok(inspect_root(path).map_err(RegistryError::Root)?.identity)
}
fn existing_parent(path: &Path) -> Result<Option<RootIdentity>, TemporaryHomeError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(Some(checked_dir(path)?)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn marker_text(root: &RootLock, input: &CreateTemporaryHome<'_>, identity: &RootIdentity,
    sid_identity: &str) -> String {
    format!("gogoke-v37-temporary-home-detached-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        root.canonical_root().identity.opaque(), input.instance_id, input.home_id, input.domain_id,
        input.kind.as_str(), input.owner_id, input.generation, input.request_id,
        sha256_hex(fingerprint(input, sid_identity).as_bytes()), identity.opaque(), sid_identity)
}
#[derive(Clone, Debug, Eq, PartialEq)]
enum DirectoryStage { Empty(RootIdentity), Marked(RootIdentity, bool) }
fn directory_stage(root: &RootLock, input: &CreateTemporaryHome<'_>, sid_identity: &str)
    -> Result<Option<DirectoryStage>, TemporaryHomeError> {
    let root_path = &root.canonical_root().canonical_path;
    if checked_dir(root_path)? != root.canonical_root().identity { return Err(TemporaryHomeError::IdentityChanged); }
    let top = root_path.join(CONTAINER);
    let Some(top_identity) = existing_parent(&top)? else { return Ok(None); };
    let instance = top.join(input.instance_id);
    let Some(instance_identity) = existing_parent(&instance)? else { return Ok(None); };
    if observed_home(root, input.instance_id)? != Some(instance_identity.clone()) {
        return Err(TemporaryHomeError::InstanceChanged);
    }
    let temporary_top = root_path.join(TEMPORARY_CONTAINER);
    let Some(temporary_top_identity) = existing_parent(&temporary_top)? else { return Ok(None); };
    let temporary = temporary_top.join(input.instance_id);
    let Some(temporary_identity) = existing_parent(&temporary)? else { return Ok(None); };
    let path = directory(root, input);
    let Some(identity) = existing_parent(&path)? else { return Ok(None); };
    let mut saw_marker = false;
    let mut saw_other_content = false;
    for entry in fs::read_dir(&path)? {
        if entry?.file_name().as_os_str() == std::ffi::OsStr::new(MARKER) { saw_marker = true; }
        else { saw_other_content = true; }
    }
    let stage = if !saw_marker && !saw_other_content {
        DirectoryStage::Empty(identity.clone())
    } else if saw_marker {
        let marker = path.join(MARKER);
        let metadata = fs::symlink_metadata(&marker)?;
        if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0
            || fs::read_to_string(&marker)? != marker_text(root, input, &identity, sid_identity) {
            return Err(TemporaryHomeError::IdentityChanged);
        }
        DirectoryStage::Marked(identity.clone(), saw_other_content)
    } else {
        return Err(TemporaryHomeError::Unknown);
    };
    if checked_dir(&top)? != top_identity || checked_dir(&instance)? != instance_identity
        || checked_dir(&temporary_top)? != temporary_top_identity
        || checked_dir(&temporary)? != temporary_identity || checked_dir(&path)? != identity {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    Ok(Some(stage))
}
fn prepare_empty_directory(root: &RootLock, input: &CreateTemporaryHome<'_>, sid_identity: &str)
    -> Result<RootIdentity, TemporaryHomeError> {
    let root_path = &root.canonical_root().canonical_path;
    if checked_dir(root_path)? != root.canonical_root().identity { return Err(TemporaryHomeError::IdentityChanged); }
    let top = root_path.join(CONTAINER);
    match fs::create_dir(&top) {
        Ok(()) => (), Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let top_identity = checked_dir(&top)?;
    let instance = top.join(input.instance_id);
    let instance_identity = checked_dir(&instance)?;
    if observed_home(root, input.instance_id)? != Some(instance_identity.clone()) {
        return Err(TemporaryHomeError::InstanceChanged);
    }
    let temporary_top = root_path.join(TEMPORARY_CONTAINER);
    match fs::create_dir(&temporary_top) {
        Ok(()) => (), Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let temporary_top_identity = checked_dir(&temporary_top)?;
    let temporary = temporary_top.join(input.instance_id);
    match fs::create_dir(&temporary) {
        Ok(()) => (), Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    let temporary_identity = checked_dir(&temporary)?;
    let path = directory(root, input);
    fs::create_dir(&path).map_err(|error| if error.kind() == io::ErrorKind::AlreadyExists {
        TemporaryHomeError::Unknown
    } else { TemporaryHomeError::Io(error) })?;
    let identity = checked_dir(&path)?;
    if checked_dir(&top)? != top_identity || checked_dir(&instance)? != instance_identity
        || checked_dir(&temporary_top)? != temporary_top_identity
        || checked_dir(&temporary)? != temporary_identity
        || directory_stage(root, input, sid_identity)? != Some(DirectoryStage::Empty(identity.clone())) {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    Ok(identity)
}
fn write_marker_after_grant(root: &RootLock, input: &CreateTemporaryHome<'_>,
    identity: &RootIdentity, sid_identity: &str) -> Result<(), TemporaryHomeError> {
    if directory_stage(root, input, sid_identity)? != Some(DirectoryStage::Empty(identity.clone())) {
        return Err(TemporaryHomeError::Unknown);
    }
    let mut marker = OpenOptions::new().write(true).create_new(true)
        .open(directory(root, input).join(MARKER))?;
    marker.write_all(marker_text(root, input, identity, sid_identity).as_bytes())?;
    marker.sync_all()?;
    if directory_stage(root, input, sid_identity)? != Some(DirectoryStage::Marked(identity.clone(), false)) {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    Ok(())
}

fn operation(connection: &VerifiedDatabaseConnection<'_>, request_id: &str)
    -> Result<Option<(String, String, String, String)>, TemporaryHomeError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    row.bind_text(1, request_id)?;
    if row.step_row()? { Ok(Some((row.column_text(0)?, row.column_text(1)?, row.column_text(2)?, row.column_text(3)?))) }
    else { Ok(None) }
}
fn home_row(connection: &VerifiedDatabaseConnection<'_>, home_id: &str)
    -> Result<Option<(String, String, String, String, String, String, String, String, String)>, TemporaryHomeError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT instance_id,domain_id,kind,owner_id,generation,COALESCE(directory_ref,''),COALESCE(directory_identity,''),state,CAST(revision AS TEXT) FROM main.gogoke_v37_instance_homes WHERE home_id=?1")?;
    row.bind_text(1, home_id)?;
    if row.step_row()? { Ok(Some((row.column_text(0)?,row.column_text(1)?,row.column_text(2)?,
        row.column_text(3)?,row.column_text(4)?,row.column_text(5)?,row.column_text(6)?,
        row.column_text(7)?,row.column_text(8)?))) } else { Ok(None) }
}
fn instance_identity(connection: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<Option<String>, TemporaryHomeError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT home_identity FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
    row.bind_text(1, instance_id)?;
    if row.step_row()? { Ok(Some(row.column_text(0)?)) } else { Ok(None) }
}
fn transaction<T>(connection: &mut VerifiedDatabaseConnection<'_>, action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, TemporaryHomeError>)
    -> Result<T, TemporaryHomeError> {
    connection.execute("BEGIN IMMEDIATE")?;
    match action(connection) {
        Ok(value) => { connection.execute("COMMIT").map_err(TemporaryHomeError::CommitUnknown)?; Ok(value) },
        Err(error) => { connection.execute("ROLLBACK").map_err(TemporaryHomeError::RollbackUnknown)?; Err(error) },
    }
}
fn matching_row(row: &(String, String, String, String, String, String, String, String, String),
    input: &CreateTemporaryHome<'_>) -> bool {
    row.0 == input.instance_id && row.1 == input.domain_id && row.2 == input.kind.as_str()
        && row.3 == input.owner_id && row.4 == input.generation
}

/// The caller supplies the native issuer and root pin. H's durable owner binding
/// is verified on the same connection and transaction as F's create intent.
pub(crate) fn create_temporary_home(connection: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    profile: &AppContainerProfile, input: &CreateTemporaryHome<'_>)
    -> Result<TemporaryHomeReceipt, TemporaryHomeError> {
    validate(input)?;
    let sid_identity = profile.sid_identity()?;
    let request_hex = fingerprint(input, &sid_identity);
    let persistent = observed_home(root, input.instance_id)?.ok_or(TemporaryHomeError::InstanceChanged)?;
    let persistent_identity = persistent.opaque();
    let prior = transaction(connection, |connection| {
        if instance_identity(connection, input.instance_id)?.as_deref() != Some(persistent_identity.as_str()) {
            return Err(TemporaryHomeError::InstanceChanged);
        }
        verify_home_owner_in_transaction(connection, input.instance_id, input.domain_id,
            input.kind.as_str(), input.owner_id, input.generation)?;
        if let Some((stored, target, phase, receipt)) = operation(connection, input.request_id)? {
            if stored != request_hex || target != input.home_id { return Err(TemporaryHomeError::RequestConflict); }
            let row = home_row(connection, input.home_id)?.ok_or(TemporaryHomeError::Unknown)?;
            if !matching_row(&row, input) { return Err(TemporaryHomeError::HomeConflict); }
            return Ok(Some((phase, receipt, row)));
        }
        if home_row(connection, input.home_id)?.is_some() { return Err(TemporaryHomeError::HomeConflict); }
        // An intent only authorizes recovery of a directory observed absent
        // before that intent was written. Later recovery uses the bound marker.
        if existing_parent(&directory(root, input))?.is_some() { return Err(TemporaryHomeError::Unknown); }
        let home = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES(?1,?2,?3,?4,?5,?6,'PREPARING',0)")?;
        for (index, value) in [input.home_id,input.instance_id,input.domain_id,input.kind.as_str(),
            input.owner_id,input.generation].iter().enumerate() { home.bind_text((index+1) as i32, value)?; }
        home.step_done()?;
        let intent = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'PREPARING')")?;
        intent.bind_text(1, input.request_id)?;
        intent.bind_text(2, &request_hex)?;
        intent.bind_text(3, input.home_id)?;
        intent.step_done()?;
        Ok(None)
    })?;
    if let Some((phase, receipt, row)) = &prior {
        if phase == "APPLIED" && row.7 == "ACTIVE" && row.5 == directory_ref(input.home_id)
            && row.6 == receipt.as_str() && !receipt.is_empty()
            && matches!(directory_stage(root, input, &sid_identity)?,
                Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == receipt.as_str()) {
            return Ok(TemporaryHomeReceipt { disposition: "REPLAYED", directory_ref: row.5.clone(),
                native_receipt_id: receipt.clone() });
        }
        if phase != "PREPARING" && phase != "UNKNOWN" { return Err(TemporaryHomeError::Unknown); }
    }
    let observed = directory_stage(root, input, &sid_identity)?;
    let identity = match observed {
        Some(DirectoryStage::Marked(identity, false)) if prior.as_ref().is_some_and(|(_, receipt, _)|
            receipt.is_empty() || receipt == &identity.opaque()) => identity,
        Some(DirectoryStage::Empty(identity)) if prior.as_ref().is_some_and(|(_, receipt, _)|
            receipt.is_empty()) => {
            profile.grant_fresh_session_directory(&directory(root, input))?;
            if checked_dir(&directory(root, input))? != identity { return Err(TemporaryHomeError::IdentityChanged); }
            write_marker_after_grant(root, input, &identity, &sid_identity)?;
            identity
        }
        Some(_) => return Err(TemporaryHomeError::Unknown),
        None if prior.as_ref().is_some_and(|(_, receipt, _)| !receipt.is_empty()) =>
            return Err(TemporaryHomeError::Unknown),
        None => {
            let identity = prepare_empty_directory(root, input, &sid_identity)?;
            profile.grant_fresh_session_directory(&directory(root, input))?;
            if checked_dir(&directory(root, input))? != identity { return Err(TemporaryHomeError::IdentityChanged); }
            write_marker_after_grant(root, input, &identity, &sid_identity)?;
            identity
        },
    };
    if directory_stage(root, input, &sid_identity)? != Some(DirectoryStage::Marked(identity.clone(), false)) {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    transaction(connection, |connection| {
        verify_home_owner_in_transaction(connection, input.instance_id, input.domain_id,
            input.kind.as_str(), input.owner_id, input.generation)?;
        let Some((stored, target, phase, receipt)) = operation(connection, input.request_id)? else { return Err(TemporaryHomeError::Unknown); };
        if stored != request_hex || target != input.home_id { return Err(TemporaryHomeError::RequestConflict); }
        if phase != "PREPARING" && phase != "UNKNOWN" { return Err(TemporaryHomeError::Unknown); }
        if !receipt.is_empty() && receipt != identity.opaque() { return Err(TemporaryHomeError::IdentityChanged); }
        if receipt.is_empty() {
            let update = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id=?1 WHERE request_id=?2 AND native_receipt_id IS NULL")?;
            update.bind_text(1, &identity.opaque())?;
            update.bind_text(2, input.request_id)?;
            update.step_done()?;
        }
        Ok(())
    })?;
    transaction(connection, |connection| {
        verify_home_owner_in_transaction(connection, input.instance_id, input.domain_id,
            input.kind.as_str(), input.owner_id, input.generation)?;
        let Some((stored, target, phase, receipt)) = operation(connection, input.request_id)? else { return Err(TemporaryHomeError::Unknown); };
        if stored != request_hex || target != input.home_id || receipt != identity.opaque()
            || (phase != "PREPARING" && phase != "UNKNOWN") { return Err(TemporaryHomeError::Unknown); }
        let row = home_row(connection, input.home_id)?.ok_or(TemporaryHomeError::Unknown)?;
        if !matching_row(&row, input) || row.7 != "PREPARING" || row.8 != "0" {
            return Err(TemporaryHomeError::HomeConflict);
        }
        let update = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_homes SET directory_ref=?1,directory_identity=?2,state='ACTIVE',revision=1 WHERE home_id=?3 AND state='PREPARING' AND revision=0")?;
        update.bind_text(1, &directory_ref(input.home_id))?;
        update.bind_text(2, &identity.opaque())?;
        update.bind_text(3, input.home_id)?;
        update.step_done()?;
        let done = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET phase='APPLIED' WHERE request_id=?1 AND phase IN ('PREPARING','UNKNOWN')")?;
        done.bind_text(1, input.request_id)?;
        done.step_done()?;
        Ok(())
    })?;
    if !matches!(directory_stage(root, input, &sid_identity)?,
        Some(DirectoryStage::Marked(observed, _)) if observed == identity) {
        return Err(TemporaryHomeError::Unknown);
    }
    Ok(TemporaryHomeReceipt { disposition: "APPLIED", directory_ref: directory_ref(input.home_id),
        native_receipt_id: identity.opaque() })
}

pub(crate) struct TransitionTemporaryHome<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) home_id: &'a str,
    pub(crate) expected_revision: i64,
}

type HomeRow = (String, String, String, String, String, String, String, String, String);

fn transition_fingerprint(operation: &str, input: &TransitionTemporaryHome<'_>, sid: &str) -> String {
    let mut framed = Vec::new();
    let revision = input.expected_revision.to_string();
    for value in [operation.as_bytes(), input.request_bytes, input.home_id.as_bytes(),
        revision.as_bytes(), sid.as_bytes()] {
        framed.extend_from_slice(&(value.len() as u64).to_be_bytes());
        framed.extend_from_slice(value);
    }
    hex(&framed)
}
fn validate_transition(input: &TransitionTemporaryHome<'_>, revision: i64)
    -> Result<(), TemporaryHomeError> {
    if !valid_id(input.request_id) || !valid_id(input.home_id)
        || input.request_bytes.is_empty() || input.request_bytes.len() > 65_536 {
        return Err(TemporaryHomeError::Invalid("transition"));
    }
    if input.expected_revision != revision { return Err(TemporaryHomeError::StaleRevision); }
    Ok(())
}
fn transition_operation(connection: &VerifiedDatabaseConnection<'_>, request_id: &str)
    -> Result<Option<(String, String, String, String)>, TemporaryHomeError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT request_hex,target_id,phase,COALESCE(receipt_json,'') FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    row.bind_text(1, request_id)?;
    if row.step_row()? { Ok(Some((row.column_text(0)?,row.column_text(1)?,row.column_text(2)?,row.column_text(3)?))) }
    else { Ok(None) }
}
fn transition_path(root: &RootLock, instance_id: &str, home_id: &str) -> std::path::PathBuf {
    root.canonical_root().canonical_path.join(TEMPORARY_CONTAINER).join(instance_id)
        .join(directory_ref(home_id))
}
fn transition_parent(root: &RootLock, row: &HomeRow) -> Result<RootIdentity, TemporaryHomeError> {
    let root_path = &root.canonical_root().canonical_path;
    if checked_dir(root_path)? != root.canonical_root().identity { return Err(TemporaryHomeError::IdentityChanged); }
    let top = root_path.join(CONTAINER);
    checked_dir(&top)?;
    let instance = top.join(&row.0);
    let instance_identity = checked_dir(&instance)?;
    if observed_home(root, &row.0)? != Some(instance_identity) {
        return Err(TemporaryHomeError::InstanceChanged);
    }
    let temporary_top = root_path.join(TEMPORARY_CONTAINER);
    checked_dir(&temporary_top)?;
    Ok(checked_dir(&temporary_top.join(&row.0))?)
}
fn create_marker_for_row(connection: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    home_id: &str, row: &HomeRow, sid: &str) -> Result<String, TemporaryHomeError> {
    let tag = b"temporary-create-detached";
    let prefix = format!("{:016x}{}", tag.len(), hex(tag));
    let query = Statement::prepare(connection.as_ptr(),
        "SELECT request_id,request_hex FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase='APPLIED'")?;
    query.bind_text(1, home_id)?;
    let mut create = None;
    while query.step_row()? {
        let request_id = query.column_text(0)?;
        let request_hex = query.column_text(1)?;
        if request_hex.starts_with(&prefix) {
            if create.replace((request_id, request_hex)).is_some() { return Err(TemporaryHomeError::Unknown); }
        }
    }
    let (request_id, request_hex) = create.ok_or(TemporaryHomeError::Unknown)?;
    Ok(format!("gogoke-v37-temporary-home-detached-v1\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        root.canonical_root().identity.opaque(), row.0, home_id, row.1, row.2, row.3, row.4,
        request_id, sha256_hex(request_hex.as_bytes()), row.6, sid))
}
fn transition_stage(connection: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    home_id: &str, row: &HomeRow, sid: &str)
    -> Result<Option<DirectoryStage>, TemporaryHomeError> {
    transition_parent(root, row)?;
    let path = transition_path(root, &row.0, home_id);
    let Some(identity) = existing_parent(&path)? else { return Ok(None); };
    let mut marker = false;
    let mut other = false;
    for entry in fs::read_dir(&path)? {
        if entry?.file_name().as_os_str() == std::ffi::OsStr::new(MARKER) { marker = true; }
        else { other = true; }
    }
    let stage = if marker {
        let marker_path = path.join(MARKER);
        let metadata = fs::symlink_metadata(&marker_path)?;
        if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0
            || fs::read_to_string(&marker_path)? != create_marker_for_row(connection, root, home_id, row, sid)? {
            return Err(TemporaryHomeError::IdentityChanged);
        }
        DirectoryStage::Marked(identity.clone(), other)
    } else if !other { DirectoryStage::Empty(identity.clone()) }
    else { return Err(TemporaryHomeError::Unknown); };
    if checked_dir(&path)? != identity { return Err(TemporaryHomeError::IdentityChanged); }
    Ok(Some(stage))
}

/// Resolve an ACTIVE SESSION home for H after checking the durable owner
/// binding, the persistent instance identity, and the bound marker/path.
/// This function only reads the marker and directory metadata; it never opens
/// a credential file.
pub(super) fn resolve_active_session_home(
    connection: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    profile: &AppContainerProfile,
    instance_id: &str,
    home_id: &str,
    domain_id: &str,
    owner_id: &str,
    generation: &str,
) -> Result<(std::path::PathBuf, RootIdentity), TemporaryHomeError> {
    for (name, value) in [
        ("instance_id", instance_id),
        ("home_id", home_id),
        ("domain_id", domain_id),
        ("owner_id", owner_id),
    ] {
        if !valid_id(value) {
            return Err(TemporaryHomeError::Invalid(name));
        }
    }
    if !valid_generation(generation) {
        return Err(TemporaryHomeError::Invalid("generation"));
    }
    let row = home_row(connection, home_id)?.ok_or(TemporaryHomeError::Unknown)?;
    if row.0 != instance_id
        || row.1 != domain_id
        || row.2 != "SESSION"
        || row.3 != owner_id
        || row.4 != generation
        || row.7 != "ACTIVE"
        || row.8 != "1"
    {
        return Err(TemporaryHomeError::Unknown);
    }
    row_identity(&row, home_id)?;

    let persistent = observed_home(root, instance_id)?.ok_or(TemporaryHomeError::InstanceChanged)?;
    if instance_identity(connection, instance_id)?.as_deref() != Some(persistent.opaque().as_str()) {
        return Err(TemporaryHomeError::InstanceChanged);
    }

    let binding = Statement::prepare(
        connection.as_ptr(),
        "SELECT COUNT(*) FROM main.gogoke_v37_h_owner_binding WHERE instance_id=?1 AND domain_id=?2 AND kind='SESSION' AND owner_id=?3 AND generation=?4 AND state='ACTIVE'",
    )?;
    for (index, value) in [instance_id, domain_id, owner_id, generation]
        .iter()
        .enumerate()
    {
        binding.bind_text((index + 1) as i32, value)?;
    }
    if !binding.step_row()? || binding.column_text(0)? != "1" {
        return Err(TemporaryHomeError::Unknown);
    }

    let sid = profile.sid_identity()?;
    let path = transition_path(root, instance_id, home_id);
    let identity = match transition_stage(connection, root, home_id, &row, &sid)? {
        Some(DirectoryStage::Marked(identity, _)) => identity,
        Some(DirectoryStage::Empty(_)) | None => return Err(TemporaryHomeError::Unknown),
    };
    if identity.opaque() != row.6 {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    Ok((path, identity))
}

fn exact_stopped_home(connection: &mut VerifiedDatabaseConnection<'_>, home_id: &str,
    row: &HomeRow) -> Result<String, TemporaryHomeError> {
    let stop = verify_home_stop_in_transaction(connection, &row.0, &row.1, &row.2, &row.3, &row.4)?;
    // H's claim is the current admission pointer and moves to the new home on
    // resume. The stopped process episode remains the original home's proof.
    let original = Statement::prepare(connection.as_ptr(),
        "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode AS e
           JOIN main.gogoke_v37_h_generation AS g ON g.domain_id=e.domain_id
             AND g.session_id=e.session_id AND g.generation=e.generation
             AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id
           JOIN main.gogoke_coordination_process_custody AS c
             ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
             AND c.generation=e.generation
           JOIN main.gogoke_v37_h_owner_binding AS b ON b.binding_id=e.binding_id
             AND b.instance_id=e.instance_id AND b.domain_id=e.domain_id
             AND b.kind='SESSION' AND b.owner_id=e.session_id AND b.generation=e.generation
          WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3
            AND e.session_id=?4 AND e.generation=?5 AND e.phase='STOPPED'
            AND e.stop_fact_id=?6 AND e.result_revision IS NOT NULL
            AND c.state='STOPPED' AND c.stop_proof_hash=?6")?;
    for (index, value) in [home_id,&row.0,&row.1,&row.3,&row.4,&stop].iter().enumerate() {
        original.bind_text((index+1) as i32, value)?;
    }
    if !original.step_row()? || original.column_text(0)? != "1" {
        return Err(TemporaryHomeError::StopFactUnavailable);
    }
    drop(original);
    let current = Statement::prepare(connection.as_ptr(),
        "SELECT generation FROM main.gogoke_v37_h_claim
          WHERE domain_id=?1 AND session_id=?2 AND instance_id=?3")?;
    current.bind_text(1, &row.1)?;
    current.bind_text(2, &row.3)?;
    current.bind_text(3, &row.0)?;
    if !current.step_row()? { return Err(TemporaryHomeError::StopFactUnavailable); }
    let current_generation = current.column_text(0)?;
    if current.step_row()? { return Err(TemporaryHomeError::StopFactUnavailable); }
    drop(current);
    if current_generation == row.4 {
        // Before rollover, a contradictory current pointer is still a denial.
        let same = Statement::prepare(connection.as_ptr(),
            "SELECT COUNT(*) FROM main.gogoke_v37_h_claim AS a
               JOIN main.gogoke_v37_h_process_episode AS e
                 ON e.domain_id=a.domain_id AND e.session_id=a.session_id
                 AND e.generation=a.generation
                 AND e.process_operation_id=a.process_operation_id
              WHERE a.domain_id=?1 AND a.session_id=?2 AND a.instance_id=?3
                AND a.home_id=?4 AND a.generation=?5
                AND a.binding_id=e.binding_id AND a.state IN ('STOPPED','RELEASED')
                AND a.stop_fact_id=?6 AND e.stop_fact_id=?6")?;
        for (index, value) in [row.1.as_str(),row.3.as_str(),row.0.as_str(),
            home_id,row.4.as_str(),stop.as_str()].iter().enumerate() {
            same.bind_text((index+1) as i32, value)?;
        }
        if !same.step_row()? || same.column_text(0)? != "1" {
            return Err(TemporaryHomeError::StopFactUnavailable);
        }
    } else {
        // Follow only successful, mapped H generations from the current
        // pointer back to this home's generation. UNION terminates on a
        // corrupted cyclic ancestry without accepting an unrelated claim.
        let ancestry = Statement::prepare(connection.as_ptr(),
            "WITH RECURSIVE chain(generation,old_generation) AS (
               SELECT e.generation,e.old_generation
                 FROM main.gogoke_v37_h_claim AS a
                 JOIN main.gogoke_v37_h_generation AS g
                   ON g.domain_id=a.domain_id AND g.session_id=a.session_id
                   AND g.generation=a.generation
                   AND g.process_operation_id=a.process_operation_id
                 JOIN main.gogoke_v37_h_process_episode AS e
                   ON e.domain_id=g.domain_id AND e.session_id=g.session_id
                   AND e.generation=g.generation AND e.request_id=g.request_id
                   AND e.process_operation_id=g.process_operation_id
                WHERE a.domain_id=?1 AND a.session_id=?2 AND a.instance_id=?3
                  AND a.generation=?4 AND a.home_id=e.home_id
                  AND a.binding_id=e.binding_id AND e.instance_id=a.instance_id
                  AND e.phase IN ('ACTIVE','STOPPED')
               UNION
               SELECT p.generation,p.old_generation FROM chain AS child
                 JOIN main.gogoke_v37_h_process_episode AS p
                   ON p.domain_id=?1 AND p.session_id=?2
                   AND p.generation=child.old_generation AND p.instance_id=?3
                 JOIN main.gogoke_v37_h_generation AS pg
                   ON pg.domain_id=p.domain_id AND pg.session_id=p.session_id
                   AND pg.generation=p.generation AND pg.request_id=p.request_id
                   AND pg.process_operation_id=p.process_operation_id
                WHERE p.phase IN ('ACTIVE','STOPPED')
             ) SELECT COUNT(*) FROM chain WHERE generation=?5")?;
        for (index, value) in [&row.1,&row.3,&row.0,&current_generation,&row.4].iter().enumerate() {
            ancestry.bind_text((index+1) as i32, value)?;
        }
        if !ancestry.step_row()? || ancestry.column_text(0)? != "1" {
            return Err(TemporaryHomeError::StopFactUnavailable);
        }
    }
    Ok(stop)
}
fn row_identity(row: &HomeRow, home_id: &str) -> Result<(), TemporaryHomeError> {
    if row.5 != directory_ref(home_id) || row.6.is_empty() { return Err(TemporaryHomeError::IdentityChanged); }
    Ok(())
}
fn receipt(disposition: &'static str, row: &HomeRow) -> TemporaryHomeReceipt {
    TemporaryHomeReceipt { disposition, directory_ref: row.5.clone(), native_receipt_id: row.6.clone() }
}

pub(crate) fn close_temporary_home(connection: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, profile: &AppContainerProfile, input: &TransitionTemporaryHome<'_>)
    -> Result<TemporaryHomeReceipt, TemporaryHomeError> {
    validate_transition(input, 1)?;
    let sid = profile.sid_identity()?;
    let request_hex = transition_fingerprint("temporary-close", input, &sid);
    transaction(connection, |connection| {
        let row = home_row(connection, input.home_id)?.ok_or(TemporaryHomeError::HomeConflict)?;
        row_identity(&row, input.home_id)?;
        if let Some((stored, target, phase, stored_receipt)) = transition_operation(connection, input.request_id)? {
            if stored != request_hex || target != input.home_id { return Err(TemporaryHomeError::RequestConflict); }
            if phase == "APPLIED" && matches!(row.7.as_str(), "CLOSED" | "CLEANUP_UNKNOWN" | "CLEANED") {
                let stop = exact_stopped_home(connection, input.home_id, &row)?;
                if stored_receipt != format!("\"{}\"", hex(stop.as_bytes())) {
                    return Err(TemporaryHomeError::Unknown);
                }
                if row.7 == "CLEANED" {
                    if transition_stage(connection, root, input.home_id, &row, &sid)?.is_some() {
                        return Err(TemporaryHomeError::Unknown);
                    }
                } else if !matches!(transition_stage(connection, root, input.home_id, &row, &sid)?,
                    Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == row.6) {
                    return Err(TemporaryHomeError::Unknown);
                }
                return Ok(receipt("REPLAYED", &row));
            }
            return Err(TemporaryHomeError::Unknown);
        }
        if row.7 != "ACTIVE" || row.8 != "1" { return Err(TemporaryHomeError::StaleRevision); }
        let stop = exact_stopped_home(connection, input.home_id, &row)?;
        if !matches!(transition_stage(connection, root, input.home_id, &row, &sid)?,
            Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == row.6) {
            return Err(TemporaryHomeError::IdentityChanged);
        }
        let op = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,receipt_json) VALUES(?1,?2,?3,'APPLIED',?4)")?;
        op.bind_text(1, input.request_id)?; op.bind_text(2, &request_hex)?;
        op.bind_text(3, input.home_id)?; op.bind_text(4, &format!("\"{}\"", hex(stop.as_bytes())))?;
        op.step_done()?;
        let update = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_homes SET state='CLOSED',revision=2 WHERE home_id=?1 AND state='ACTIVE' AND revision=1")?;
        update.bind_text(1, input.home_id)?; update.step_done()?;
        Ok(receipt("APPLIED", &row))
    })
}

fn cleanup_receipt(root: &RootLock, parent: &RootIdentity, home_id: &str,
    row: &HomeRow, sid: &str, stop: &str, fence: &str, marker: &str) -> String {
    let mut framed = Vec::new();
    let root_identity = root.canonical_root().identity.opaque();
    let parent_identity = parent.opaque();
    let marker_hash = sha256_hex(marker.as_bytes());
    for value in [&b"temporary-cleanup-v1"[..], root_identity.as_bytes(),
        parent_identity.as_bytes(), home_id.as_bytes(), row.6.as_bytes(), sid.as_bytes(),
        stop.as_bytes(), fence.as_bytes(), marker_hash.as_bytes()] {
        framed.extend_from_slice(&(value.len() as u64).to_be_bytes());
        framed.extend_from_slice(value);
    }
    format!("\"{}\"", hex(&framed))
}
fn remove_content_preserving_marker(path: &Path) -> Result<(), TemporaryHomeError> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_name().as_os_str() == std::ffi::OsStr::new(MARKER) { continue; }
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child)?;
        if metadata.file_attributes() & REPARSE_POINT != 0 {
            if metadata.file_attributes() & 0x10 != 0 { fs::remove_dir(&child)?; }
            else { fs::remove_file(&child)?; }
        } else if metadata.is_dir() { fs::remove_dir_all(&child)?; }
        else { fs::remove_file(&child)?; }
    }
    Ok(())
}

pub(crate) fn cleanup_temporary_home(connection: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, profile: &AppContainerProfile, input: &TransitionTemporaryHome<'_>)
    -> Result<TemporaryHomeReceipt, TemporaryHomeError> {
    validate_transition(input, 2)?;
    let sid = profile.sid_identity()?;
    let request_hex = transition_fingerprint("temporary-cleanup", input, &sid);
    let prepared = transaction(connection, |connection| {
        let mut row = home_row(connection, input.home_id)?.ok_or(TemporaryHomeError::HomeConflict)?;
        row_identity(&row, input.home_id)?;
        let prior = transition_operation(connection, input.request_id)?;
        if let Some((stored,target,_,_)) = &prior {
            if stored != &request_hex || target != input.home_id { return Err(TemporaryHomeError::RequestConflict); }
        }
        let stop = exact_stopped_home(connection, input.home_id, &row)?;
        let parent = transition_parent(root, &row)?;
        let marker = create_marker_for_row(connection, root, input.home_id, &row, &sid)?;
        if prior.is_none() {
            if row.7 != "CLOSED" || row.8 != "2" { return Err(TemporaryHomeError::StaleRevision); }
            if !matches!(transition_stage(connection, root, input.home_id, &row, &sid)?,
                Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == row.6) {
                return Err(TemporaryHomeError::IdentityChanged);
            }
        } else if !matches!(row.7.as_str(), "CLEANUP_UNKNOWN" | "CLEANED") {
            return Err(TemporaryHomeError::Unknown);
        }
        let fence = fence_home_admission_in_transaction(connection, input.home_id,
            &row.0, &row.1, &row.2, &row.3, &row.4)?;
        let expected = cleanup_receipt(root, &parent, input.home_id, &row, &sid,
            &stop, &fence, &marker);
        if let Some((_,_,phase,stored_receipt)) = prior {
            if stored_receipt != expected { return Err(TemporaryHomeError::Unknown); }
            if phase == "APPLIED" && row.7 == "CLEANED" {
                if transition_stage(connection, root, input.home_id, &row, &sid)?.is_some() {
                    return Err(TemporaryHomeError::Unknown);
                }
                return Ok((row, parent, expected, true));
            }
            if phase != "PREPARING" && phase != "UNKNOWN" || row.7 != "CLEANUP_UNKNOWN" || row.8 != "3" {
                return Err(TemporaryHomeError::Unknown);
            }
        } else {
            let op = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,receipt_json) VALUES(?1,?2,?3,'PREPARING',?4)")?;
            op.bind_text(1, input.request_id)?; op.bind_text(2, &request_hex)?;
            op.bind_text(3, input.home_id)?; op.bind_text(4, &expected)?; op.step_done()?;
            let update = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_instance_homes SET state='CLEANUP_UNKNOWN',revision=3 WHERE home_id=?1 AND state='CLOSED' AND revision=2")?;
            update.bind_text(1, input.home_id)?; update.step_done()?;
            row.7 = "CLEANUP_UNKNOWN".to_owned();
            row.8 = "3".to_owned();
        }
        Ok((row, parent, expected, false))
    })?;
    let (row, parent, expected, replayed) = prepared;
    if replayed { return Ok(receipt("REPLAYED", &row)); }
    let path = transition_path(root, &row.0, input.home_id);
    if transition_parent(root, &row)? != parent { return Err(TemporaryHomeError::IdentityChanged); }
    match transition_stage(connection, root, input.home_id, &row, &sid)? {
        Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == row.6 => {
            remove_content_preserving_marker(&path)?;
            if transition_parent(root, &row)? != parent { return Err(TemporaryHomeError::IdentityChanged); }
            if transition_stage(connection, root, input.home_id, &row, &sid)?
                != Some(DirectoryStage::Marked(identity.clone(), false)) {
                return Err(TemporaryHomeError::Unknown);
            }
            fs::remove_file(path.join(MARKER))?;
            if transition_parent(root, &row)? != parent { return Err(TemporaryHomeError::IdentityChanged); }
            if transition_stage(connection, root, input.home_id, &row, &sid)?
                != Some(DirectoryStage::Empty(identity.clone())) {
                return Err(TemporaryHomeError::Unknown);
            }
            fs::remove_dir(&path)?;
        }
        Some(DirectoryStage::Empty(identity)) if identity.opaque() == row.6 => fs::remove_dir(&path)?,
        None => (),
        Some(_) => return Err(TemporaryHomeError::Unknown),
    }
    transaction(connection, |connection| {
        let current = home_row(connection, input.home_id)?.ok_or(TemporaryHomeError::Unknown)?;
        if current != row || current.7 != "CLEANUP_UNKNOWN" || current.8 != "3" {
            return Err(TemporaryHomeError::Unknown);
        }
        let Some((stored,target,phase,stored_receipt)) = transition_operation(connection, input.request_id)? else {
            return Err(TemporaryHomeError::Unknown);
        };
        if stored != request_hex || target != input.home_id || stored_receipt != expected
            || (phase != "PREPARING" && phase != "UNKNOWN") { return Err(TemporaryHomeError::Unknown); }
        let stop = exact_stopped_home(connection, input.home_id, &current)?;
        let fence = fence_home_admission_in_transaction(connection, input.home_id,
            &current.0, &current.1, &current.2, &current.3, &current.4)?;
        let parent = transition_parent(root, &current)?;
        let marker = create_marker_for_row(connection, root, input.home_id, &current, &sid)?;
        if cleanup_receipt(root, &parent, input.home_id, &current, &sid, &stop, &fence, &marker) != expected
            || transition_stage(connection, root, input.home_id, &current, &sid)?.is_some() {
            return Err(TemporaryHomeError::Unknown);
        }
        let update = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_homes SET state='CLEANED',revision=4 WHERE home_id=?1 AND state='CLEANUP_UNKNOWN' AND revision=3")?;
        update.bind_text(1, input.home_id)?; update.step_done()?;
        let op = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_operations SET phase='APPLIED' WHERE request_id=?1 AND phase IN ('PREPARING','UNKNOWN')")?;
        op.bind_text(1, input.request_id)?; op.step_done()?;
        Ok(receipt("APPLIED", &current))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::instance::initialize_schema;
    use crate::store::instance::registry::{register_instance, ProgramObservation, Registration};
    use crate::store::same_open::{create_new, route_b_test_guard};
    use crate::store::authority::initialize_process_custody_schema;
    use crate::store::session_transport::{bind_owner_in_transaction, initialize_admission_schema,
        release_admission, AdmissionRequest, AdmissionResult, OwnerBinding};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &RootLock, &AppContainerProfile)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root_path = std::env::temp_dir().join(format!("gogoke-v37-temp-home-{}-{nonce}", std::process::id()));
        fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        initialize_schema(&mut connection).unwrap();
        initialize_admission_schema(&mut connection).unwrap();
        initialize_process_custody_schema(&mut connection).unwrap();
        let program_path = root_path.join("test-program.bin");
        fs::write(&program_path, b"fixture program bytes").unwrap();
        let program = ProgramObservation::observe(&program_path, "0.160.0").unwrap();
        register_instance(&mut connection, &root, &Registration {
            request_id: "instanceReg", request_bytes: b"register instance", instance_id: "instanceA",
            driver_id: "codex", program: &program,
        }).unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        bind_owner_in_transaction(&mut connection, &OwnerBinding {
            binding_id: "bindingA", instance_id: "instanceA", domain_id: "projectA",
            kind: "SESSION", owner_id: "sessionA", generation: "1",
        }).unwrap();
        connection.execute("COMMIT").unwrap();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.FTemporaryHome").unwrap();
        run(&mut connection, &root, &profile);
        connection.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(root_path).unwrap();
    }

    fn input<'a>(bytes: &'a [u8]) -> CreateTemporaryHome<'a> {
        CreateTemporaryHome { request_id: "tempCreate", request_bytes: bytes, home_id: "tempA",
            instance_id: "instanceA", domain_id: "projectA", kind: TemporaryKind::Session,
            owner_id: "sessionA", generation: "1" }
    }

    fn stopped_claim(connection: &mut VerifiedDatabaseConnection<'_>, home_id: &str) {
        // Synthetic same-schema H history for this F stop-proof control. The
        // original episode home comes from the F row actually created by the
        // fixture; `home_id` may deliberately give the current claim a wrong
        // home in the negative control below. No process/vendor ran here.
        let original = Statement::prepare(connection.as_ptr(),
            "SELECT home_id FROM main.gogoke_v37_instance_homes WHERE instance_id='instanceA'
               AND domain_id='projectA' AND kind='SESSION' AND owner_id='sessionA'
               AND generation='1' AND state='ACTIVE'").unwrap();
        assert!(original.step_row().unwrap());
        let original_home = original.column_text(0).unwrap();
        assert!(!original.step_row().unwrap());
        drop(original);
        connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('processA','ticketA','nonceA','11','1','fixture-program','sha256:fixture','profileA','projectA','1','STOPPED','proofA')").unwrap();
        let claim = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('projectA','sessionA','instanceA',?1,'bindingA','1','STOPPED',1,'processA','proofA')").unwrap();
        claim.bind_text(1, home_id).unwrap();
        claim.step_done().unwrap();
        drop(claim);
        let episode = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,
                generation,old_generation,raw_hex,previous_revision,result_revision,
                process_operation_id,instance_id,home_id,binding_id,phase,stop_fact_id)
             VALUES('projectA','openA','sessionA','1',NULL,'6f70656e2d66697874757265',0,1,
                'processA','instanceA',?1,'bindingA','STOPPED','proofA')").unwrap();
        episode.bind_text(1,&original_home).unwrap();
        episode.step_done().unwrap();
        connection.execute("INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,
            generation,request_id,process_operation_id)
            VALUES('projectA','sessionA','1','openA','processA')").unwrap();
    }

    fn release_stopped_claim(connection: &mut VerifiedDatabaseConnection<'_>) {
        let request = AdmissionRequest { domain_id: "projectA", session_id: "sessionA",
            request_id: "releaseA", raw_bytes: b"release stopped claim", instance_id: "instanceA",
            home_id: "tempA", generation: "1", expected_revision: 1 };
        assert_eq!(release_admission(connection, &request, |_| Ok(())).unwrap(), AdmissionResult::Applied(2));
    }

    fn synthetic_successful_resume(connection: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, profile: &AppContainerProfile) {
        // Same-schema durable H rows, not evidence that a native process ran.
        connection.execute("BEGIN IMMEDIATE").unwrap();
        bind_owner_in_transaction(connection, &OwnerBinding {
            binding_id: "bindingB", instance_id: "instanceA", domain_id: "projectA",
            kind: "SESSION", owner_id: "sessionA", generation: "2",
        }).unwrap();
        connection.execute("COMMIT").unwrap();
        let next = CreateTemporaryHome { request_id: "tempCreateB", request_bytes: b"new home",
            home_id: "tempB", instance_id: "instanceA", domain_id: "projectA",
            kind: TemporaryKind::Session, owner_id: "sessionA", generation: "2" };
        create_temporary_home(connection, root, profile, &next).unwrap();
        connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('processB','ticketB','nonceB','12','2','fixture-program','sha256:fixture','profileA','projectA','2','ACTIVE')").unwrap();
        connection.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,phase) VALUES('projectA','resumeB','sessionA','2','1','726573756d652d66697874757265',1,2,'processB','instanceA','tempB','bindingB','ACTIVE')").unwrap();
        connection.execute("UPDATE main.gogoke_v37_h_claim SET generation='2',home_id='tempB',binding_id='bindingB',process_operation_id='processB',stop_fact_id=NULL,state='COMMITTED',revision=2 WHERE domain_id='projectA' AND session_id='sessionA' AND generation='1' AND state='STOPPED'").unwrap();
    }

    #[test]
    fn create_is_durable_and_exact_request_replay_conflicts_on_changed_bytes() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            let applied = create_temporary_home(connection, root, profile, &first).unwrap();
            assert_eq!(applied.disposition, "APPLIED");
            assert_eq!(applied.directory_ref, directory_ref(first.home_id));
            assert_eq!(create_temporary_home(connection, root, profile, &first).unwrap().disposition, "REPLAYED");
            assert!(matches!(create_temporary_home(connection, root, profile, &input(b"create changed")),
                Err(TemporaryHomeError::RequestConflict)));
            fs::write(directory(root, &first).join("session-content"), b"later content").unwrap();
            assert_eq!(create_temporary_home(connection, root, profile, &first).unwrap().disposition, "REPLAYED");
            let transition = TransitionTemporaryHome { request_id: "closeOne", request_bytes: b"close",
                home_id: "tempA", expected_revision: 1 };
            assert!(matches!(close_temporary_home(connection, root, profile, &transition),
                Err(TemporaryHomeError::Admission(AdmissionError::Denied))));
            assert!(matches!(directory_stage(root, &first, &profile.sid_identity().unwrap()).unwrap(),
                Some(DirectoryStage::Marked(_, true))));
        });
    }

    #[test]
    fn legacy_create_intent_is_not_reinterpreted_in_detached_layout() {
        fixture(|connection, root, profile| {
            let first = input(b"legacy create");
            let sid = profile.sid_identity().unwrap();
            let current = fingerprint(&first, &sid);
            let current_tag = b"temporary-create-detached";
            let legacy_tag = b"temporary-create";
            let current_prefix = format!("{:016x}{}", current_tag.len(), hex(current_tag));
            let legacy = format!("{:016x}{}{}", legacy_tag.len(), hex(legacy_tag),
                current.strip_prefix(&current_prefix).expect("versioned creation fingerprint"));
            let intent = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'UNKNOWN')").unwrap();
            intent.bind_text(1, first.request_id).unwrap();
            intent.bind_text(2, &legacy).unwrap();
            intent.bind_text(3, first.home_id).unwrap();
            intent.step_done().unwrap();
            assert!(matches!(create_temporary_home(connection, root, profile, &first),
                Err(TemporaryHomeError::RequestConflict)));
            assert!(!root.canonical_root().canonical_path.join(TEMPORARY_CONTAINER).exists(),
                "an old request cannot authorize a new physical location");
            assert_eq!(operation(connection, first.request_id).unwrap().unwrap().2, "UNKNOWN");
        });
    }

    #[test]
    fn preparing_intent_recovers_bound_directory_before_receipt() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            let home = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES(?1,?2,?3,?4,?5,?6,'PREPARING',0)").unwrap();
            for (index, value) in [first.home_id,first.instance_id,first.domain_id,first.kind.as_str(),
                first.owner_id,first.generation].iter().enumerate() {
                home.bind_text((index+1) as i32, value).unwrap();
            }
            home.step_done().unwrap();
            let intent = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'UNKNOWN')").unwrap();
            intent.bind_text(1, first.request_id).unwrap();
            let sid = profile.sid_identity().unwrap();
            intent.bind_text(2, &fingerprint(&first, &sid)).unwrap();
            intent.bind_text(3, first.home_id).unwrap();
            intent.step_done().unwrap();
            let identity = prepare_empty_directory(root, &first, &sid).unwrap();
            assert_eq!(directory_stage(root, &first, &sid).unwrap(), Some(DirectoryStage::Empty(identity.clone())));
            let other = AppContainerProfile::derived_for_test("Gogoke37.FTemporaryOther").unwrap();
            assert!(matches!(create_temporary_home(connection, root, &other, &first),
                Err(TemporaryHomeError::RequestConflict)));
            assert_eq!(create_temporary_home(connection, root, profile, &first).unwrap().disposition, "APPLIED");
            assert_eq!(directory_stage(root, &first, &sid).unwrap(), Some(DirectoryStage::Marked(identity, false)));
        });
    }

    #[test]
    fn existing_directory_is_not_adopted_without_prior_intent() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            prepare_empty_directory(root, &first, &profile.sid_identity().unwrap()).unwrap();
            assert!(matches!(create_temporary_home(connection, root, profile, &first),
                Err(TemporaryHomeError::Unknown)));
            assert!(operation(connection, first.request_id).unwrap().is_none());
            assert!(home_row(connection, first.home_id).unwrap().is_none());
        });
    }

    #[test]
    fn nonempty_directory_before_acl_grant_stays_unknown() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            let sid = profile.sid_identity().unwrap();
            let home = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_homes(home_id,instance_id,domain_id,kind,owner_id,generation,state,revision) VALUES(?1,?2,?3,?4,?5,?6,'PREPARING',0)").unwrap();
            for (index, value) in [first.home_id,first.instance_id,first.domain_id,first.kind.as_str(),
                first.owner_id,first.generation].iter().enumerate() {
                home.bind_text((index+1) as i32, value).unwrap();
            }
            home.step_done().unwrap();
            let intent = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'PREPARING')").unwrap();
            intent.bind_text(1, first.request_id).unwrap();
            intent.bind_text(2, &fingerprint(&first, &sid)).unwrap();
            intent.bind_text(3, first.home_id).unwrap();
            intent.step_done().unwrap();
            prepare_empty_directory(root, &first, &sid).unwrap();
            fs::write(directory(root, &first).join("foreign-content"), b"no grant yet").unwrap();
            assert!(matches!(create_temporary_home(connection, root, profile, &first),
                Err(TemporaryHomeError::Unknown)));
        });
    }

    #[test]
    fn revoked_owner_binding_rejects_create_before_intent() {
        fixture(|connection, root, profile| {
            connection.execute("UPDATE main.gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='bindingA'").unwrap();
            let first = input(b"create one");
            assert!(matches!(create_temporary_home(connection, root, profile, &first),
                Err(TemporaryHomeError::Admission(AdmissionError::Denied))));
            assert!(operation(connection, first.request_id).unwrap().is_none());
            assert!(home_row(connection, first.home_id).unwrap().is_none());
        });
    }

    #[test]
    fn close_and_cleanup_require_exact_stop_release_and_preserve_replay() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            create_temporary_home(connection, root, profile, &first).unwrap();
            let path = directory(root, &first);
            fs::create_dir(path.join("session-data")).unwrap();
            fs::write(path.join("session-data").join("output"), b"temporary session content").unwrap();
            stopped_claim(connection, "differentHome");
            let close = TransitionTemporaryHome { request_id: "closeA", request_bytes: b"close exact",
                home_id: first.home_id, expected_revision: 1 };
            connection.execute("DELETE FROM main.gogoke_v37_h_generation WHERE domain_id='projectA' AND session_id='sessionA' AND generation='1'").unwrap();
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::Admission(AdmissionError::Denied))),
                "a stopped custody row without its original H generation is not a stop receipt");
            connection.execute("INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openA','processA')").unwrap();
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            connection.execute("UPDATE main.gogoke_v37_h_claim SET home_id='tempA' WHERE session_id='sessionA'").unwrap();
            assert_eq!(close_temporary_home(connection, root, profile, &close).unwrap().disposition, "APPLIED");
            assert_eq!(close_temporary_home(connection, root, profile, &close).unwrap().disposition, "REPLAYED");
            let changed = TransitionTemporaryHome { request_bytes: b"close changed", ..close };
            assert!(matches!(close_temporary_home(connection, root, profile, &changed),
                Err(TemporaryHomeError::RequestConflict)));
            let cleanup = TransitionTemporaryHome { request_id: "cleanupA", request_bytes: b"cleanup exact",
                home_id: first.home_id, expected_revision: 2 };
            assert!(matches!(cleanup_temporary_home(connection, root, profile, &cleanup),
                Err(TemporaryHomeError::Admission(AdmissionError::Denied))));
            assert!(path.exists());
            release_stopped_claim(connection);
            assert_eq!(cleanup_temporary_home(connection, root, profile, &cleanup).unwrap().disposition, "APPLIED");
            assert!(!path.exists());
            assert_eq!(cleanup_temporary_home(connection, root, profile, &cleanup).unwrap().disposition, "REPLAYED");
            let row = home_row(connection, first.home_id).unwrap().unwrap();
            assert_eq!((&row.7[..], &row.8[..]), ("CLEANED", "4"));
        });
    }

    #[test]
    fn stopped_original_home_survives_successful_generation_rollover() {
        fixture(|connection, root, profile| {
            let first = input(b"old home");
            create_temporary_home(connection, root, profile, &first).unwrap();
            let old_path = directory(root, &first);
            fs::write(old_path.join("session-content"), b"old data").unwrap();
            stopped_claim(connection, first.home_id);
            let close = TransitionTemporaryHome { request_id: "closeOld",
                request_bytes: b"close old", home_id: first.home_id, expected_revision: 1 };
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET home_id='wrongOriginalHome' WHERE process_operation_id='processA'").unwrap();
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET home_id='tempA' WHERE process_operation_id='processA'").unwrap();
            connection.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='wrongProof' WHERE operation_id='processA'").unwrap();
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::Admission(AdmissionError::Denied))));
            connection.execute("UPDATE main.gogoke_coordination_process_custody SET stop_proof_hash='proofA' WHERE operation_id='processA'").unwrap();
            connection.execute("DELETE FROM main.gogoke_coordination_process_custody WHERE operation_id='processA'").unwrap();
            connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('otherProcess','wrongTicket','otherNonce','11','1','fixture-program','sha256:fixture','profileA','projectA','1','STOPPED','proofA')").unwrap();
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::Admission(AdmissionError::Denied))),
                "another ticket's custody cannot replace the original process");
            connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('processA','ticketA','nonceA','11','1','fixture-program','sha256:fixture','profileA','projectA','1','STOPPED','proofA')").unwrap();
            synthetic_successful_resume(connection, root, profile);
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::StopFactUnavailable)),
                "a current claim in a new generation needs its successful mapping");
            connection.execute("INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','2','resumeB','processB')").unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET old_generation='unrelated' WHERE process_operation_id='processB'").unwrap();
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::StopFactUnavailable)),
                "a mapped but unrelated generation cannot authorize the old home");
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET old_generation='1' WHERE process_operation_id='processB'").unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_owner_binding SET state='REVOKED' WHERE binding_id='bindingA'").unwrap();
            assert_eq!(close_temporary_home(connection, root, profile, &close).unwrap().disposition, "APPLIED");
            assert_eq!(close_temporary_home(connection, root, profile, &close).unwrap().disposition, "REPLAYED");
            let cleanup = TransitionTemporaryHome { request_id: "cleanupOld",
                request_bytes: b"cleanup old", home_id: first.home_id, expected_revision: 2 };
            assert_eq!(cleanup_temporary_home(connection, root, profile, &cleanup).unwrap().disposition, "APPLIED");
            assert!(!old_path.exists());
            assert_eq!(cleanup_temporary_home(connection, root, profile, &cleanup).unwrap().disposition, "REPLAYED");
            assert_eq!(close_temporary_home(connection, root, profile, &close).unwrap().disposition, "REPLAYED");
            assert_eq!(home_row(connection, "tempB").unwrap().unwrap().7, "ACTIVE");
        });
    }

    #[test]
    fn cleanup_recovers_after_marker_removed_but_before_final_receipt() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            create_temporary_home(connection, root, profile, &first).unwrap();
            stopped_claim(connection, first.home_id);
            let close = TransitionTemporaryHome { request_id: "closeA", request_bytes: b"close exact",
                home_id: first.home_id, expected_revision: 1 };
            close_temporary_home(connection, root, profile, &close).unwrap();
            release_stopped_claim(connection);
            let cleanup = TransitionTemporaryHome { request_id: "cleanupA", request_bytes: b"cleanup exact",
                home_id: first.home_id, expected_revision: 2 };
            let sid = profile.sid_identity().unwrap();
            transaction(connection, |connection| {
                let row = home_row(connection, first.home_id)?.unwrap();
                let stop = exact_stopped_home(connection, first.home_id, &row)?;
                let parent = transition_parent(root, &row)?;
                let marker = create_marker_for_row(connection, root, first.home_id, &row, &sid)?;
                let fence = fence_home_admission_in_transaction(connection, first.home_id,
                    &row.0, &row.1, &row.2, &row.3, &row.4)?;
                let expected = cleanup_receipt(root, &parent, first.home_id, &row, &sid,
                    &stop, &fence, &marker);
                let op = Statement::prepare(connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,receipt_json) VALUES(?1,?2,?3,'PREPARING',?4)")?;
                op.bind_text(1, cleanup.request_id)?;
                op.bind_text(2, &transition_fingerprint("temporary-cleanup", &cleanup, &sid))?;
                op.bind_text(3, cleanup.home_id)?; op.bind_text(4, &expected)?; op.step_done()?;
                connection.execute("UPDATE main.gogoke_v37_instance_homes SET state='CLEANUP_UNKNOWN',revision=3 WHERE home_id='tempA'")?;
                Ok(())
            }).unwrap();
            let path = directory(root, &first);
            fs::remove_file(path.join(MARKER)).unwrap();
            assert_eq!(cleanup_temporary_home(connection, root, profile, &cleanup).unwrap().disposition, "APPLIED");
            assert!(!path.exists());
        });
    }

    #[test]
    fn call_home_close_stays_unknown_without_native_completion_fact() {
        fixture(|connection, root, profile| {
            connection.execute("BEGIN IMMEDIATE").unwrap();
            bind_owner_in_transaction(connection, &OwnerBinding {
                binding_id: "bindingCall", instance_id: "instanceA", domain_id: "projectA",
                kind: "CALL", owner_id: "callA", generation: "1",
            }).unwrap();
            connection.execute("COMMIT").unwrap();
            let call = CreateTemporaryHome { request_id: "callCreate", request_bytes: b"call create",
                home_id: "callA", instance_id: "instanceA", domain_id: "projectA",
                kind: TemporaryKind::Call, owner_id: "callA", generation: "1" };
            create_temporary_home(connection, root, profile, &call).unwrap();
            let close = TransitionTemporaryHome { request_id: "callClose", request_bytes: b"call close",
                home_id: "callA", expected_revision: 1 };
            assert!(matches!(close_temporary_home(connection, root, profile, &close),
                Err(TemporaryHomeError::Admission(AdmissionError::Unknown))));
            assert!(directory(root, &call).exists());
        });
    }

    #[test]
    fn changed_marker_blocks_cleanup_before_fence_or_delete() {
        fixture(|connection, root, profile| {
            let first = input(b"create one");
            create_temporary_home(connection, root, profile, &first).unwrap();
            stopped_claim(connection, first.home_id);
            let close = TransitionTemporaryHome { request_id: "closeA", request_bytes: b"close exact",
                home_id: first.home_id, expected_revision: 1 };
            close_temporary_home(connection, root, profile, &close).unwrap();
            release_stopped_claim(connection);
            fs::write(directory(root, &first).join(MARKER), b"changed marker").unwrap();
            let cleanup = TransitionTemporaryHome { request_id: "cleanupA", request_bytes: b"cleanup exact",
                home_id: first.home_id, expected_revision: 2 };
            assert!(matches!(cleanup_temporary_home(connection, root, profile, &cleanup),
                Err(TemporaryHomeError::IdentityChanged)));
            let fence = Statement::prepare(connection.as_ptr(),
                "SELECT COUNT(*) FROM main.gogoke_v37_h_home_fence WHERE home_id='tempA'").unwrap();
            assert!(fence.step_row().unwrap());
            assert_eq!(fence.column_text(0).unwrap(), "0");
            assert!(directory(root, &first).exists());
        });
    }
}
