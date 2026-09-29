//! F.1 temporary SESSION/CALL home creation. H's durable stop and admission
//! fence are not yet wired: close and cleanup fail closed and never delete.

use super::registry::{observed_home, RegistryError};
use crate::process::{AppContainerProfile, IsolationError};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::{AtomicError, Statement};
use crate::store::digest::sha256_hex;
use crate::store::same_open::{SameOpenError, VerifiedDatabaseConnection};
use crate::store::session_transport::{verify_home_owner_in_transaction, AdmissionError};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::windows::fs::MetadataExt;
use std::path::Path;

const CONTAINER: &str = "v37-instances";
const TEMPORARY_CONTAINER: &str = "temporary-homes";
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
    InstanceChanged,
    IdentityChanged,
    Unknown,
    CommitUnknown(SameOpenError),
    RollbackUnknown(SameOpenError),
    StopFactUnavailable,
    AdmissionFenceUnavailable,
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
    for value in [&b"temporary-create"[..], input.request_bytes, input.home_id.as_bytes(),
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
    root.canonical_root().canonical_path.join(CONTAINER).join(input.instance_id)
        .join(TEMPORARY_CONTAINER).join(directory_ref(input.home_id))
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
    format!("gogoke-v37-temporary-home-v2\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
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
    let temporary = instance.join(TEMPORARY_CONTAINER);
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
    let temporary = instance.join(TEMPORARY_CONTAINER);
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

pub(crate) fn close_temporary_home(_connection: &mut VerifiedDatabaseConnection<'_>,
    _root: &RootLock, _input: &TransitionTemporaryHome<'_>) -> Result<TemporaryHomeReceipt, TemporaryHomeError> {
    Err(TemporaryHomeError::StopFactUnavailable)
}
pub(crate) fn cleanup_temporary_home(_connection: &mut VerifiedDatabaseConnection<'_>,
    _root: &RootLock, _input: &TransitionTemporaryHome<'_>) -> Result<TemporaryHomeReceipt, TemporaryHomeError> {
    Err(TemporaryHomeError::AdmissionFenceUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::instance::initialize_schema;
    use crate::store::instance::registry::{register_instance, ProgramObservation, Registration};
    use crate::store::same_open::{create_new, route_b_test_guard};
    use crate::store::session_transport::{bind_owner_in_transaction, initialize_admission_schema, OwnerBinding};
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
        let program_path = root_path.join("test-program.bin");
        fs::write(&program_path, b"fixture program bytes").unwrap();
        let program = ProgramObservation::observe(&program_path, "0.149.0").unwrap();
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
            assert!(matches!(close_temporary_home(connection, root, &transition),
                Err(TemporaryHomeError::StopFactUnavailable)));
            assert!(matches!(cleanup_temporary_home(connection, root, &transition),
                Err(TemporaryHomeError::AdmissionFenceUnavailable)));
            assert!(matches!(directory_stage(root, &first, &profile.sid_identity().unwrap()).unwrap(),
                Some(DirectoryStage::Marked(_, true))));
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
}
