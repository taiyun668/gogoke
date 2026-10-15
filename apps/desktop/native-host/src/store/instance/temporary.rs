//! F.1 temporary SESSION/CALL home creation. H's durable stop and admission
//! fence are not yet wired: close and cleanup fail closed and never delete.

use super::registry::{observed_home, RegistryError};
use crate::process::{AppContainerProfile, IsolationError};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::{AtomicError, Json, JsonString, Parser, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
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
    Owner(crate::store::orchestration::OrchestrationError),
    ExactGone(crate::store::session_transport::ExactHomeGoneError),
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
impl From<crate::store::session_transport::ExactHomeGoneError> for TemporaryHomeError {
    fn from(error: crate::store::session_transport::ExactHomeGoneError) -> Self { Self::ExactGone(error) }
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

/// Read the original SESSION root of an already recorded H episode. This is
/// physical provenance, not a current admission check: a historical home may
/// have been closed, and its binding may belong to an older H generation.
/// Missing or partly cleaned roots cannot prove disjointness.
pub(super) fn resolve_historical_session_home(
    connection: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    profile: &AppContainerProfile, instance_id: &str, home_id: &str,
    domain_id: &str, owner_id: &str, generation: &str, binding_id: &str,
) -> Result<(std::path::PathBuf, RootIdentity), TemporaryHomeError> {
    for (name, value) in [("instance_id", instance_id), ("home_id", home_id),
        ("domain_id", domain_id), ("owner_id", owner_id), ("binding_id", binding_id)] {
        if !valid_id(value) { return Err(TemporaryHomeError::Invalid(name)); }
    }
    if !valid_generation(generation) { return Err(TemporaryHomeError::Invalid("generation")); }
    let row = home_row(connection, home_id)?.ok_or(TemporaryHomeError::Unknown)?;
    if row.0 != instance_id || row.1 != domain_id || row.2 != "SESSION"
        || row.3 != owner_id || row.4 != generation
        || !matches!(row.7.as_str(), "ACTIVE" | "CLOSED") {
        return Err(TemporaryHomeError::Unknown);
    }
    row_identity(&row, home_id)?;
    let persistent = observed_home(root, instance_id)?.ok_or(TemporaryHomeError::InstanceChanged)?;
    if instance_identity(connection, instance_id)?.as_deref() != Some(persistent.opaque().as_str()) {
        return Err(TemporaryHomeError::InstanceChanged);
    }
    let binding = Statement::prepare(connection.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_owner_binding WHERE binding_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION' AND owner_id=?4 AND generation=?5 AND state='ACTIVE'")?;
    for (index, value) in [binding_id,instance_id,domain_id,owner_id,generation].iter().enumerate() {
        binding.bind_text((index+1) as i32,value)?;
    }
    if !binding.step_row()? || binding.step_row()? { return Err(TemporaryHomeError::Unknown); }
    let sid = profile.sid_identity()?;
    let path = transition_path(root, instance_id, home_id);
    let identity = match transition_stage(connection, root, home_id, &row, &sid)? {
        Some(DirectoryStage::Marked(identity, _)) => identity,
        Some(DirectoryStage::Empty(_)) | None => return Err(TemporaryHomeError::Unknown),
    };
    if identity.opaque() != row.6 { return Err(TemporaryHomeError::IdentityChanged); }
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

/// The authenticated USER bridge selects exactly one lookup key. Neither key
/// is proof: the F/H/C original producer chain is reread below.
#[derive(Clone, Copy)]
pub(crate) enum UserTemporaryHomeSelector<'a> {
    OriginalCreateRequestId(&'a str),
    HistoricalHomeId(&'a str),
}
pub(crate) struct UserTemporaryHomeCloseInput<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) instance_id: &'a str,
    pub(crate) domain_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) expected_revision: i64,
    pub(crate) selector: UserTemporaryHomeSelector<'a>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UserTemporaryHomeCloseReceipt {
    pub(crate) disposition: &'static str,
    pub(crate) home_id: String,
    pub(crate) directory_ref: String,
    pub(crate) native_receipt_id: String,
    pub(crate) proof_kind: &'static str,
    pub(crate) source_fingerprint: String,
    pub(crate) before_revision: i64,
    pub(crate) after_revision: i64,
}

fn decode_lower_hex(value: &str) -> Result<Vec<u8>, TemporaryHomeError> {
    if value.is_empty() || value.len() % 2 != 0 || value.len() > 262_144
        || !value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return Err(TemporaryHomeError::Invalid("original create fingerprint"));
    }
    value.as_bytes().chunks_exact(2).map(|pair| {
        let digit = |byte: u8| if byte.is_ascii_digit() { byte - b'0' } else { byte - b'a' + 10 };
        Ok((digit(pair[0]) << 4) | digit(pair[1]))
    }).collect()
}

fn create_request_from_fingerprint(value: &str) -> Result<Vec<u8>, TemporaryHomeError> {
    let framed = decode_lower_hex(value)?;
    let mut offset = 0usize;
    let mut parts = Vec::with_capacity(9);
    for _ in 0..9 {
        let length_end = offset.checked_add(8).ok_or(TemporaryHomeError::Unknown)?;
        let length_bytes: [u8; 8] = framed.get(offset..length_end)
            .ok_or(TemporaryHomeError::Unknown)?.try_into()
            .map_err(|_| TemporaryHomeError::Unknown)?;
        let length = usize::try_from(u64::from_be_bytes(length_bytes))
            .map_err(|_| TemporaryHomeError::Unknown)?;
        offset = length_end;
        let end = offset.checked_add(length).ok_or(TemporaryHomeError::Unknown)?;
        parts.push(framed.get(offset..end).ok_or(TemporaryHomeError::Unknown)?);
        offset = end;
    }
    if offset != framed.len() || parts[0] != b"temporary-create-detached"
        || parts[1].is_empty() || parts[1].len() > 65_536 {
        return Err(TemporaryHomeError::Unknown);
    }
    Ok(parts[1].to_vec())
}

fn original_create_source(connection: &VerifiedDatabaseConnection<'_>, home_id: &str,
    row: &HomeRow, selector: &UserTemporaryHomeSelector<'_>, sid: &str)
    -> Result<(String, String, String, String, String), TemporaryHomeError> {
    let tag = b"temporary-create-detached";
    let prefix = format!("{:016x}{}", tag.len(), hex(tag));
    let query = Statement::prepare(connection.as_ptr(),
        "SELECT request_id,request_hex,phase FROM main.gogoke_v37_instance_operations
          WHERE target_id=?1")?;
    query.bind_text(1, home_id)?;
    let mut source = None;
    while query.step_row()? {
        let id = query.column_text(0)?;
        let fingerprint = query.column_text(1)?;
        let phase = query.column_text(2)?;
        if fingerprint.starts_with(&prefix) {
            if source.replace((id, fingerprint, phase)).is_some() {
                return Err(TemporaryHomeError::HomeConflict);
            }
        }
    }
    let (operation_id, stored, phase) = source.ok_or(TemporaryHomeError::Unknown)?;
    if phase != "APPLIED" { return Err(TemporaryHomeError::Unknown); }
    let raw = create_request_from_fingerprint(&stored)?;
    let request = crate::store::session_transport::decode_request(&raw)
        .map_err(|_| TemporaryHomeError::Invalid("original create USER raw"))?;
    let digest = sha256_hex(format!("{}\n{}", row.1, request.request_id).as_bytes());
    if operation_id != format!("homeprep-{}", &digest[..40])
        || request.family != "K-SESSION" || request.domain_id != row.1
        || request.target_id != row.3
        || !matches!(request.operation.as_str(), "admission-reserve" | "open" | "resume" | "compact" | "renew-session") {
        return Err(TemporaryHomeError::RequestConflict);
    }
    match selector {
        UserTemporaryHomeSelector::OriginalCreateRequestId(id) if *id == request.request_id.as_str() => {},
        UserTemporaryHomeSelector::HistoricalHomeId(id) if *id == home_id => {},
        _ => return Err(TemporaryHomeError::RequestConflict),
    }
    let create = CreateTemporaryHome { request_id: &operation_id, request_bytes: &raw,
        home_id, instance_id: &row.0, domain_id: &row.1, kind: TemporaryKind::Session,
        owner_id: &row.3, generation: &row.4 };
    if fingerprint(&create, sid) != stored { return Err(TemporaryHomeError::RequestConflict); }
    // The F fingerprint embeds raw bytes, but each USER creation operation has
    // a different direct H producer shape. No union/cardinality guess permits
    // an unrelated operation to substitute for that producer.
    let original_hex = hex(&raw);
    let h_operation = Statement::prepare(connection.as_ptr(),
        "SELECT raw_hex,status FROM main.gogoke_v37_h_operation
          WHERE domain_id=?1 AND request_id=?2 AND session_id=?3 AND operation=?4")?;
    for (index, value) in [row.1.as_str(), request.request_id.as_str(), row.3.as_str(),
        request.operation.as_str()].iter().enumerate() {
        h_operation.bind_text(index as i32 + 1, value)?;
    }
    let operation_source = if h_operation.step_row()? {
        let found = (h_operation.column_text(0)?, h_operation.column_text(1)?);
        if h_operation.step_row()? { return Err(TemporaryHomeError::HomeConflict); }
        Some(found)
    } else { None };
    drop(h_operation);
    let h_episode = Statement::prepare(connection.as_ptr(),
        "SELECT e.raw_hex,COALESCE(e.old_generation,''),e.process_operation_id
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id
             AND g.session_id=e.session_id AND g.generation=e.generation
             AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id
          WHERE e.domain_id=?1 AND e.request_id=?2 AND e.session_id=?3
            AND e.home_id=?4 AND e.instance_id=?5 AND e.generation=?6
            AND e.binding_id=?7")?;
    let expected_binding = format!("binding-{}", home_id.strip_prefix("home-")
        .ok_or(TemporaryHomeError::HomeConflict)?);
    for (index, value) in [row.1.as_str(), request.request_id.as_str(), row.3.as_str(),
        home_id, row.0.as_str(), row.4.as_str(), expected_binding.as_str()]
        .iter().enumerate() {
        h_episode.bind_text(index as i32 + 1, value)?;
    }
    let episode_source = if h_episode.step_row()? {
        let found = (h_episode.column_text(0)?, h_episode.column_text(1)?,
            h_episode.column_text(2)?);
        if h_episode.step_row()? { return Err(TemporaryHomeError::HomeConflict); }
        Some(found)
    } else { None };
    match request.operation.as_str() {
        "admission-reserve" => {
            if !matches!(operation_source.as_ref(), Some((value, status))
                if value == &original_hex && status == "APPLIED")
                || episode_source.is_some() {
                return Err(TemporaryHomeError::StopFactUnavailable);
            }
        },
        "open" | "resume" | "compact" | "renew-session" => {
            let Some((episode_raw, old_generation, process_operation)) = episode_source else {
                return Err(TemporaryHomeError::StopFactUnavailable);
            };
            let process_digest = sha256_hex(&raw);
            let compound = matches!(request.operation.as_str(), "compact" | "renew-session");
            let execution = if compound { "resume" } else { request.operation.as_str() };
            if episode_raw != original_hex
                || process_operation != format!("h-{execution}-{}", &process_digest[..40])
                || (request.operation == "open" && (!old_generation.is_empty()
                    || !matches!(operation_source.as_ref(), Some((value, status))
                        if value == &original_hex && status == "APPLIED")))
                || (request.operation != "open" && (old_generation.is_empty()
                    || old_generation == row.4 || operation_source.is_some())) {
                return Err(TemporaryHomeError::StopFactUnavailable);
            }
            if request.operation != "open" {
                let previous = old_generation.parse::<u64>()
                    .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
                let generation = row.4.parse::<u64>()
                    .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
                if previous.checked_add(1) != Some(generation) {
                    return Err(TemporaryHomeError::StopFactUnavailable);
                }
                let duplicate = Statement::prepare(connection.as_ptr(),
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_operation
                      WHERE domain_id=?1 AND request_id=?2")?;
                duplicate.bind_text(1, &row.1)?;
                duplicate.bind_text(2, &request.request_id)?;
                if !duplicate.step_row()? || duplicate.column_text(0)? != "0" {
                    return Err(TemporaryHomeError::StopFactUnavailable);
                }
                let old = Statement::prepare(connection.as_ptr(),
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode p
                      JOIN main.gogoke_v37_h_generation g ON g.domain_id=p.domain_id
                        AND g.session_id=p.session_id AND g.generation=p.generation
                        AND g.request_id=p.request_id AND g.process_operation_id=p.process_operation_id
                      JOIN main.gogoke_coordination_process_custody c
                        ON c.operation_id=p.process_operation_id AND c.domain_id=p.domain_id
                        AND c.generation=p.generation
                     WHERE p.domain_id=?1 AND p.session_id=?2 AND p.instance_id=?3
                       AND p.generation=?4 AND p.phase='STOPPED'
                       AND p.stop_fact_id IS NOT NULL AND c.state='STOPPED'
                       AND c.stop_proof_hash=p.stop_fact_id")?;
                for (index, value) in [&row.1, &row.3, &row.0, &old_generation]
                    .iter().enumerate() { old.bind_text(index as i32 + 1, value)?; }
                if !old.step_row()? || old.column_text(0)? != "1" {
                    return Err(TemporaryHomeError::StopFactUnavailable);
                }
            }
            if compound {
                let expected_ack = format!("compact-{}", &process_digest[..40]);
                let change = Statement::prepare(connection.as_ptr(),
                    "SELECT COUNT(*) FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode p ON p.domain_id=g.domain_id AND p.session_id=g.session_id AND p.generation=g.old_generation AND p.process_operation_id=g.old_process_operation_id JOIN main.gogoke_coordination_process_custody c ON c.operation_id=p.process_operation_id AND c.profile_id=p.instance_id AND c.domain_id=p.domain_id AND c.generation=p.generation JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=g.domain_id AND e.request_id=g.request_id AND e.process_operation_id=?8 WHERE g.domain_id=?1 AND g.request_id=?2 AND g.raw_hex=?3 AND g.operation=?4 AND g.session_id=?5 AND g.old_generation=?6 AND g.stage='APPLIED' AND g.owner_stop_request_id IS NULL AND g.seat_id=e.seat_id AND g.previous_revision=?7 AND g.result_revision=e.result_revision AND ((g.unknown_revision IS NULL AND e.previous_revision=g.previous_revision) OR (g.unknown_revision=g.previous_revision+1 AND e.previous_revision=g.unknown_revision)) AND e.home_id=?9 AND e.generation=?10 AND e.old_generation=g.old_generation AND p.phase='STOPPED' AND p.stop_request_id IS NULL AND p.stop_fact_id IS NOT NULL AND c.ticket=g.old_ticket AND c.custodian_nonce=g.old_nonce AND c.state='STOPPED' AND c.stop_proof_hash=p.stop_fact_id AND ((g.operation='compact' AND g.ack_step_id=?11 AND g.item_source_epoch IS NOT NULL AND g.item_source_cursor IS NOT NULL AND g.item_id IS NOT NULL) OR (g.operation='renew-session' AND g.ack_step_id IS NULL AND g.item_source_epoch IS NULL AND g.item_source_cursor IS NULL AND g.item_id IS NULL))")?;
                let expected_revision = request.expected_revision.to_string();
                for (index, value) in [row.1.as_str(), request.request_id.as_str(),
                    original_hex.as_str(), request.operation.as_str(), row.3.as_str(),
                    old_generation.as_str(), expected_revision.as_str(),
                    process_operation.as_str(), home_id, row.4.as_str(), expected_ack.as_str()]
                    .iter().enumerate() { change.bind_text(index as i32 + 1, value)?; }
                if !change.step_row()? || change.column_text(0)? != "1" {
                    return Err(TemporaryHomeError::StopFactUnavailable);
                }
            }
        },
        _ => return Err(TemporaryHomeError::RequestConflict),
    }
    Ok((operation_id, request.request_id, request.operation, stored, sha256_hex(&raw)))
}

fn selected_user_request(input: &UserTemporaryHomeCloseInput<'_>)
    -> Result<(), TemporaryHomeError> {
    use crate::store::atomic::{Json, JsonString};
    if !valid_id(input.request_id) || !valid_id(input.instance_id)
        || !valid_id(input.domain_id) || !valid_id(input.session_id)
        || input.expected_revision != 1 || input.request_bytes.is_empty()
        || input.request_bytes.len() > 65_536 {
        return Err(TemporaryHomeError::Invalid("temporary-home-close input"));
    }
    let request = crate::store::session_transport::decode_request(input.request_bytes)
        .map_err(|_| TemporaryHomeError::Invalid("temporary-home-close frame"))?;
    if request.family != "K-INSTANCE" || request.operation != "temporary-home-close"
        || request.request_id != input.request_id || request.target_id != input.instance_id
        || request.domain_id != input.domain_id || request.expected_revision != 1
        || request.payload.len() != 2 {
        return Err(TemporaryHomeError::RequestConflict);
    }
    let value = |name: &str| -> Option<String> {
        match request.payload.get(&JsonString::from_str(name)) {
            Some(Json::String(value)) => value.to_well_formed_string(), _ => None,
        }
    };
    if value("sessionId").as_deref() != Some(input.session_id) {
        return Err(TemporaryHomeError::RequestConflict);
    }
    let selected = match input.selector {
        UserTemporaryHomeSelector::OriginalCreateRequestId(id) =>
            value("originalCreateRequestId").as_deref() == Some(id)
                && value("historicalHomeId").is_none() && valid_id(id),
        UserTemporaryHomeSelector::HistoricalHomeId(id) =>
            value("historicalHomeId").as_deref() == Some(id)
                && value("originalCreateRequestId").is_none() && valid_id(id),
    };
    if !selected { return Err(TemporaryHomeError::RequestConflict); }
    Ok(())
}

fn selected_home(connection: &VerifiedDatabaseConnection<'_>, input: &UserTemporaryHomeCloseInput<'_>)
    -> Result<String, TemporaryHomeError> {
    match input.selector {
        UserTemporaryHomeSelector::HistoricalHomeId(id) => Ok(id.to_owned()),
        UserTemporaryHomeSelector::OriginalCreateRequestId(id) => {
            let digest = sha256_hex(format!("{}\n{}", input.domain_id, id).as_bytes());
            let create_id = format!("homeprep-{}", &digest[..40]);
            let selected = Statement::prepare(connection.as_ptr(),
                "SELECT target_id FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
            selected.bind_text(1, &create_id)?;
            if !selected.step_row()? { return Err(TemporaryHomeError::HomeConflict); }
            let home = selected.column_text(0)?;
            if selected.step_row()? { return Err(TemporaryHomeError::HomeConflict); }
            Ok(home)
        }
    }
}

fn prior_stop_marker(connection: &VerifiedDatabaseConnection<'_>, home: &str,
    row: &HomeRow) -> Result<bool, TemporaryHomeError> {
    let query = Statement::prepare(connection.as_ptr(),
        "SELECT COUNT(*) FROM main.gogoke_v37_h_process_episode e
          LEFT JOIN main.gogoke_coordination_process_custody c
            ON c.operation_id=e.process_operation_id
          WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3
            AND e.session_id=?4 AND e.generation=?5
            AND (e.phase='STOPPED' OR e.stop_fact_id IS NOT NULL
              OR c.state='STOPPED' OR c.stop_proof_hash IS NOT NULL)")?;
    for (index, value) in [home, &row.0, &row.1, &row.3, &row.4].iter().enumerate() {
        query.bind_text(index as i32 + 1, value)?;
    }
    if !query.step_row()? { return Err(TemporaryHomeError::Unknown); }
    Ok(query.column_text(0)? != "0")
}

fn original_stop_request(connection: &VerifiedDatabaseConnection<'_>, home: &str,
    row: &HomeRow, stop_fact: &str) -> Result<String, TemporaryHomeError> {
    use crate::store::atomic::{Json, JsonString};
    let source = Statement::prepare(connection.as_ptr(),
        "SELECT e.stop_request_id,o.raw_hex,o.status,
                CAST(o.previous_revision AS TEXT),CAST(o.revision AS TEXT),
                COALESCE(e.seat_id,''),COALESCE(e.seat_incarnation,''),
                COALESCE(CAST(e.result_revision AS TEXT),'')
           FROM main.gogoke_v37_h_process_episode e
           JOIN main.gogoke_v37_h_operation o ON o.domain_id=e.domain_id
             AND o.request_id=e.stop_request_id AND o.session_id=e.session_id
             AND o.operation='stop'
          WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3
            AND e.session_id=?4 AND e.generation=?5 AND e.phase='STOPPED'
            AND e.stop_fact_id=?6")?;
    for (index, value) in [home, &row.0, &row.1, &row.3, &row.4, stop_fact]
        .iter().enumerate() { source.bind_text(index as i32 + 1, value)?; }
    if !source.step_row()? { return Err(TemporaryHomeError::StopFactUnavailable); }
    let request_id = source.column_text(0)?;
    let raw_hex = source.column_text(1)?;
    let status = source.column_text(2)?;
    let before = source.column_text(3)?.parse::<u64>()
        .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
    let after = source.column_text(4)?.parse::<u64>()
        .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
    let seat_id = source.column_text(5)?;
    let incarnation = source.column_text(6)?;
    let episode_revision = source.column_text(7)?.parse::<u64>()
        .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
    if source.step_row()? || status != "APPLIED" || request_id.is_empty()
        || seat_id.is_empty() || incarnation.is_empty()
        || after != before.checked_add(1).ok_or(TemporaryHomeError::StopFactUnavailable)?
        || before < episode_revision {
        return Err(TemporaryHomeError::StopFactUnavailable);
    }
    let raw = decode_lower_hex(&raw_hex)?;
    let request = crate::store::session_transport::decode_request(&raw)
        .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
    if request.family != "K-SESSION" || request.operation != "stop"
        || request.request_id != request_id || request.domain_id != row.1
        || request.target_id != row.3 || hex(&raw) != raw_hex
        || request.expected_revision != before || request.payload.len() != 2 {
        return Err(TemporaryHomeError::StopFactUnavailable);
    }
    let text = |name: &str| match request.payload.get(&JsonString::from_str(name)) {
        Some(Json::String(value)) => value.to_well_formed_string(), _ => None,
    };
    if text("generation").as_deref() != Some(row.4.as_str())
        || text("seatId").as_deref() != Some(seat_id.as_str()) {
        return Err(TemporaryHomeError::StopFactUnavailable);
    }
    let same_generation = Statement::prepare(connection.as_ptr(),
        "SELECT CAST(revision AS TEXT),COALESCE(stop_fact_id,''),state
           FROM main.gogoke_v37_h_claim
          WHERE domain_id=?1 AND session_id=?2 AND instance_id=?3
            AND generation=?4 AND home_id=?5")?;
    for (index, value) in [&row.1, &row.3, &row.0, &row.4, home]
        .iter().enumerate() { same_generation.bind_text(index as i32 + 1, value)?; }
    if same_generation.step_row()? {
        let current = same_generation.column_text(0)?.parse::<u64>()
            .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
        let current_stop = same_generation.column_text(1)?;
        let state = same_generation.column_text(2)?;
        if same_generation.step_row()? || current < after || current_stop != stop_fact
            || !matches!(state.as_str(), "STOPPED" | "RELEASED") {
            return Err(TemporaryHomeError::StopFactUnavailable);
        }
    }
    Ok(sha256_hex(&raw))
}

fn user_close_operation(connection: &VerifiedDatabaseConnection<'_>, request_id: &str)
    -> Result<Option<(String, String, String, String, String)>, TemporaryHomeError> {
    let source = Statement::prepare(connection.as_ptr(),
        "SELECT request_hex,target_id,phase,COALESCE(receipt_json,''),
                COALESCE(native_receipt_id,'')
           FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    source.bind_text(1, request_id)?;
    if !source.step_row()? { return Ok(None); }
    let row = (source.column_text(0)?, source.column_text(1)?,
        source.column_text(2)?, source.column_text(3)?, source.column_text(4)?);
    if source.step_row()? { return Err(TemporaryHomeError::RequestConflict); }
    Ok(Some(row))
}

fn original_pairs_digest(pairs: &[(u32, u64)]) -> String {
    let mut bytes = Vec::with_capacity(pairs.len() * 12);
    for (pid, creation) in pairs {
        bytes.extend_from_slice(&pid.to_be_bytes());
        bytes.extend_from_slice(&creation.to_be_bytes());
    }
    sha256_hex(&bytes)
}

fn saved_kernel_witness(receipt: &str) -> Result<(u64, String, String), TemporaryHomeError> {
    let Json::Object(fields) = Parser::parse(receipt).map_err(|_| TemporaryHomeError::Unknown)?
        else { return Err(TemporaryHomeError::Unknown); };
    let text = |name: &str| -> Result<String, TemporaryHomeError> {
        match fields.get(&JsonString::from_str(name)) {
            Some(Json::String(value)) => value.to_well_formed_string()
                .ok_or(TemporaryHomeError::Unknown),
            _ => Err(TemporaryHomeError::Unknown),
        }
    };
    if text("schema")? != "gogoke.37.temporary-home-close.kernel.v1" ||
        text("proofKind")? != "KERNEL_BOOT_CUTOFF" {
        return Err(TemporaryHomeError::Unknown);
    }
    let boot_text = text("kernelBootStart")?;
    let boot = boot_text.parse::<u64>().map_err(|_| TemporaryHomeError::Unknown)?;
    let source = text("originalSourceDigest")?;
    let pairs = text("originalPairsDigest")?;
    if boot == 0 || boot.to_string() != boot_text ||
        [source.as_str(), pairs.as_str()].iter().any(|digest|
            digest.len() != 64 || !digest.bytes().all(|byte|
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))) {
        return Err(TemporaryHomeError::Unknown);
    }
    Ok((boot, source, pairs))
}

fn kernel_replay_boot(receipt: Option<&str>, current_boot: u64,
    source: &str, pairs: &[(u32, u64)]) -> Result<u64, TemporaryHomeError> {
    let Some(receipt) = receipt else { return Ok(current_boot); };
    let (sealed_boot, sealed_source, sealed_pairs) = saved_kernel_witness(receipt)?;
    if sealed_source != source || sealed_pairs != original_pairs_digest(pairs) ||
        sealed_boot > current_boot || pairs.iter().any(|(_, creation)| *creation >= sealed_boot) {
        return Err(TemporaryHomeError::Unknown);
    }
    Ok(sealed_boot)
}

/// Independently journaled F USER action. This leaves both the original H/C
/// uncertainty and the physical HOME directory/marker untouched.
pub(crate) fn close_user_temporary_home(connection: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, input: &UserTemporaryHomeCloseInput<'_>)
    -> Result<UserTemporaryHomeCloseReceipt, TemporaryHomeError> {
    selected_user_request(input)?;
    if connection.root_identity() != &root.canonical_root().identity {
        return Err(TemporaryHomeError::IdentityChanged);
    }
    transaction(connection, |connection| {
        check_owner_in_current_transaction(connection, owner).map_err(TemporaryHomeError::Owner)?;
        let home_id = selected_home(connection, input)?;
        if !valid_id(&home_id) { return Err(TemporaryHomeError::HomeConflict); }
        let row = home_row(connection, &home_id)?.ok_or(TemporaryHomeError::HomeConflict)?;
        if row.0 != input.instance_id || row.1 != input.domain_id || row.2 != "SESSION"
            || row.3 != input.session_id || !valid_generation(&row.4) {
            return Err(TemporaryHomeError::HomeConflict);
        }
        row_identity(&row, &home_id)?;
        let suffix = home_id.strip_prefix("home-")
            .ok_or(TemporaryHomeError::HomeConflict)?;
        if suffix.len() != 40 || !suffix.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
            return Err(TemporaryHomeError::HomeConflict);
        }
        let profile = AppContainerProfile::derive_for_revocation(&format!("Gogoke37.Session.{suffix}"))?;
        let sid = profile.sid_identity()?;
        let binding = verify_home_owner_in_transaction(connection, &row.0, &row.1,
            &row.2, &row.3, &row.4)?;
        if binding != format!("binding-{suffix}") {
            return Err(TemporaryHomeError::HomeConflict);
        }
        let original_seat = Statement::prepare(connection.as_ptr(),
            "SELECT e.seat_incarnation,COALESCE(e.seat_id,''),e.request_id,
                    e.raw_hex,e.process_operation_id,COALESCE(e.old_generation,'')
               FROM main.gogoke_v37_h_process_episode e
               JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id
                 AND g.session_id=e.session_id AND g.generation=e.generation
                 AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id
              WHERE e.home_id=?1 AND e.instance_id=?2 AND e.domain_id=?3
                AND e.session_id=?4 AND e.generation=?5 AND e.binding_id=?6")?;
        for (index, value) in [&home_id, &row.0, &row.1, &row.3, &row.4, &binding]
            .iter().enumerate() { original_seat.bind_text(index as i32 + 1, value)?; }
        if !original_seat.step_row()? {
            return Err(TemporaryHomeError::StopFactUnavailable);
        }
        let incarnation = original_seat.column_text(0)?;
        let seat_id = original_seat.column_text(1)?;
        let episode_request_id = original_seat.column_text(2)?;
        let episode_raw_hex = original_seat.column_text(3)?;
        let episode_process = original_seat.column_text(4)?;
        let old_generation = original_seat.column_text(5)?;
        if incarnation.is_empty() || seat_id.is_empty() || original_seat.step_row()? {
            return Err(TemporaryHomeError::HomeConflict);
        }
        let episode_raw = decode_lower_hex(&episode_raw_hex)?;
        let episode_request = crate::store::session_transport::decode_request(&episode_raw)
            .map_err(|_| TemporaryHomeError::StopFactUnavailable)?;
        let process_digest = sha256_hex(&episode_raw);
        if episode_request.family != "K-SESSION"
            || !matches!(episode_request.operation.as_str(), "open" | "resume" | "compact" | "renew-session")
            || episode_request.request_id != episode_request_id
            || episode_request.domain_id != row.1 || episode_request.target_id != row.3
            || episode_raw_hex != hex(&episode_raw)
            || episode_process != format!("h-{}-{}",
                if episode_request.operation == "open" { "open" } else { "resume" }, &process_digest[..40])
            || (episode_request.operation == "open" && !old_generation.is_empty())
            || (episode_request.operation != "open" && old_generation.is_empty()) {
            return Err(TemporaryHomeError::StopFactUnavailable);
        }
        if episode_request.operation == "open" {
            let original_open = Statement::prepare(connection.as_ptr(),
                "SELECT raw_hex,status FROM main.gogoke_v37_h_operation
                  WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                    AND operation='open'")?;
            for (index, value) in [&row.1, &episode_request_id, &row.3]
                .iter().enumerate() { original_open.bind_text(index as i32 + 1, value)?; }
            if !original_open.step_row()? || original_open.column_text(0)? != episode_raw_hex
                || original_open.column_text(1)? != "APPLIED" || original_open.step_row()? {
                return Err(TemporaryHomeError::StopFactUnavailable);
            }
        } else {
            let unexpected = Statement::prepare(connection.as_ptr(),
                "SELECT COUNT(*) FROM main.gogoke_v37_h_operation
                  WHERE domain_id=?1 AND request_id=?2")?;
            unexpected.bind_text(1, &row.1)?;
            unexpected.bind_text(2, &episode_request_id)?;
            if !unexpected.step_row()? || unexpected.column_text(0)? != "0" {
                return Err(TemporaryHomeError::StopFactUnavailable);
            }
        }
        let expected_suffix = sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            root.canonical_root().identity.opaque(), row.1, row.3, incarnation, row.4).as_bytes());
        if suffix != &expected_suffix[..40] { return Err(TemporaryHomeError::HomeConflict); }
        drop(original_seat);
        let (create_id, original_create_request_id, create_operation, create_fingerprint,
            create_raw_sha256) =
            original_create_source(connection, &home_id, &row, &input.selector, &sid)?;
        if !matches!((create_operation.as_str(), episode_request.operation.as_str()),
            ("admission-reserve", "open") | ("open", "open") | ("resume", "resume")
                | ("compact", "compact") | ("renew-session", "renew-session"))
            || (create_operation != "admission-reserve"
                && original_create_request_id != episode_request_id) {
            return Err(TemporaryHomeError::StopFactUnavailable);
        }
        let physical = match transition_stage(connection, root, &home_id, &row, &sid)? {
            Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == row.6 => identity,
            _ => return Err(TemporaryHomeError::IdentityChanged),
        };
        let request = TransitionTemporaryHome { request_id: input.request_id,
            request_bytes: input.request_bytes, home_id: &home_id,
            expected_revision: input.expected_revision };
        let request_hex = transition_fingerprint("temporary-home-close-v2", &request, &sid);
        let prior_operation = user_close_operation(connection, input.request_id)?;
        if prior_operation.as_ref().is_some_and(|(stored, target, _, _, _)|
            stored != &request_hex || target != &home_id) {
            return Err(TemporaryHomeError::RequestConflict);
        }
        let (proof_kind, stop_fingerprint, kernel_source) = match exact_stopped_home(connection, &home_id, &row) {
            Ok(stop) => {
                let stop_request_sha256 = original_stop_request(connection, &home_id, &row, &stop)?;
                ("ORIGINAL_STOP", sha256_hex(format!(
                    "home-stop-v1\n{home_id}\n{create_id}\n{create_raw_sha256}\n{stop_request_sha256}\n{stop}"
                ).as_bytes()), None)
            },
            Err(TemporaryHomeError::StopFactUnavailable)
            | Err(TemporaryHomeError::Admission(AdmissionError::Denied
                | AdmissionError::Unknown | AdmissionError::Conflict | AdmissionError::Stale)) => {
                // A damaged or contradictory StopFact never falls back to an
                // absence proof. The original facts remain unchanged.
                if prior_stop_marker(connection, &home_id, &row)? {
                    return Err(TemporaryHomeError::StopFactUnavailable);
                }
                let (fact, source, pairs) = crate::store::session_transport::observe_exact_home_gone(
                    connection, &home_id, &row.0, &row.1, &row.3, &row.4, &binding)?;
                ("KERNEL_BOOT_CUTOFF", String::new(), Some((fact, source, pairs)))
            },
            Err(error) => return Err(error),
        };
        let (proof_fingerprint, kernel_witness) = if let Some((fact, source, pairs)) = kernel_source.as_ref() {
            let pair_digest = original_pairs_digest(pairs);
            let sealed_boot = kernel_replay_boot(prior_operation.as_ref()
                .map(|(_, _, _, receipt, _)| receipt.as_str()), fact.boot_start(), source, pairs)?;
            (sha256_hex(format!("temporary-home-kernel-cutoff-v1\n{source}\n{sealed_boot}").as_bytes()),
                Some((sealed_boot, source.clone(), pair_digest)))
        } else { (stop_fingerprint, None) };
        let recheck_kernel = |connection: &VerifiedDatabaseConnection<'_>| -> Result<(), TemporaryHomeError> {
            if let Some((original, _, _)) = kernel_source.as_ref() {
                match transition_stage(connection, root, &home_id, &row, &sid)? {
                    Some(DirectoryStage::Marked(identity, _)) if identity.opaque() == row.6 => (),
                    _ => return Err(TemporaryHomeError::IdentityChanged),
                }
                let (current, source, pairs) = crate::store::session_transport::observe_exact_home_gone(
                    connection, &home_id, &row.0, &row.1, &row.3, &row.4, &binding)?;
                if current.boot_start() != original.boot_start() ||
                    source.as_str() != kernel_source.as_ref().unwrap().1.as_str() ||
                    pairs.as_slice() != kernel_source.as_ref().unwrap().2.as_slice() {
                    return Err(TemporaryHomeError::IdentityChanged);
                }
            }
            Ok(())
        };
        let source_fingerprint = sha256_hex(format!(
            "temporary-home-close-v2\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
            root.canonical_root().identity.opaque(), connection.identity().opaque(),
            home_id, row.0, row.1, row.3, row.4, row.6, sid,
            create_fingerprint, create_raw_sha256, proof_kind, proof_fingerprint
        ).as_bytes());
        let receipt_id = format!("f-home-close-v2-{}", sha256_hex(format!(
            "f-home-close-receipt-v2\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n1\n2",
            input.request_id, sha256_hex(input.request_bytes), source_fingerprint,
            home_id, input.instance_id, input.domain_id, input.session_id,
        ).as_bytes()));
        let stored_receipt = if let Some((boot, source, pairs)) = kernel_witness.as_ref() {
            format!("{{\"schema\":\"gogoke.37.temporary-home-close.kernel.v1\",\"receiptId\":\"{receipt_id}\",\"proofKind\":\"{proof_kind}\",\"sourceFingerprint\":\"{source_fingerprint}\",\"homeId\":\"{home_id}\",\"originalCreateRequestId\":\"{original_create_request_id}\",\"fCreateOperationId\":\"{create_id}\",\"kernelBootStart\":\"{boot}\",\"originalSourceDigest\":\"{source}\",\"originalPairsDigest\":\"{pairs}\",\"beforeRevision\":1,\"afterRevision\":2}}")
        } else {
            format!("{{\"schema\":\"gogoke.37.temporary-home-close.v2\",\"receiptId\":\"{receipt_id}\",\"proofKind\":\"{proof_kind}\",\"sourceFingerprint\":\"{source_fingerprint}\",\"homeId\":\"{home_id}\",\"originalCreateRequestId\":\"{original_create_request_id}\",\"fCreateOperationId\":\"{create_id}\",\"beforeRevision\":1,\"afterRevision\":2}}")
        };
        recheck_kernel(connection)?;
        if let Some((stored, target, phase, receipt_json, original_receipt_id)) =
            prior_operation {
            if stored != request_hex || target != home_id {
                return Err(TemporaryHomeError::RequestConflict);
            }
            if phase != "APPLIED" || row.7 != "CLOSED" || row.8 != "2"
                || receipt_json != stored_receipt || original_receipt_id != receipt_id {
                return Err(TemporaryHomeError::Unknown);
            }
            return Ok(UserTemporaryHomeCloseReceipt {
                disposition: "REPLAYED", home_id, directory_ref: row.5,
                native_receipt_id: receipt_id, proof_kind, source_fingerprint,
                before_revision: 1, after_revision: 2,
            });
        }
        if row.7 != "ACTIVE" || row.8 != "1" {
            return Err(TemporaryHomeError::StaleRevision);
        }
        if physical.opaque() != row.6 { return Err(TemporaryHomeError::IdentityChanged); }
        let insert = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations
             (request_id,request_hex,target_id,phase,receipt_json,native_receipt_id)
             VALUES(?1,?2,?3,'APPLIED',?4,?5)")?;
        for (index, value) in [input.request_id, &request_hex, &home_id, &stored_receipt, &receipt_id]
            .iter().enumerate() { insert.bind_text(index as i32 + 1, value)?; }
        insert.step_done()?;
        let update = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instance_homes SET state='CLOSED',revision=2
               WHERE home_id=?1 AND instance_id=?2 AND domain_id=?3 AND kind='SESSION'
                 AND owner_id=?4 AND generation=?5 AND directory_identity=?6
                 AND state='ACTIVE' AND revision=1")?;
        for (index, value) in [&home_id, &row.0, &row.1, &row.3, &row.4, &row.6]
            .iter().enumerate() { update.bind_text(index as i32 + 1, value)?; }
        update.step_done()?;
        let changed = Statement::prepare(connection.as_ptr(), "SELECT changes()")?;
        if !changed.step_row()? || changed.column_text(0)? != "1" {
            return Err(TemporaryHomeError::StaleRevision);
        }
        recheck_kernel(connection)?;
        Ok(UserTemporaryHomeCloseReceipt {
            disposition: "APPLIED", home_id, directory_ref: row.5,
            native_receipt_id: receipt_id, proof_kind, source_fingerprint,
            before_revision: 1, after_revision: 2,
        })
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
        crate::store::seat::initialize_schema(&mut connection).unwrap();
        crate::store::session_transport::session_binding::initialize_schema(&mut connection).unwrap();
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
        // Existing synthetic F control, including the exact E/legacy H
        // relationship required by release; this is not launch authority.
        connection.execute("INSERT INTO main.gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,state,generation,revision) VALUES('projectA','seatA','incA','USER',NULL,'LONG','instanceA','BUSY',1,1)").unwrap();
        connection.execute("INSERT INTO main.gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES('projectA','seatA','templateA','{}')").unwrap();
        connection.execute("INSERT INTO main.gogoke_v37_h_seat_binding(domain_id,session_id,seat_id,seat_incarnation,generation) VALUES('projectA','sessionA','seatA','incA','1')").unwrap();
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

    #[test]
    fn user_close_requires_original_h_stop_and_keeps_independent_f_receipt() {
        fixture(|connection, root, _| {
            let owner = crate::store::authority::initialize_profile(connection, root).unwrap();
            let create_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"admission-reserve","requestId":"reserveUserA","targetId":"sessionA","domainId":"projectA","expectedRevision":"0","payload":{}}"#;
            let stop_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"stop","requestId":"stopUserA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1"}}"#;
            let close_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"temporary-home-close","requestId":"closeUserA","targetId":"instanceA","domainId":"projectA","expectedRevision":"1","payload":{"sessionId":"sessionA","originalCreateRequestId":"reserveUserA"}}"#;
            let open_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openUserA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{}}"#;
            let open_digest = sha256_hex(open_raw);
            let process_id = format!("h-open-{}", &open_digest[..40]);
            let suffix = sha256_hex(format!("{}\nprojectA\nsessionA\nincA\n1",
                root.canonical_root().identity.opaque()).as_bytes());
            let suffix = &suffix[..40];
            let home_id = format!("home-{suffix}");
            let binding_id = format!("binding-{suffix}");
            let prepare_hash = sha256_hex(b"projectA\nreserveUserA");
            let prepare_id = format!("homeprep-{}", &prepare_hash[..40]);
            connection.execute("BEGIN IMMEDIATE").unwrap();
            let bind = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_owner_binding SET binding_id=?1 WHERE binding_id='bindingA'").unwrap();
            bind.bind_text(1, &binding_id).unwrap(); bind.step_done().unwrap();
            connection.execute("COMMIT").unwrap();
            let profile = AppContainerProfile::derived_for_test(&format!("Gogoke37.Session.{suffix}")).unwrap();
            let create = CreateTemporaryHome { request_id: &prepare_id, request_bytes: create_raw,
                home_id: &home_id, instance_id: "instanceA", domain_id: "projectA",
                kind: TemporaryKind::Session, owner_id: "sessionA", generation: "1" };
            create_temporary_home(connection, root, &profile, &create).unwrap();
            let custody = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES(?1,'ticketA','nonceA','11','1','fixture-program','sha256:fixture','instanceA','projectA','1','STOPPED','proofA')").unwrap();
            custody.bind_text(1, &process_id).unwrap(); custody.step_done().unwrap();
            let insert = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id,stop_fact_id) VALUES('projectA','sessionA','instanceA',?1,?2,'1','STOPPED',2,?3,'proofA')").unwrap();
            insert.bind_text(1, &home_id).unwrap(); insert.bind_text(2, &binding_id).unwrap();
            insert.bind_text(3, &process_id).unwrap();
            insert.step_done().unwrap();
            let episode = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,stop_request_id,phase,stop_fact_id) VALUES('projectA','openUserA','sessionA','1',?1,0,1,?2,'instanceA',?3,?4,'seatA','incA','stopUserA','STOPPED','proofA')").unwrap();
            episode.bind_text(1, &hex(open_raw)).unwrap(); episode.bind_text(2, &process_id).unwrap();
            episode.bind_text(3, &home_id).unwrap(); episode.bind_text(4, &binding_id).unwrap();
            episode.step_done().unwrap();
            let generation = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openUserA',?1)").unwrap();
            generation.bind_text(1, &process_id).unwrap(); generation.step_done().unwrap();
            for (id, raw, operation) in [("reserveUserA", create_raw.as_slice(), "admission-reserve"),
                ("openUserA", open_raw.as_slice(), "open"),
                ("stopUserA", stop_raw.as_slice(), "stop")] {
                let h = Statement::prepare(connection.as_ptr(),
                    "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA',?1,?2,?3,'sessionA','APPLIED',?4,?5)").unwrap();
                h.bind_text(1, id).unwrap(); h.bind_text(2, &hex(raw)).unwrap();
                h.bind_text(3, operation).unwrap();
                h.bind_i64(4, if operation == "admission-reserve" { 0 } else { 1 }).unwrap();
                h.bind_i64(5, if operation == "admission-reserve" { 1 } else { 2 }).unwrap();
                h.step_done().unwrap();
            }
            let close = UserTemporaryHomeCloseInput { request_id: "closeUserA",
                request_bytes: close_raw, instance_id: "instanceA", domain_id: "projectA",
                session_id: "sessionA", expected_revision: 1,
                selector: UserTemporaryHomeSelector::OriginalCreateRequestId("reserveUserA") };
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET raw_hex='00' WHERE request_id='openUserA'").unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            let restore_episode = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_process_episode SET raw_hex=?1 WHERE request_id='openUserA'").unwrap();
            restore_episode.bind_text(1, &hex(open_raw)).unwrap(); restore_episode.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_operation SET raw_hex='00' WHERE request_id='openUserA'").unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            let restore_open = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET raw_hex=?1 WHERE request_id='openUserA'").unwrap();
            restore_open.bind_text(1, &hex(open_raw)).unwrap(); restore_open.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_operation SET raw_hex='00' WHERE request_id='reserveUserA'").unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            let original = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET raw_hex=?1 WHERE request_id='reserveUserA'").unwrap();
            original.bind_text(1, &hex(create_raw)).unwrap(); original.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_operation SET raw_hex='00' WHERE request_id='stopUserA'").unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            let repair = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET raw_hex=?1 WHERE request_id='stopUserA'").unwrap();
            repair.bind_text(1, &hex(stop_raw)).unwrap(); repair.step_done().unwrap();
            let wrong_generation = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"stop","requestId":"stopUserA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"2"}}"#;
            let wrong = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET raw_hex=?1 WHERE request_id='stopUserA'").unwrap();
            wrong.bind_text(1, &hex(wrong_generation)).unwrap(); wrong.step_done().unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::StopFactUnavailable)));
            let repair = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_operation SET raw_hex=?1 WHERE request_id='stopUserA'").unwrap();
            repair.bind_text(1, &hex(stop_raw)).unwrap(); repair.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET phase='ACTIVE',stop_fact_id=NULL WHERE request_id='openUserA'").unwrap();
            let unknown = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_coordination_process_custody SET state='UNKNOWN',stop_proof_hash=NULL WHERE operation_id=?1").unwrap();
            unknown.bind_text(1, &process_id).unwrap(); unknown.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_claim SET state='COMMITTED',stop_fact_id=NULL WHERE session_id='sessionA'").unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::ExactGone(
                    crate::store::session_transport::ExactHomeGoneError::Missing(
                        "unique completed original H episode")))));
            assert_eq!(home_row(connection, &home_id).unwrap().unwrap().7, "ACTIVE");
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET phase='STOPPED',stop_fact_id='proofA' WHERE request_id='openUserA'").unwrap();
            let stopped = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_coordination_process_custody SET state='STOPPED',stop_proof_hash='proofA' WHERE operation_id=?1").unwrap();
            stopped.bind_text(1, &process_id).unwrap(); stopped.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_claim SET state='STOPPED',stop_fact_id='proofA' WHERE session_id='sessionA'").unwrap();
            connection.execute("INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('otherDomain','otherUnknown','00','open','otherSession','UNKNOWN',0,0)").unwrap();
            let applied = close_user_temporary_home(connection, root, &owner, &close).unwrap();
            assert_eq!(applied.disposition, "APPLIED");
            assert_eq!(applied.proof_kind, "ORIGINAL_STOP");
            assert_eq!(applied.home_id, home_id);
            assert_ne!(applied.native_receipt_id, home_row(connection, &home_id).unwrap().unwrap().6);
            assert!(applied.native_receipt_id.starts_with("f-home-close-v2-"));
            let saved = user_close_operation(connection, "closeUserA").unwrap().unwrap();
            assert_eq!(saved.4, applied.native_receipt_id);
            assert!(saved.3.contains(&format!("\"receiptId\":\"{}\"", applied.native_receipt_id)));
            assert_eq!(close_user_temporary_home(connection, root, &owner, &close).unwrap().disposition,
                "REPLAYED");
            connection.execute("UPDATE main.gogoke_v37_instance_operations SET native_receipt_id='wrongCloseReceipt' WHERE request_id='closeUserA'").unwrap();
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::Unknown)));
            let restore = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET native_receipt_id=?1 WHERE request_id='closeUserA'").unwrap();
            restore.bind_text(1, &applied.native_receipt_id).unwrap(); restore.step_done().unwrap();
            let mut changed_raw = close_raw.to_vec();
            changed_raw.push(b' ');
            let changed = UserTemporaryHomeCloseInput { request_bytes: &changed_raw, ..close };
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &changed),
                Err(TemporaryHomeError::RequestConflict)));
            let row = home_row(connection, &home_id).unwrap().unwrap();
            assert_eq!((row.7.as_str(), row.8.as_str()), ("CLOSED", "2"));
            assert!(directory(root, &create).exists(), "logical close retains the original HOME");
        });
    }

    #[test]
    fn synthetic_preboot_identity_checks_only_kernel_close_transaction_semantics() {
        // This is a transaction fixture. PID MAX / FILETIME 1 did not come
        // from a prior real boot; installed preboot qualification remains NOT_RUN.
        fixture(|connection, root, _| {
            let owner = crate::store::authority::initialize_profile(connection, root).unwrap();
            let create_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"admission-reserve","requestId":"reserveColdA","targetId":"sessionA","domainId":"projectA","expectedRevision":"0","payload":{}}"#;
            let open_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openColdA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"worktreeA"}}"#;
            let close_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-INSTANCE","operation":"temporary-home-close","requestId":"closeColdA","targetId":"instanceA","domainId":"projectA","expectedRevision":"1","payload":{"sessionId":"sessionA","originalCreateRequestId":"reserveColdA"}}"#;
            let suffix = sha256_hex(format!("{}\nprojectA\nsessionA\nincA\n1",
                root.canonical_root().identity.opaque()).as_bytes());
            let suffix = &suffix[..40];
            let home_id = format!("home-{suffix}");
            let binding_id = format!("binding-{suffix}");
            let prepare_hash = sha256_hex(b"projectA\nreserveColdA");
            let prepare_id = format!("homeprep-{}", &prepare_hash[..40]);
            let process_hash = sha256_hex(open_raw);
            let process_id = format!("h-open-{}", &process_hash[..40]);
            connection.execute("BEGIN IMMEDIATE").unwrap();
            let binding = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_h_owner_binding SET binding_id=?1 WHERE binding_id='bindingA'").unwrap();
            binding.bind_text(1, &binding_id).unwrap(); binding.step_done().unwrap(); drop(binding);
            connection.execute("COMMIT").unwrap();
            let profile = AppContainerProfile::derived_for_test(&format!("Gogoke37.Session.{suffix}")).unwrap();
            let create = CreateTemporaryHome { request_id: &prepare_id, request_bytes: create_raw,
                home_id: &home_id, instance_id: "instanceA", domain_id: "projectA",
                kind: TemporaryKind::Session, owner_id: "sessionA", generation: "1" };
            create_temporary_home(connection, root, &profile, &create).unwrap();
            let reserve = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','reserveColdA',?1,'admission-reserve','sessionA','APPLIED',0,1)").unwrap();
            reserve.bind_text(1, &hex(create_raw)).unwrap(); reserve.step_done().unwrap(); drop(reserve);
            let open = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openColdA',?1,'open','sessionA','APPLIED',1,2)").unwrap();
            let open_hex = hex(open_raw);
            open.bind_text(1, &open_hex).unwrap(); open.step_done().unwrap(); drop(open);
            let claim = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision,process_operation_id) VALUES('projectA','sessionA','instanceA',?1,?2,'1','COMMITTED',2,?3)").unwrap();
            claim.bind_text(1, &home_id).unwrap(); claim.bind_text(2, &binding_id).unwrap();
            claim.bind_text(3, &process_id).unwrap(); claim.step_done().unwrap(); drop(claim);
            let episode = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','openColdA','sessionA','1',?1,1,2,?2,'instanceA',?3,?4,'seatA','incA','ACTIVE')").unwrap();
            for (index, value) in [open_hex.as_str(), process_id.as_str(), home_id.as_str(),
                binding_id.as_str()].iter().enumerate() { episode.bind_text((index + 1) as i32, value).unwrap(); }
            episode.step_done().unwrap(); drop(episode);
            let generation = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openColdA',?1)").unwrap();
            generation.bind_text(1, &process_id).unwrap(); generation.step_done().unwrap(); drop(generation);
            let custody = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES(?1,'ticketCold','nonceCold','4294967295','1','synthetic.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','projectA','1','UNKNOWN')").unwrap();
            custody.bind_text(1, &process_id).unwrap(); custody.step_done().unwrap(); drop(custody);
            let close = UserTemporaryHomeCloseInput { request_id: "closeColdA", request_bytes: close_raw,
                instance_id: "instanceA", domain_id: "projectA", session_id: "sessionA",
                expected_revision: 1,
                selector: UserTemporaryHomeSelector::OriginalCreateRequestId("reserveColdA") };
            let applied = close_user_temporary_home(connection, root, &owner, &close).unwrap();
            assert_eq!(applied.disposition, "APPLIED");
            assert_eq!(applied.proof_kind, "KERNEL_BOOT_CUTOFF");
            assert_eq!(close_user_temporary_home(connection, root, &owner, &close).unwrap().disposition,
                "REPLAYED");
            let mut changed_bytes = close_raw.to_vec(); changed_bytes.push(b' ');
            let changed = UserTemporaryHomeCloseInput { request_bytes: &changed_bytes, ..close };
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &changed),
                Err(TemporaryHomeError::RequestConflict)));
            let saved = user_close_operation(connection, "closeColdA").unwrap().unwrap();
            assert_eq!(saved.2, "APPLIED");
            assert_eq!(saved.4, applied.native_receipt_id);
            assert!(saved.3.contains("\"proofKind\":\"KERNEL_BOOT_CUTOFF\""));
            let (sealed_boot, sealed_source, sealed_pairs) = saved_kernel_witness(&saved.3).unwrap();
            assert!(sealed_boot > 1);
            assert_eq!(sealed_source.len(), 64);
            assert_eq!(sealed_pairs.len(), 64);
            assert_eq!(kernel_replay_boot(Some(&saved.3), sealed_boot + 1,
                &sealed_source, &[(u32::MAX, 1)]).unwrap(), sealed_boot,
                "a later boot must reuse the original sealed cutoff");
            assert!(matches!(kernel_replay_boot(Some(&saved.3), sealed_boot + 1,
                "bad-source", &[(u32::MAX, 1)]), Err(TemporaryHomeError::Unknown)));
            let damaged = saved.3.replace(&format!("\"kernelBootStart\":\"{sealed_boot}\""),
                "\"kernelBootStart\":\"1\"");
            let tamper = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET receipt_json=?1 WHERE request_id='closeColdA'").unwrap();
            tamper.bind_text(1, &damaged).unwrap(); tamper.step_done().unwrap(); drop(tamper);
            assert!(matches!(close_user_temporary_home(connection, root, &owner, &close),
                Err(TemporaryHomeError::Unknown)));
            let restore = Statement::prepare(connection.as_ptr(),
                "UPDATE main.gogoke_v37_instance_operations SET receipt_json=?1 WHERE request_id='closeColdA'").unwrap();
            restore.bind_text(1, &saved.3).unwrap(); restore.step_done().unwrap(); drop(restore);
            assert_eq!(close_user_temporary_home(connection, root, &owner, &close).unwrap().disposition,
                "REPLAYED");
            let home = home_row(connection, &home_id).unwrap().unwrap();
            assert_eq!((home.7.as_str(), home.8.as_str()), ("CLOSED", "2"));
            assert!(directory(root, &create).join(MARKER).exists());
            let unchanged = Statement::prepare(connection.as_ptr(),
                "SELECT e.phase,COALESCE(e.stop_fact_id,''),c.state,COALESCE(c.stop_proof_hash,''),a.state,COALESCE(a.stop_fact_id,'') FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id JOIN main.gogoke_v37_h_claim a ON a.domain_id=e.domain_id AND a.session_id=e.session_id WHERE e.home_id=?1").unwrap();
            unchanged.bind_text(1, &home_id).unwrap();
            assert!(unchanged.step_row().unwrap());
            assert_eq!(unchanged.column_text(0).unwrap(), "ACTIVE");
            assert_eq!(unchanged.column_text(1).unwrap(), "");
            assert_eq!(unchanged.column_text(2).unwrap(), "UNKNOWN");
            assert_eq!(unchanged.column_text(3).unwrap(), "");
            assert_eq!(unchanged.column_text(4).unwrap(), "COMMITTED");
            assert_eq!(unchanged.column_text(5).unwrap(), "");
            assert!(!unchanged.step_row().unwrap());
        });
    }

    #[test]
    fn compound_create_source_keeps_original_wire_and_internal_resume_identity() {
        // Writer-shaped durable rows exercise F's actual source lookup. No
        // provider, former boot, or installed-product outcome is simulated.
        fixture(|connection, root, _| {
            macro_rules! put {
                ($sql:expr, [$($value:expr),+ $(,)?]) => {{
                    let row = Statement::prepare(connection.as_ptr(), $sql).unwrap();
                    let values: &[&str] = &[$($value),+];
                    for (index, value) in values.iter().enumerate() {
                        row.bind_text((index + 1) as i32, value).unwrap();
                    }
                    row.step_done().unwrap();
                }};
            }
            let raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"compact","requestId":"nextA","targetId":"sessionA","domainId":"projectA","expectedRevision":"2","payload":{"generation":"1"}}"#;
            let raw_hex = hex(raw);
            let digest = sha256_hex(raw);
            let process = format!("h-resume-{}", &digest[..40]);
            let old_raw = br#"{"schema":"gogoke.37.operations.v1","family":"K-SESSION","operation":"open","requestId":"openA","targetId":"sessionA","domainId":"projectA","expectedRevision":"1","payload":{"seatId":"seatA","generation":"1","repositoryId":"repoA","worktreeId":"worktreeA"}}"#;
            let old_hex = hex(old_raw);
            let old_process = format!("h-open-{}", &sha256_hex(old_raw)[..40]);
            let suffix = sha256_hex(format!("{}\nprojectA\nsessionA\nincA\n2",
                root.canonical_root().identity.opaque()).as_bytes());
            let suffix = &suffix[..40];
            let home = format!("home-{suffix}");
            let binding = format!("binding-{suffix}");
            connection.execute("BEGIN IMMEDIATE").unwrap();
            bind_owner_in_transaction(connection, &OwnerBinding { binding_id: &binding,
                instance_id: "instanceA", domain_id: "projectA", kind: "SESSION",
                owner_id: "sessionA", generation: "2" }).unwrap();
            connection.execute("COMMIT").unwrap();
            let profile = AppContainerProfile::derived_for_test(&format!("Gogoke37.Session.{suffix}")).unwrap();
            let preparation = sha256_hex(b"projectA\nnextA");
            let prepare_id = format!("homeprep-{}", &preparation[..40]);
            let create = CreateTemporaryHome { request_id: &prepare_id, request_bytes: raw,
                home_id: &home, instance_id: "instanceA", domain_id: "projectA",
                kind: TemporaryKind::Session, owner_id: "sessionA", generation: "2" };
            create_temporary_home(connection, root, &profile, &create).unwrap();
            put!("INSERT INTO main.gogoke_v37_h_operation(domain_id,request_id,raw_hex,operation,session_id,status,previous_revision,revision) VALUES('projectA','openA',?1,'open','sessionA','APPLIED',1,2)", [&old_hex]);
            put!("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase,stop_fact_id) VALUES('projectA','openA','sessionA','1',?1,1,2,?2,'instanceA','oldHome','oldBinding','seatA','incA','STOPPED','oldProof')", [&old_hex,&old_process]);
            put!("INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','1','openA',?1)", [&old_process]);
            put!("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES(?1,'oldTicket','oldNonce','41','1','fixture.exe','sha256:0000000000000000000000000000000000000000000000000000000000000000','instanceA','projectA','1','STOPPED','oldProof')", [&old_process]);
            put!("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,seat_incarnation,phase) VALUES('projectA','nextA','sessionA','2','1',?1,2,3,?2,'instanceA',?3,?4,'seatA','incA','ACTIVE')", [&raw_hex,&process,&home,&binding]);
            put!("INSERT INTO main.gogoke_v37_h_generation(domain_id,session_id,generation,request_id,process_operation_id) VALUES('projectA','sessionA','2','nextA',?1)", [&process]);
            let ack = format!("compact-{}", &digest[..40]);
            put!("INSERT INTO main.gogoke_v37_h_generation_change(domain_id,request_id,raw_hex,operation,session_id,old_generation,old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,previous_revision,source_watermark,stage,ack_step_id,item_source_epoch,item_source_cursor,item_id) VALUES('projectA','nextA',?1,'compact','sessionA','1',?2,'oldTicket','oldNonce','threadOld','seatA',2,0,'OLD_STOPPED',?3,'epochA','1','itemA')", [&raw_hex,&old_process,&ack]);
            crate::store::session_transport::generation_change::mark_applied(connection,
                "projectA","nextA",3).unwrap();
            let row = home_row(connection, &home).unwrap().unwrap();
            let sid = profile.sid_identity().unwrap();
            let source = original_create_source(connection, &home, &row,
                &UserTemporaryHomeSelector::OriginalCreateRequestId("nextA"), &sid).unwrap();
            assert_eq!((&source.0[..], &source.1[..], &source.2[..]),
                (prepare_id.as_str(), "nextA", "compact"));
            connection.execute("UPDATE main.gogoke_v37_h_generation_change SET old_ticket='wrong' WHERE request_id='nextA'").unwrap();
            assert!(matches!(original_create_source(connection, &home, &row,
                &UserTemporaryHomeSelector::OriginalCreateRequestId("nextA"), &sid),
                Err(TemporaryHomeError::StopFactUnavailable)));
        });
    }
}
