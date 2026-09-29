//! F.1 global instance registration. The caller must already hold the native
//! Owner issuer, root pin, and verified database connection. No wire request
//! or model-supplied claim is admitted here.

use super::home::{prepare_persistent_home, HomeError};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::Statement;
use crate::store::digest::content_hash;
use crate::store::same_open::VerifiedDatabaseConnection;
use std::fs;
use std::io::{self, Read};
use std::os::windows::fs::MetadataExt;
use std::path::Path;

const CONTAINER: &str = "v37-instances";
const MARKER: &str = "gogoke-instance.marker";
const REPARSE_POINT: u32 = 0x400;

#[derive(Debug)]
pub(crate) enum RegistryError {
    Invalid(&'static str),
    RequestConflict,
    InstanceConflict,
    Unknown,
    CommitUnknown(crate::store::same_open::SameOpenError),
    RollbackUnknown(crate::store::same_open::SameOpenError),
    RevisionParse(std::num::ParseIntError),
    IdentityChanged,
    Root(crate::root::RootLockError),
    Home(HomeError),
    Io(io::Error),
    Store(crate::store::atomic::AtomicError),
    Sqlite(crate::store::same_open::SameOpenError),
}

impl From<crate::store::atomic::AtomicError> for RegistryError {
    fn from(error: crate::store::atomic::AtomicError) -> Self { Self::Store(error) }
}
impl From<crate::store::same_open::SameOpenError> for RegistryError {
    fn from(error: crate::store::same_open::SameOpenError) -> Self { Self::Sqlite(error) }
}
impl From<io::Error> for RegistryError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}

/// A digest/version observed by a trusted native program probe. This record
/// does not assert that a future process launch uses these same bytes; H must
/// pin and check the executable again at launch. F.2 owns repinning.
pub(crate) struct ProgramObservation {
    digest: String,
    version: String,
}

impl ProgramObservation {
    /// Capture bytes through one file handle. The version is a display fact
    /// supplied by the trusted native probe, not derived from the file name.
    pub(crate) fn observe(path: &Path, version: &str) -> Result<Self, RegistryError> {
        if version.is_empty() || version.len() > 128 || version.chars().any(char::is_control) {
            return Err(RegistryError::Invalid("version"));
        }
        let path_before = fs::symlink_metadata(path)?;
        if !path_before.is_file() || path_before.file_attributes() & REPARSE_POINT != 0 {
            return Err(RegistryError::IdentityChanged);
        }
        let mut file = fs::File::open(path)?;
        let before = file.metadata()?;
        if !before.is_file() || before.file_attributes() & REPARSE_POINT != 0 {
            return Err(RegistryError::IdentityChanged);
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let after = file.metadata()?;
        let path_after = fs::symlink_metadata(path)?;
        if before.len() != after.len() || before.modified()? != after.modified()?
            || path_before.len() != path_after.len() || path_before.modified()? != path_after.modified()?
            || path_after.file_attributes() & REPARSE_POINT != 0 || bytes.len() as u64 != after.len() {
            return Err(RegistryError::IdentityChanged);
        }
        Ok(Self { digest: content_hash(&bytes), version: version.to_owned() })
    }
}

pub(crate) struct Registration<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) instance_id: &'a str,
    pub(crate) driver_id: &'a str,
    pub(crate) program: &'a ProgramObservation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationDisposition { Applied, Replayed }

#[derive(Clone, Copy)]
pub(crate) enum InstanceObservation {
    InstallUnknown,
    Installed,
    Missing,
    LoginUnknown,
    LoggedIn,
    LoggedOut,
}

impl InstanceObservation {
    fn value(self) -> &'static str {
        match self {
            Self::InstallUnknown | Self::LoginUnknown => "UNKNOWN",
            Self::Installed => "INSTALLED",
            Self::Missing => "MISSING",
            Self::LoggedIn => "LOGGED_IN",
            Self::LoggedOut => "LOGGED_OUT",
        }
    }
    fn field(self) -> &'static str {
        match self {
            Self::InstallUnknown | Self::Installed | Self::Missing => "install",
            _ => "login",
        }
    }
}

pub(crate) struct ObservationRequest<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) instance_id: &'a str,
    pub(crate) expected_revision: i64,
    pub(crate) observation: InstanceObservation,
}

fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty() && bytes.len() <= 64 && bytes[0].is_ascii_alphabetic()
        && bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
        && !matches!(value.to_ascii_uppercase().as_str(), "CON" | "PRN" | "AUX" | "NUL")
        && !(value.len() == 4 && (value[..3].eq_ignore_ascii_case("COM") || value[..3].eq_ignore_ascii_case("LPT"))
            && matches!(bytes[3], b'1'..=b'9'))
}

fn valid_atom(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/'))
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}

fn fingerprint(input: &Registration<'_>) -> Result<String, RegistryError> {
    if !valid_atom(input.request_id) { return Err(RegistryError::Invalid("request_id")); }
    if !valid_id(input.instance_id) { return Err(RegistryError::Invalid("instance_id")); }
    if !valid_atom(input.driver_id) { return Err(RegistryError::Invalid("driver_id")); }
    if input.request_bytes.is_empty() || input.request_bytes.len() > 65_536 {
        return Err(RegistryError::Invalid("request_bytes"));
    }
    if !input.program.digest.starts_with("sha256:") || input.program.digest.len() != 71
        || !input.program.digest[7..].bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegistryError::Invalid("program_digest"));
    }
    if input.program.version.is_empty() || input.program.version.len() > 128
        || input.program.version.chars().any(char::is_control) {
        return Err(RegistryError::Invalid("version"));
    }
    // Length framing binds the exact request bytes to every typed field. A
    // caller cannot replay one request ID with changed bytes or observations.
    let mut framed = Vec::new();
    for field in [input.request_bytes, input.instance_id.as_bytes(), input.driver_id.as_bytes(),
        input.program.digest.as_bytes(), input.program.version.as_bytes()] {
        framed.extend_from_slice(&(field.len() as u64).to_be_bytes());
        framed.extend_from_slice(field);
    }
    Ok(encode_hex(&framed))
}

fn observation_fingerprint(input: &ObservationRequest<'_>) -> Result<String, RegistryError> {
    if !valid_atom(input.request_id) || !valid_id(input.instance_id)
        || input.request_bytes.is_empty() || input.request_bytes.len() > 65_536
        || input.expected_revision < 1 {
        return Err(RegistryError::Invalid("observation"));
    }
    let mut framed = Vec::new();
    let revision = input.expected_revision.to_string();
    for field in [&b"observe"[..], input.request_bytes, input.instance_id.as_bytes(),
        revision.as_bytes(), input.observation.field().as_bytes(),
        input.observation.value().as_bytes()] {
        framed.extend_from_slice(&(field.len() as u64).to_be_bytes());
        framed.extend_from_slice(field);
    }
    Ok(encode_hex(&framed))
}

fn operation(connection: &VerifiedDatabaseConnection<'_>, request_id: &str)
    -> Result<Option<(String, String, String, String)>, RegistryError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM gogoke_v37_instance_operations WHERE request_id=?1")?;
    row.bind_text(1, request_id)?;
    if row.step_row()? { Ok(Some((row.column_text(0)?, row.column_text(1)?, row.column_text(2)?, row.column_text(3)?))) }
    else { Ok(None) }
}

fn pending_target(connection: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<bool, RegistryError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT request_id FROM gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN') LIMIT 1")?;
    row.bind_text(1, instance_id)?;
    row.step_row().map_err(Into::into)
}

fn instance(connection: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<Option<(String, String, String, String, String, String, String)>, RegistryError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT driver_id,home_ref,home_identity,program_digest,version,install_state,login_state FROM gogoke_v37_instances WHERE instance_id=?1")?;
    row.bind_text(1, instance_id)?;
    if row.step_row()? {
        Ok(Some((row.column_text(0)?,row.column_text(1)?,row.column_text(2)?,
            row.column_text(3)?,row.column_text(4)?,row.column_text(5)?,row.column_text(6)?)))
    } else { Ok(None) }
}

fn transaction<T>(connection: &mut VerifiedDatabaseConnection<'_>, action: impl FnOnce(&mut VerifiedDatabaseConnection<'_>) -> Result<T, RegistryError>)
    -> Result<T, RegistryError> {
    connection.execute("BEGIN IMMEDIATE")?;
    match action(connection) {
        Ok(value) => {
            connection.execute("COMMIT").map_err(RegistryError::CommitUnknown)?;
            Ok(value)
        }
        Err(error) => {
            connection.execute("ROLLBACK").map_err(RegistryError::RollbackUnknown)?;
            Err(error)
        }
    }
}

fn checked_identity(path: &Path) -> Result<RootIdentity, RegistryError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(RegistryError::IdentityChanged);
    }
    Ok(inspect_root(path).map_err(RegistryError::Root)?.identity)
}

fn observed_home(root: &RootLock, instance_id: &str) -> Result<Option<RootIdentity>, RegistryError> {
    let root_path = &root.canonical_root().canonical_path;
    if checked_identity(root_path)? != root.canonical_root().identity { return Err(RegistryError::IdentityChanged); }
    let parent = root_path.join(CONTAINER);
    if !parent.exists() { return Ok(None); }
    let parent_identity = checked_identity(&parent)?;
    let path = parent.join(instance_id);
    if !path.exists() { return Ok(None); }
    let identity = checked_identity(&path)?;
    let marker = path.join(MARKER);
    let metadata = fs::symlink_metadata(&marker)?;
    if !metadata.is_file() || metadata.file_attributes() & REPARSE_POINT != 0 {
        return Err(RegistryError::IdentityChanged);
    }
    let expected = format!("gogoke-v37-instance-home-v1\n{}\n{instance_id}\n", root.canonical_root().identity.opaque());
    if metadata.len() != expected.len() as u64 {
        return Err(RegistryError::IdentityChanged);
    }
    let mut actual = String::new();
    fs::File::open(&marker)?.take(512).read_to_string(&mut actual)?;
    if actual != expected
        || checked_identity(&parent)? != parent_identity || checked_identity(&path)? != identity {
        return Err(RegistryError::IdentityChanged);
    }
    Ok(Some(identity))
}

/// An exact replay may complete an interrupted registration after inspecting
/// the native home. An ambiguous home or commit remains UNKNOWN; no directory
/// is removed, reused under another request, or reported as registered.
pub(crate) fn register_instance(connection: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &Registration<'_>) -> Result<RegistrationDisposition, RegistryError> {
    let request_hex = fingerprint(input)?;
    // The first attempt may reserve only an absent name. A later replay may
    // adopt a home only when this operation already holds its physical receipt.
    let initially_absent = observed_home(root, input.instance_id)?.is_none();
    let prior = transaction(connection, |connection| {
        if let Some((stored, target, phase, receipt)) = operation(connection, input.request_id)? {
            if stored != request_hex || target != input.instance_id { return Err(RegistryError::RequestConflict); }
            return Ok(Some((phase, receipt)));
        }
        if !initially_absent { return Err(RegistryError::InstanceConflict); }
        if instance(connection, input.instance_id)?.is_some() { return Err(RegistryError::InstanceConflict); }
        if pending_target(connection, input.instance_id)? { return Err(RegistryError::InstanceConflict); }
        let insert = Statement::prepare(connection.as_ptr(),
            "INSERT INTO gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'PREPARING')")?;
        insert.bind_text(1, input.request_id)?;
        insert.bind_text(2, &request_hex)?;
        insert.bind_text(3, input.instance_id)?;
        insert.step_done()?;
        Ok(None)
    })?;
    if prior.as_ref().is_some_and(|(phase, _)| phase == "APPLIED") {
        let home = observed_home(root, input.instance_id)?.ok_or(RegistryError::Unknown)?;
        let expected = format!("instance-home-{}", input.instance_id);
        match instance(connection, input.instance_id)? {
            Some((driver, home_ref, identity, digest, version, _, _))
                if driver == input.driver_id && home_ref == expected && identity == home.opaque()
                    && digest == input.program.digest && version == input.program.version =>
                    return Ok(RegistrationDisposition::Replayed),
            _ => return Err(RegistryError::Unknown),
        }
    }
    if prior.as_ref().is_some_and(|(phase, _)| phase != "PREPARING" && phase != "UNKNOWN") {
        return Err(RegistryError::Unknown);
    }
    let home = match observed_home(root, input.instance_id) {
        Ok(Some(identity)) if prior.as_ref().is_some_and(|(_, receipt)| receipt == &identity.opaque()) => identity,
        Ok(Some(_)) => return Err(RegistryError::Unknown),
        Ok(None) if prior.as_ref().is_some_and(|(_, receipt)| !receipt.is_empty()) =>
            return Err(RegistryError::Unknown),
        Ok(None) => match prepare_persistent_home(root, input.instance_id) {
            Ok(prepared) => prepared.identity,
            Err(HomeError::ExistingInstance) => return Err(RegistryError::Unknown),
            Err(error) => return Err(RegistryError::Home(error)),
        },
        Err(error) => return Err(error),
    };
    if observed_home(root, input.instance_id)? != Some(home.clone()) {
        return Err(RegistryError::IdentityChanged);
    }
    // Persist the physical creation receipt independently from the final row.
    // A crash between mkdir and this commit remains UNKNOWN, not adoptable.
    transaction(connection, |connection| {
        let Some((stored, target, phase, receipt)) = operation(connection, input.request_id)? else { return Err(RegistryError::Unknown); };
        if stored != request_hex || target != input.instance_id { return Err(RegistryError::RequestConflict); }
        if phase != "PREPARING" && phase != "UNKNOWN" { return Err(RegistryError::Unknown); }
        if !receipt.is_empty() && receipt != home.opaque() { return Err(RegistryError::IdentityChanged); }
        if receipt.is_empty() {
            let update = Statement::prepare(connection.as_ptr(),
                "UPDATE gogoke_v37_instance_operations SET native_receipt_id=?1 WHERE request_id=?2 AND native_receipt_id IS NULL")?;
            update.bind_text(1, &home.opaque())?;
            update.bind_text(2, input.request_id)?;
            update.step_done()?;
        }
        Ok(())
    })?;
    transaction(connection, |connection| {
        let Some((stored, target, phase, receipt)) = operation(connection, input.request_id)? else { return Err(RegistryError::Unknown); };
        if stored != request_hex || target != input.instance_id { return Err(RegistryError::RequestConflict); }
        if receipt != home.opaque() { return Err(RegistryError::Unknown); }
        if phase == "APPLIED" { return Err(RegistryError::Unknown); }
        if phase != "PREPARING" && phase != "UNKNOWN" { return Err(RegistryError::Unknown); }
        if instance(connection, input.instance_id)?.is_some() { return Err(RegistryError::InstanceConflict); }
        let insert = Statement::prepare(connection.as_ptr(),
            "INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES(?1,?2,?3,?4,?5,?6,'UNKNOWN','UNKNOWN',1)")?;
        insert.bind_text(1, input.instance_id)?;
        insert.bind_text(2, input.driver_id)?;
        insert.bind_text(3, &format!("instance-home-{}", input.instance_id))?;
        insert.bind_text(4, &home.opaque())?;
        insert.bind_text(5, &input.program.digest)?;
        insert.bind_text(6, &input.program.version)?;
        insert.step_done()?;
        let update = Statement::prepare(connection.as_ptr(),
            "UPDATE gogoke_v37_instance_operations SET phase='APPLIED' WHERE request_id=?1 AND phase IN ('PREPARING','UNKNOWN')")?;
        update.bind_text(1, input.request_id)?;
        update.step_done()?;
        Ok(())
    })?;
    if observed_home(root, input.instance_id)? != Some(home) {
        return Err(RegistryError::Unknown);
    }
    Ok(RegistrationDisposition::Applied)
}

/// Persist a fact from a trusted native probe. This never probes credentials,
/// infers login from files, or turns an unknown observation into a false value.
/// The caller must bind its probe to this instance and to the current revision.
pub(crate) fn record_observation(connection: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &ObservationRequest<'_>) -> Result<RegistrationDisposition, RegistryError> {
    let request_hex = observation_fingerprint(input)?;
    let home = observed_home(root, input.instance_id)?.ok_or(RegistryError::Unknown)?;
    transaction(connection, |connection| {
        if let Some((stored, target, phase, _)) = operation(connection, input.request_id)? {
            if stored != request_hex || target != input.instance_id { return Err(RegistryError::RequestConflict); }
            match instance(connection, input.instance_id)? {
                Some((_, _, identity, _, _, _, _)) if identity == home.opaque() => (),
                _ => return Err(RegistryError::Unknown),
            }
            return if phase == "APPLIED" { Ok(RegistrationDisposition::Replayed) }
                else { Err(RegistryError::Unknown) };
        }
        let row = Statement::prepare(connection.as_ptr(),
            "SELECT home_identity,revision FROM gogoke_v37_instances WHERE instance_id=?1")?;
        row.bind_text(1, input.instance_id)?;
        if !row.step_row()? { return Err(RegistryError::InstanceConflict); }
        if row.column_text(0)? != home.opaque() { return Err(RegistryError::IdentityChanged); }
        let revision = row.column_text(1)?.parse::<i64>().map_err(RegistryError::RevisionParse)?;
        drop(row);
        if revision != input.expected_revision || revision == i64::MAX {
            return Err(RegistryError::InstanceConflict);
        }
        let sql = if input.observation.field() == "install" {
            "UPDATE gogoke_v37_instances SET install_state=?1,revision=revision+1 WHERE instance_id=?2 AND revision=?3"
        } else {
            "UPDATE gogoke_v37_instances SET login_state=?1,revision=revision+1 WHERE instance_id=?2 AND revision=?3"
        };
        let update = Statement::prepare(connection.as_ptr(), sql)?;
        update.bind_text(1, input.observation.value())?;
        update.bind_text(2, input.instance_id)?;
        update.bind_i64(3, input.expected_revision)?;
        update.step_done()?;
        let insert = Statement::prepare(connection.as_ptr(),
            "INSERT INTO gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'APPLIED')")?;
        insert.bind_text(1, input.request_id)?;
        insert.bind_text(2, &request_hex)?;
        insert.bind_text(3, input.instance_id)?;
        insert.step_done()?;
        Ok(RegistrationDisposition::Applied)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::instance::initialize_schema;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &RootLock)) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("gogoke-v37-registry-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let root = RootLock::acquire(&path).unwrap();
        let database = path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        initialize_schema(&mut connection).unwrap();
        run(&mut connection, &root);
        connection.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    fn program(root: &RootLock) -> ProgramObservation {
        let path = root.canonical_root().canonical_path.join("observed-cli.bin");
        fs::write(&path, b"owned test program bytes").unwrap();
        ProgramObservation::observe(&path, "0.1.0").unwrap()
    }

    fn registration<'a>(id: &'a str, bytes: &'a [u8], instance_id: &'a str,
        program: &'a ProgramObservation) -> Registration<'a> {
        Registration { request_id: id, request_bytes: bytes, instance_id,
            driver_id: "codex", program }
    }

    #[test]
    fn durable_register_and_exact_replay_reject_changed_bytes() {
        fixture(|connection, root| {
            let observed = program(root);
            let input = registration("req-1", b"register one", "instanceA", &observed);
            assert_eq!(register_instance(connection, root, &input).unwrap(), RegistrationDisposition::Applied);
            assert_eq!(register_instance(connection, root, &input).unwrap(), RegistrationDisposition::Replayed);
            let changed = registration("req-1", b"register two", "instanceA", &observed);
            assert!(matches!(register_instance(connection, root, &changed), Err(RegistryError::RequestConflict)));
            let other = registration("req-2", b"register one", "instanceA", &observed);
            assert!(matches!(register_instance(connection, root, &other), Err(RegistryError::InstanceConflict)));
        });
    }

    #[test]
    fn prepared_request_recovers_only_its_own_marked_home() {
        fixture(|connection, root| {
            let observed = program(root);
            let input = registration("req-1", b"one", "instanceA", &observed);
            let key = fingerprint(&input).unwrap();
            let prepared = prepare_persistent_home(root, input.instance_id).unwrap();
            let insert = Statement::prepare(connection.as_ptr(),
                "INSERT INTO gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,native_receipt_id) VALUES(?1,?2,?3,'UNKNOWN',?4)").unwrap();
            insert.bind_text(1, input.request_id).unwrap();
            insert.bind_text(2, &key).unwrap();
            insert.bind_text(3, input.instance_id).unwrap();
            insert.bind_text(4, &prepared.identity.opaque()).unwrap();
            insert.step_done().unwrap();
            assert_eq!(observed_home(root, input.instance_id).unwrap(), Some(prepared.identity));
            assert_eq!(register_instance(connection, root, &input).unwrap(), RegistrationDisposition::Applied);
            let operation = operation(connection, input.request_id).unwrap().unwrap();
            assert_eq!(operation.2, "APPLIED");
        });
    }

    #[test]
    fn missing_physical_receipt_never_adopts_a_marked_home() {
        fixture(|connection, root| {
            let observed = program(root);
            let input = registration("req-1", b"one", "instanceA", &observed);
            let insert = Statement::prepare(connection.as_ptr(),
                "INSERT INTO gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'PREPARING')").unwrap();
            insert.bind_text(1, input.request_id).unwrap();
            insert.bind_text(2, &fingerprint(&input).unwrap()).unwrap();
            insert.bind_text(3, input.instance_id).unwrap();
            insert.step_done().unwrap();
            prepare_persistent_home(root, input.instance_id).unwrap();
            assert!(matches!(register_instance(connection, root, &input), Err(RegistryError::Unknown)));
            assert!(instance(connection, input.instance_id).unwrap().is_none());
        });
    }

    #[test]
    fn unowned_home_is_not_adopted_and_observation_is_revision_bound() {
        fixture(|connection, root| {
            let observed = program(root);
            prepare_persistent_home(root, "instanceA").unwrap();
            let unowned = registration("req-1", b"one", "instanceA", &observed);
            assert!(matches!(register_instance(connection, root, &unowned), Err(RegistryError::InstanceConflict)));
            assert!(matches!(register_instance(connection, root, &unowned), Err(RegistryError::InstanceConflict)));
            let input = registration("req-2", b"two", "instanceB", &observed);
            register_instance(connection, root, &input).unwrap();
            let observed = ObservationRequest { request_id: "obs-1", request_bytes: b"login observation",
                instance_id: "instanceB", expected_revision: 1, observation: InstanceObservation::LoggedIn };
            assert_eq!(record_observation(connection, root, &observed).unwrap(), RegistrationDisposition::Applied);
            assert_eq!(record_observation(connection, root, &observed).unwrap(), RegistrationDisposition::Replayed);
            let stale = ObservationRequest { request_id: "obs-2", ..observed };
            assert!(matches!(record_observation(connection, root, &stale), Err(RegistryError::InstanceConflict)));
        });
    }
}
