//! F.1 global instance registration. The caller must already hold the native
//! Owner issuer, root pin, and verified database connection. No wire request
//! or model-supplied claim is admitted here.

use super::home::{prepare_persistent_home, HomeError};
use crate::root::{inspect_root, RootIdentity, RootLock};
use crate::store::atomic::{Json, JsonString, Statement};
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::digest::content_hash;
use crate::store::same_open::VerifiedDatabaseConnection;
use std::fs;
use std::io::{self, Read};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

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
    Authority(String),
    RepinFormat(String),
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
    pub(crate) fn matches_pin(&self, digest: &str, version: &str) -> bool {
        self.digest == digest && self.version == version
    }

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

pub(crate) struct ProgramRepin<'a> {
    pub(crate) request_id: &'a str,
    pub(crate) request_bytes: &'a [u8],
    pub(crate) instance_id: &'a str,
    pub(crate) expected_revision: i64,
}

#[derive(Debug)]
pub(crate) struct ProgramRepinReceipt {
    pub(crate) disposition: RegistrationDisposition,
    pub(crate) revision: i64,
    pub(crate) program_digest: String,
    pub(crate) version: String,
}

const REPIN_MARKER: &[u8] = b"manual-program-repin";

fn framed(fields: &[&[u8]]) -> String {
    let mut bytes = Vec::new();
    for field in fields {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field);
    }
    encode_hex(&bytes)
}

fn repin_revision(connection: &VerifiedDatabaseConnection<'_>, id: &str) -> Result<i64, RegistryError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT revision FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
    row.bind_text(1, id)?;
    if !row.step_row()? { return Err(RegistryError::InstanceConflict); }
    Ok(row.column_text(0)?.parse().map_err(RegistryError::RevisionParse)?)
}

fn valid_repin(input: &ProgramRepin<'_>) -> Result<(), RegistryError> {
    if !valid_atom(input.request_id) || !valid_id(input.instance_id)
        || input.request_bytes.is_empty() || input.request_bytes.len() > 65_536
        || input.expected_revision < 1 || input.expected_revision == i64::MAX {
        return Err(RegistryError::Invalid("manual program repin"));
    }
    let request = crate::store::session_transport::decode_request(input.request_bytes)
        .map_err(|error| RegistryError::RepinFormat(format!("manual repin request: {error:?}")))?;
    if request.family != "K-INSTANCE" || request.operation != "repin-after-manual-upgrade"
        || request.domain_id != "global" || request.request_id != input.request_id
        || request.target_id != input.instance_id || !request.payload.is_empty()
        || request.expected_revision != input.expected_revision as u64 {
        return Err(RegistryError::Invalid("manual program repin wire identity"));
    }
    Ok(())
}

/// The old registration remains an immutable creation fact. A later native
/// repin must explain a changed current program without rewriting that fact.
fn has_program_repin_witness(connection: &VerifiedDatabaseConnection<'_>, id: &str,
    home: &str, old_digest: &str, old_version: &str, digest: &str, version: &str)
    -> Result<bool, RegistryError> {
    let revision = repin_revision(connection, id)?;
    let rows = Statement::prepare(connection.as_ptr(),
        "SELECT request_hex,receipt_json,request_id FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase='APPLIED' AND receipt_json IS NOT NULL")?;
    rows.bind_text(1, id)?;
    while rows.step_row()? {
        let fields = decode_framed_hex(&rows.column_text(0)?)?;
        if fields.first().map(Vec::as_slice) != Some(REPIN_MARKER) { continue; }
        if fields.len() != 10 || fields[2] != id.as_bytes() || fields[4] != old_digest.as_bytes()
            || fields[5] != old_version.as_bytes() || fields[6] != digest.as_bytes()
            || fields[7] != version.as_bytes() || fields[8] != home.as_bytes() { continue; }
        let number = |bytes: &[u8]| -> Result<i64, RegistryError> {
            std::str::from_utf8(bytes).map_err(|error| RegistryError::RepinFormat(format!("repin revision UTF-8: {error}")))?
                .parse().map_err(RegistryError::RevisionParse)
        };
        let before = number(&fields[3])?;
        let after = number(&fields[9])?;
        if before < 1 || before.checked_add(1) != Some(after) || after > revision { continue; }
        valid_repin(&ProgramRepin { request_id: &rows.column_text(2)?, request_bytes: &fields[1],
            instance_id: id, expected_revision: before })?;
        if rows.column_text(1)? != repin_receipt_json(after, home, digest, version, &rows.column_text(0)?) { continue; }
        return Ok(true);
    }
    Ok(false)
}

fn repin_receipt_json(revision: i64, home: &str, digest: &str, version: &str, fingerprint: &str) -> String {
    Json::Object([
        ("schema", "gogoke.37.instance-repin.v1".to_owned()),
        ("revision", revision.to_string()), ("homeIdentity", home.to_owned()),
        ("programDigest", digest.to_owned()), ("version", version.to_owned()),
        ("requestFingerprint", fingerprint.to_owned()),
    ].into_iter().map(|(key, value)| (JsonString::from_str(key),
        Json::String(JsonString::from_str(&value)))).collect()).canonical()
}

fn reconcile_creation(connection: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    id: &str) -> Result<(), RegistryError> {
    let rows = Statement::prepare(connection.as_ptr(),
        "SELECT request_id,request_hex FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase='APPLIED' AND native_receipt_id IS NOT NULL")?;
    rows.bind_text(1, id)?;
    if !rows.step_row()? { return Err(RegistryError::Unknown); }
    let request_id = rows.column_text(0)?;
    let fields = decode_framed_hex(&rows.column_text(1)?)?;
    if fields.len() != 5 || rows.step_row()? { return Err(RegistryError::Unknown); }
    if reconcile_register_replay(connection, root, &request_id, id, &fields[0])?
        != RegistrationReplay::Replayed { return Err(RegistryError::Unknown); }
    Ok(())
}

pub(crate) fn reconcile_program_repin(connection: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    input: &ProgramRepin<'_>) -> Result<Option<ProgramRepinReceipt>, RegistryError> {
    valid_repin(input)?;
    let Some(prior) = operation(connection, input.request_id)? else { return Ok(None); };
    let fields = decode_framed_hex(&prior.0)?;
    let expected = input.expected_revision.to_string();
    if fields.len() != 10 || fields[0] != REPIN_MARKER || fields[1] != input.request_bytes
        || fields[2] != input.instance_id.as_bytes() || fields[3] != expected.as_bytes()
        || prior.1 != input.instance_id { return Err(RegistryError::RequestConflict); }
    if prior.2 != "APPLIED" { return Err(RegistryError::Unknown); }
    let home = observed_home(root, input.instance_id)?.ok_or(RegistryError::IdentityChanged)?;
    let row = instance(connection, input.instance_id)?.ok_or(RegistryError::Unknown)?;
    let next = input.expected_revision.checked_add(1).ok_or(RegistryError::Unknown)?;
    if row.0 != "codex" || row.1 != format!("instance-home-{}", input.instance_id)
        || row.2 != home.opaque() || fields[8] != row.2.as_bytes()
        || fields[6] != row.3.as_bytes() || fields[7] != row.4.as_bytes()
        || fields[9] != next.to_string().as_bytes()
        || repin_revision(connection, input.instance_id)? < next { return Err(RegistryError::Unknown); }
    let receipt = Statement::prepare(connection.as_ptr(),
        "SELECT receipt_json FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    receipt.bind_text(1, input.request_id)?;
    if !receipt.step_row()? || receipt.column_text(0)? != repin_receipt_json(next, &row.2, &row.3, &row.4, &prior.0) {
        return Err(RegistryError::Unknown);
    }
    reconcile_creation(connection, root, input.instance_id)?;
    Ok(Some(ProgramRepinReceipt { disposition: RegistrationDisposition::Replayed,
        revision: next, program_digest: row.3, version: row.4 }))
}

/// Explicit User action after an external manual install. Only the native
/// catalog supplies the program; this function never installs or reads auth.
pub(crate) fn repin_program(connection: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    owner: &OwnerIssuer, input: &ProgramRepin<'_>, program: &ProgramObservation)
    -> Result<ProgramRepinReceipt, RegistryError> {
    valid_repin(input)?;
    transaction(connection, |connection| {
        check_owner_in_current_transaction(connection, owner)
            .map_err(|error| RegistryError::Authority(format!("{error:?}")))?;
        if let Some(replay) = reconcile_program_repin(connection, root, input)? { return Ok(replay); }
        if pending_target(connection, input.instance_id)? { return Err(RegistryError::Unknown); }
        let before = instance(connection, input.instance_id)?.ok_or(RegistryError::InstanceConflict)?;
        if repin_revision(connection, input.instance_id)? != input.expected_revision {
            return Err(RegistryError::InstanceConflict);
        }
        let home = observed_home(root, input.instance_id)?.ok_or(RegistryError::IdentityChanged)?;
        if before.0 != "codex" || before.1 != format!("instance-home-{}", input.instance_id)
            || before.2 != home.opaque() { return Err(RegistryError::IdentityChanged); }
        reconcile_creation(connection, root, input.instance_id)?;
        if before.3 == program.digest {
            return Err(RegistryError::Invalid("program pin unchanged"));
        }
        // Historical global login UNKNOWN is not a session reservation. It is
        // preserved, never promoted to a stop proof by this metadata operation.
        for sql in [
            "SELECT 1 FROM main.gogoke_v37_h_claim WHERE instance_id=?1 AND state!='RELEASED' LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_h_process_episode e LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE e.instance_id=?1 AND (e.phase NOT IN ('STOPPED','FAILED') OR (e.process_operation_id IS NOT NULL AND (e.stop_fact_id IS NULL OR c.state IS NULL OR c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR e.stop_fact_id!=c.stop_proof_hash))) LIMIT 1",
            "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id WHERE e.instance_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
        ] {
            let busy = Statement::prepare(connection.as_ptr(), sql)?;
            busy.bind_text(1, input.instance_id)?;
            if busy.step_row()? { return Err(RegistryError::InstanceConflict); }
        }
        let next = input.expected_revision + 1;
        let prior_revision = input.expected_revision.to_string();
        let next_revision = next.to_string();
        let hex = framed(&[REPIN_MARKER, input.request_bytes, input.instance_id.as_bytes(),
            prior_revision.as_bytes(), before.3.as_bytes(), before.4.as_bytes(),
            program.digest.as_bytes(), program.version.as_bytes(), before.2.as_bytes(), next_revision.as_bytes()]);
        let update = Statement::prepare(connection.as_ptr(),
            "UPDATE main.gogoke_v37_instances SET program_digest=?1,version=?2,login_state='UNKNOWN',revision=?3 WHERE instance_id=?4 AND revision=?5 AND home_identity=?6 AND program_digest=?7 AND version=?8")?;
        for (index, value) in [(1, program.digest.as_str()), (2, program.version.as_str()), (4, input.instance_id),
            (6, before.2.as_str()), (7, before.3.as_str()), (8, before.4.as_str())] { update.bind_text(index, value)?; }
        update.bind_i64(3, next)?;
        update.bind_i64(5, input.expected_revision)?;
        update.step_done()?;
        let after = instance(connection, input.instance_id)?.ok_or(RegistryError::Unknown)?;
        if after.2 != before.2 || after.3 != program.digest || after.4 != program.version
            || repin_revision(connection, input.instance_id)? != next
            || observed_home(root, input.instance_id)? != Some(home) { return Err(RegistryError::Unknown); }
        let journal = Statement::prepare(connection.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase,receipt_json) VALUES(?1,?2,?3,'APPLIED',?4)")?;
        journal.bind_text(1, input.request_id)?; journal.bind_text(2, &hex)?;
        journal.bind_text(3, input.instance_id)?;
        journal.bind_text(4, &repin_receipt_json(next, &before.2, &program.digest, &program.version, &hex))?;
        journal.step_done()?;
        Ok(ProgramRepinReceipt { disposition: RegistrationDisposition::Applied,
            revision: next, program_digest: program.digest.clone(), version: program.version.clone() })
    })
}

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

pub(super) fn valid_id(value: &str) -> bool {
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
        "SELECT request_hex,target_id,phase,COALESCE(native_receipt_id,'') FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    row.bind_text(1, request_id)?;
    if row.step_row()? { Ok(Some((row.column_text(0)?, row.column_text(1)?, row.column_text(2)?, row.column_text(3)?))) }
    else { Ok(None) }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationJournalPhase { Preparing, Unknown, Applied, Denied, Failed }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationPreflight {
    Unseen,
    MatchingPrior(RegistrationJournalPhase),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegistrationReplay { Unseen, Pending, Replayed }

fn decode_framed_hex(value: &str) -> Result<Vec<Vec<u8>>, RegistryError> {
    if value.is_empty() || value.len() > 262_144 || value.len() % 2 != 0 {
        return Err(RegistryError::Unknown);
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().chunks_exact(2) {
        let high = (pair[0] as char).to_digit(16).ok_or(RegistryError::Unknown)?;
        let low = (pair[1] as char).to_digit(16).ok_or(RegistryError::Unknown)?;
        bytes.push(((high << 4) | low) as u8);
    }
    let mut fields = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        if fields.len() == 16 || bytes.len() - offset < 8 {
            return Err(RegistryError::Unknown);
        }
        let length = u64::from_be_bytes(bytes[offset..offset + 8].try_into()
            .map_err(|_| RegistryError::Unknown)?);
        offset += 8;
        let length = usize::try_from(length).map_err(|_| RegistryError::Unknown)?;
        if length > 65_536 || length > bytes.len() - offset {
            return Err(RegistryError::Unknown);
        }
        fields.push(bytes[offset..offset + length].to_vec());
        offset += length;
    }
    if fields.is_empty() { return Err(RegistryError::Unknown); }
    Ok(fields)
}

/// Read-only request-ID preflight before catalog lookup, version checks or
/// revision checks. `request_hex` already contains the exact raw request as
/// the first length-framed field; no new schema or migration is necessary.
/// MatchingPrior is only a replay candidate, never a replay receipt: the
/// native home and full operation fingerprint still require reconciliation.
pub(crate) fn preflight_register_request(connection: &VerifiedDatabaseConnection<'_>,
    request_id: &str, instance_id: &str, request_bytes: &[u8])
    -> Result<RegistrationPreflight, RegistryError> {
    if !valid_atom(request_id) || !valid_id(instance_id)
        || request_bytes.is_empty() || request_bytes.len() > 65_536 {
        return Err(RegistryError::Invalid("registration preflight"));
    }
    // Bound the amount copied from a damaged journal and qualify main so a
    // TEMP shadow cannot change the answer after schema initialization.
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT CASE WHEN length(request_hex)<=262144 THEN request_hex ELSE '' END,target_id,phase \
         FROM main.gogoke_v37_instance_operations WHERE request_id=?1")?;
    row.bind_text(1, request_id)?;
    if !row.step_row()? { return Ok(RegistrationPreflight::Unseen); }
    let stored = row.column_text(0)?;
    let target = row.column_text(1)?;
    let phase = row.column_text(2)?;
    let fields = decode_framed_hex(&stored)?;
    let first = fields.first().ok_or(RegistryError::Unknown)?;
    if first.as_slice() != request_bytes || target != instance_id {
        return Err(RegistryError::RequestConflict);
    }
    // A registration fingerprint has exactly five framed fields, with the
    // target repeated inside the durable fingerprint. Other F operations use
    // distinct first-field markers and cannot become registration replays.
    if fields.len() != 5 || fields.get(1).map(Vec::as_slice) != Some(instance_id.as_bytes()) {
        return Err(RegistryError::Unknown);
    }
    let phase = match phase.as_str() {
        "PREPARING" => RegistrationJournalPhase::Preparing,
        "UNKNOWN" => RegistrationJournalPhase::Unknown,
        "APPLIED" => RegistrationJournalPhase::Applied,
        "DENIED" => RegistrationJournalPhase::Denied,
        "FAILED" => RegistrationJournalPhase::Failed,
        _ => return Err(RegistryError::Unknown),
    };
    Ok(RegistrationPreflight::MatchingPrior(phase))
}

/// Reconcile an exact prior registration without consulting the currently
/// installed CLI. APPLIED alone is insufficient: the durable fingerprint,
/// registered row, physical home and native receipt must all still agree.
pub(crate) fn reconcile_register_replay(connection: &VerifiedDatabaseConnection<'_>,
    root: &RootLock, request_id: &str, instance_id: &str, request_bytes: &[u8])
    -> Result<RegistrationReplay, RegistryError> {
    match preflight_register_request(connection, request_id, instance_id, request_bytes)? {
        RegistrationPreflight::Unseen => return Ok(RegistrationReplay::Unseen),
        RegistrationPreflight::MatchingPrior(RegistrationJournalPhase::Preparing |
            RegistrationJournalPhase::Unknown) => return Ok(RegistrationReplay::Pending),
        RegistrationPreflight::MatchingPrior(RegistrationJournalPhase::Denied |
            RegistrationJournalPhase::Failed) => return Err(RegistryError::Unknown),
        RegistrationPreflight::MatchingPrior(RegistrationJournalPhase::Applied) => (),
    }
    let prior = operation(connection, request_id)?.ok_or(RegistryError::Unknown)?;
    if prior.1 != instance_id || prior.2 != "APPLIED" {
        return Err(RegistryError::Unknown);
    }
    let home = observed_home(root, instance_id)?.ok_or(RegistryError::Unknown)?;
    if prior.3 != home.opaque() { return Err(RegistryError::Unknown); }
    let (driver, home_ref, identity, digest, version, _, _) =
        instance(connection, instance_id)?.ok_or(RegistryError::Unknown)?;
    if home_ref != format!("instance-home-{instance_id}") || identity != home.opaque() {
        return Err(RegistryError::Unknown);
    }
    let fields = decode_framed_hex(&prior.0)?;
    if fields.len() != 5 { return Err(RegistryError::Unknown); }
    let text = |bytes: &[u8]| -> Result<String, RegistryError> {
        String::from_utf8(bytes.to_vec()).map_err(|error|
            RegistryError::RepinFormat(format!("creation program UTF-8: {error}")))
    };
    let observed_at_commit = ProgramObservation { digest: text(&fields[3])?, version: text(&fields[4])? };
    let original = Registration { request_id, request_bytes, instance_id,
        driver_id: &driver, program: &observed_at_commit };
    if prior.0 != fingerprint(&original)? || operation(connection, request_id)? != Some(prior) {
        return Err(RegistryError::Unknown);
    }
    if !observed_at_commit.matches_pin(&digest, &version)
        && !has_program_repin_witness(connection, instance_id, &identity,
            &observed_at_commit.digest, &observed_at_commit.version, &digest, &version)? {
        return Err(RegistryError::Unknown);
    }
    if observed_home(root, instance_id)? != Some(home) {
        return Err(RegistryError::Unknown);
    }
    Ok(RegistrationReplay::Replayed)
}

fn pending_target(connection: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<bool, RegistryError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT request_id FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND phase IN ('PREPARING','UNKNOWN') LIMIT 1")?;
    row.bind_text(1, instance_id)?;
    row.step_row().map_err(Into::into)
}

fn instance(connection: &VerifiedDatabaseConnection<'_>, instance_id: &str)
    -> Result<Option<(String, String, String, String, String, String, String)>, RegistryError> {
    let row = Statement::prepare(connection.as_ptr(),
        "SELECT driver_id,home_ref,home_identity,program_digest,version,install_state,login_state FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
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

pub(super) fn observed_home(root: &RootLock, instance_id: &str) -> Result<Option<RootIdentity>, RegistryError> {
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

/// Resolve the physical home recorded for a Codex instance. This is an
/// internal launch seam: the database row, the v37 marker, and the current
/// physical directory must all agree before a path is returned.
pub(super) fn resolve_registered_codex_home(
    connection: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    instance_id: &str,
) -> Result<(PathBuf, RootIdentity), RegistryError> {
    if !valid_id(instance_id) {
        return Err(RegistryError::Invalid("instance_id"));
    }
    let Some((driver, home_ref, recorded_identity, _, _, _, _)) =
        instance(connection, instance_id)?
    else {
        return Err(RegistryError::Unknown);
    };
    if driver != "codex" || home_ref != format!("instance-home-{instance_id}")
        || recorded_identity.is_empty()
    {
        return Err(RegistryError::Unknown);
    }
    let observed = observed_home(root, instance_id)?.ok_or(RegistryError::Unknown)?;
    if observed.opaque() != recorded_identity {
        return Err(RegistryError::IdentityChanged);
    }
    let path = root
        .canonical_root()
        .canonical_path
        .join(CONTAINER)
        .join(instance_id);
    let current = checked_identity(&path)?;
    if current != observed {
        return Err(RegistryError::IdentityChanged);
    }
    // A legacy nested temporary container must never receive the persistent
    // home's recursive launch grant. Preserve it and refuse this instance;
    // neither readback nor a new request migrates or deletes its contents.
    match fs::symlink_metadata(path.join("temporary-homes")) {
        Ok(_) => return Err(RegistryError::Invalid("legacy nested temporary-home layout")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (),
        Err(error) => return Err(RegistryError::Io(error)),
    }
    Ok((path, current))
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

    fn repin_setup(connection: &mut VerifiedDatabaseConnection<'_>, root: &RootLock)
        -> (OwnerIssuer, ProgramObservation, ProgramObservation) {
        let owner = crate::store::authority::initialize_profile(connection, root).unwrap();
        crate::store::authority::initialize_process_custody_schema(connection).unwrap();
        crate::store::seat::initialize_schema(connection).unwrap();
        crate::store::session_transport::initialize_admission_schema(connection).unwrap();
        let old = program(root);
        register_instance(connection, root, &registration("creation", b"original registration", "instanceA", &old)).unwrap();
        let path = root.canonical_root().canonical_path.join("new-cli.bin");
        fs::write(&path, b"new owned program bytes").unwrap();
        let new = ProgramObservation::observe(&path, "0.160.0").unwrap();
        (owner, old, new)
    }

    fn upgrade_request<'a>(id: &'a str, raw: &'a [u8], revision: i64) -> ProgramRepin<'a> {
        ProgramRepin { request_id: id, request_bytes: raw, instance_id: "instanceA", expected_revision: revision }
    }

    fn upgrade_bytes(id: &str, revision: i64) -> Vec<u8> {
        format!("{{\"schema\":\"gogoke.37.operations.v1\",\"family\":\"K-INSTANCE\",\"operation\":\"repin-after-manual-upgrade\",\"requestId\":\"{id}\",\"targetId\":\"instanceA\",\"domainId\":\"global\",\"expectedRevision\":\"{revision}\",\"payload\":{{}}}}").into_bytes()
    }

    fn custody_row(connection: &VerifiedDatabaseConnection<'_>, id: &str) -> Vec<String> {
        let row = Statement::prepare(connection.as_ptr(),
            "SELECT operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,COALESCE(stop_proof_hash,'') FROM main.gogoke_coordination_process_custody WHERE operation_id=?1").unwrap();
        row.bind_text(1, id).unwrap();
        assert!(row.step_row().unwrap());
        (0..12).map(|index| row.column_text(index).unwrap()).collect()
    }

    #[test]
    fn manual_repin_preserves_home_creation_credentials_and_legacy_unknown_exactly() {
        fixture(|connection, root| {
            let (owner, old, new) = repin_setup(connection, root);
            let before_home = observed_home(root, "instanceA").unwrap();
            let before_creation = operation(connection, "creation").unwrap();
            let home = root.canonical_root().canonical_path.join(CONTAINER).join("instanceA");
            let credential = home.join("credential-fixture.txt");
            fs::write(&credential, b"synthetic credential bytes, never real auth").unwrap();
            connection.execute("UPDATE main.gogoke_v37_instances SET login_state='LOGGED_IN'").unwrap();
            connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state) VALUES('legacy','ticketOld','nonceOld','123','456','synthetic-cli','old-digest','instanceA','global','2','UNKNOWN')").unwrap();
            let legacy = custody_row(connection, "legacy");
            let raw = upgrade_bytes("upgradeA", 1);
            let input = upgrade_request("upgradeA", &raw, 1);
            let receipt = repin_program(connection, root, &owner, &input, &new).unwrap();
            assert_eq!((receipt.disposition, receipt.revision), (RegistrationDisposition::Applied, 2));
            assert_eq!(observed_home(root, "instanceA").unwrap(), before_home);
            assert_eq!(operation(connection, "creation").unwrap(), before_creation);
            assert_eq!(fs::read(&credential).unwrap(), b"synthetic credential bytes, never real auth");
            assert_eq!(custody_row(connection, "legacy"), legacy);
            let row = instance(connection, "instanceA").unwrap().unwrap();
            assert_eq!((&row.3, &row.4, row.6.as_str()), (&new.digest, &new.version, "UNKNOWN"));
            assert_eq!(reconcile_register_replay(connection, root, "creation", "instanceA", b"original registration").unwrap(), RegistrationReplay::Replayed);
            assert_eq!(reconcile_program_repin(connection, root, &input).unwrap().unwrap().disposition, RegistrationDisposition::Replayed);
            assert_eq!(repin_program(connection, root, &owner, &input, &old).unwrap().revision, 2);
            assert_eq!(repin_revision(connection, "instanceA").unwrap(), 2);
            let mut changed = raw.clone(); changed.push(b'\n');
            assert!(matches!(repin_program(connection, root, &owner,
                &upgrade_request("upgradeA", &changed, 1), &new), Err(RegistryError::RequestConflict)));
            assert!(matches!(repin_program(connection, root, &owner,
                &upgrade_request("upgradeB", &upgrade_bytes("upgradeB", 1), 1), &new), Err(RegistryError::InstanceConflict)));
            assert!(matches!(repin_program(connection, root, &owner,
                &upgrade_request("upgradeC", &upgrade_bytes("upgradeC", 2), 2), &new), Err(RegistryError::Invalid(_))));
            assert_eq!(repin_revision(connection, "instanceA").unwrap(), 2);
        });
    }

    #[test]
    fn manual_repin_rejects_pending_claim_episode_and_generation_change_without_writes() {
        fixture(|connection, root| {
            let (owner, _, new) = repin_setup(connection, root);
            let before = instance(connection, "instanceA").unwrap();
            let raw = upgrade_bytes("upgradeA", 1);
            let input = upgrade_request("upgradeA", &raw, 1);
            connection.execute("INSERT INTO main.gogoke_v37_h_owner_binding VALUES('bindingA','instanceA','projectA','SESSION','sessionA','1','ACTIVE')").unwrap();
            connection.execute("INSERT INTO main.gogoke_v37_h_claim(domain_id,session_id,instance_id,home_id,binding_id,generation,state,revision) VALUES('projectA','sessionA','instanceA','homeA','bindingA','1','STOPPED',1)").unwrap();
            assert!(matches!(repin_program(connection, root, &owner, &input, &new), Err(RegistryError::InstanceConflict)));
            connection.execute("UPDATE main.gogoke_v37_h_claim SET state='RELEASED'").unwrap();
            connection.execute("INSERT INTO main.gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,raw_hex,previous_revision,instance_id,home_id,binding_id,phase) VALUES('projectA','openA','sessionA','1','00',1,'instanceA','homeA','bindingA','UNKNOWN')").unwrap();
            assert!(matches!(repin_program(connection, root, &owner, &input, &new), Err(RegistryError::InstanceConflict)));
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET phase='FAILED'").unwrap();
            connection.execute("INSERT INTO main.gogoke_v37_h_generation_change(domain_id,request_id,raw_hex,operation,session_id,old_generation,old_process_operation_id,old_ticket,old_nonce,thread_id,seat_id,previous_revision,source_watermark,stage) VALUES('projectA','changeA','00','compact','sessionA','1','processA','ticketA','nonceA','threadA','seatA',1,0,'INTENT')").unwrap();
            connection.execute("INSERT INTO main.gogoke_coordination_process_custody(operation_id,ticket,custodian_nonce,pid,creation_time_100ns,image_path,binary_digest_sha256,profile_id,domain_id,generation,state,stop_proof_hash) VALUES('processA','ticketA','nonceA','123','456','synthetic-cli','digestA','instanceA','projectA','1','STOPPED','proofA')").unwrap();
            connection.execute("UPDATE main.gogoke_v37_h_process_episode SET phase='STOPPED',process_operation_id='processA',stop_fact_id='proofA'").unwrap();
            assert!(matches!(repin_program(connection, root, &owner, &input, &new), Err(RegistryError::InstanceConflict)));
            connection.execute("UPDATE main.gogoke_v37_h_generation_change SET stage='APPLIED'").unwrap();
            connection.execute("UPDATE main.gogoke_coordination_process_custody SET state='UNKNOWN'").unwrap();
            assert!(matches!(repin_program(connection, root, &owner, &input, &new), Err(RegistryError::InstanceConflict)));
            assert_eq!(instance(connection, "instanceA").unwrap(), before);
            assert!(operation(connection, "upgradeA").unwrap().is_none());
            connection.execute("UPDATE main.gogoke_coordination_process_custody SET state='STOPPED'").unwrap();
            assert_eq!(repin_program(connection, root, &owner, &input, &new).unwrap().disposition, RegistrationDisposition::Applied);
        });
    }

    #[test]
    fn manual_repin_does_not_legitimize_changed_creation_pin_or_damaged_witness() {
        fixture(|connection, root| {
            let (owner, old, new) = repin_setup(connection, root);
            let raw = upgrade_bytes("upgradeA", 1);
            let input = upgrade_request("upgradeA", &raw, 1);
            connection.execute("UPDATE main.gogoke_v37_instances SET version='tampered'").unwrap();
            assert!(matches!(repin_program(connection, root, &owner, &input, &new), Err(RegistryError::Unknown)));
            assert!(operation(connection, "upgradeA").unwrap().is_none());
            let restore = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_instances SET version=?1").unwrap();
            restore.bind_text(1, &old.version).unwrap(); restore.step_done().unwrap();
            repin_program(connection, root, &owner, &input, &new).unwrap();
            let witness = operation(connection, "upgradeA").unwrap().unwrap();
            let mut fields = decode_framed_hex(&witness.0).unwrap();
            fields[1].push(b'\n');
            let changed = framed(&fields.iter().map(Vec::as_slice).collect::<Vec<_>>());
            let corrupt = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id='upgradeA'").unwrap();
            corrupt.bind_text(1, &changed).unwrap(); corrupt.step_done().unwrap();
            assert!(matches!(reconcile_register_replay(connection, root, "creation", "instanceA", b"original registration"), Err(RegistryError::Unknown)));
            // Use a new statement: the completed one has no implicit replay.
            let restore_witness = Statement::prepare(connection.as_ptr(), "UPDATE main.gogoke_v37_instance_operations SET request_hex=?1 WHERE request_id='upgradeA'").unwrap();
            restore_witness.bind_text(1, &witness.0).unwrap(); restore_witness.step_done().unwrap();
            connection.execute("UPDATE main.gogoke_v37_instance_operations SET receipt_json='{}' WHERE request_id='upgradeA'").unwrap();
            assert!(matches!(reconcile_program_repin(connection, root, &input), Err(RegistryError::Unknown)));
            assert!(matches!(reconcile_register_replay(connection, root, "creation", "instanceA", b"original registration"), Err(RegistryError::Unknown)));
        });
    }

    #[test]
    fn durable_register_and_exact_replay_reject_changed_bytes() {
        fixture(|connection, root| {
            let observed = program(root);
            let input = registration("req-1", b"register one", "instanceA", &observed);
            assert_eq!(preflight_register_request(connection, "req-1", "instanceA", b"register one").unwrap(),
                RegistrationPreflight::Unseen);
            assert_eq!(register_instance(connection, root, &input).unwrap(), RegistrationDisposition::Applied);
            assert_eq!(preflight_register_request(connection, "req-1", "instanceA", b"register one").unwrap(),
                RegistrationPreflight::MatchingPrior(RegistrationJournalPhase::Applied));
            assert!(matches!(preflight_register_request(connection, "req-1", "instanceA",
                b"different driver in raw request"), Err(RegistryError::RequestConflict)));
            assert!(matches!(preflight_register_request(connection, "req-1", "instanceB",
                b"register one"), Err(RegistryError::RequestConflict)));
            assert_eq!(register_instance(connection, root, &input).unwrap(), RegistrationDisposition::Replayed);
            let changed = registration("req-1", b"register two", "instanceA", &observed);
            assert!(matches!(register_instance(connection, root, &changed), Err(RegistryError::RequestConflict)));
            let other = registration("req-2", b"register one", "instanceA", &observed);
            assert!(matches!(register_instance(connection, root, &other), Err(RegistryError::InstanceConflict)));
        });
    }

    #[test]
    fn preflight_reads_legacy_framed_request_without_catalog_and_rejects_corrupt_journal() {
        fixture(|connection, root| {
            let observed = program(root);
            let raw = br#"{"driverId":"codex"}"#;
            let input = registration("legacy-1", raw, "instanceA", &observed);
            let insert = Statement::prepare(connection.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_operations(request_id,request_hex,target_id,phase) VALUES(?1,?2,?3,'PREPARING')").unwrap();
            insert.bind_text(1, input.request_id).unwrap();
            insert.bind_text(2, &fingerprint(&input).unwrap()).unwrap();
            insert.bind_text(3, input.instance_id).unwrap();
            insert.step_done().unwrap();
            assert_eq!(preflight_register_request(connection, "legacy-1", "instanceA", raw).unwrap(),
                RegistrationPreflight::MatchingPrior(RegistrationJournalPhase::Preparing));
            assert_eq!(reconcile_register_replay(connection, root, "legacy-1", "instanceA", raw).unwrap(),
                RegistrationReplay::Pending);
            assert!(matches!(preflight_register_request(connection, "legacy-1", "instanceA", br#"{"driverId":"unknown"}"#),
                Err(RegistryError::RequestConflict)));
            assert!(matches!(preflight_register_request(connection, "legacy-1", "instanceA", br#"{ "driverId":"codex" }"#),
                Err(RegistryError::RequestConflict)));
            connection.execute("UPDATE main.gogoke_v37_instance_operations SET request_hex='not-hex' WHERE request_id='legacy-1'").unwrap();
            assert!(matches!(preflight_register_request(connection, "legacy-1", "instanceA", raw),
                Err(RegistryError::Unknown)));
        });
    }

    #[test]
    fn exact_replay_survives_missing_cli_but_requires_original_home_and_row() {
        fixture(|connection, root| {
            let observed = program(root);
            let input = registration("replay-1", b"same raw bytes", "instanceA", &observed);
            assert_eq!(reconcile_register_replay(connection, root, input.request_id,
                input.instance_id, input.request_bytes).unwrap(), RegistrationReplay::Unseen);
            register_instance(connection, root, &input).unwrap();
            fs::remove_file(root.canonical_root().canonical_path.join("observed-cli.bin")).unwrap();
            assert_eq!(reconcile_register_replay(connection, root, input.request_id,
                input.instance_id, input.request_bytes).unwrap(), RegistrationReplay::Replayed);
            assert!(matches!(reconcile_register_replay(connection, root, input.request_id,
                input.instance_id, b"changed raw bytes"), Err(RegistryError::RequestConflict)));
            connection.execute("UPDATE main.gogoke_v37_instances SET program_digest='sha256:0000000000000000000000000000000000000000000000000000000000000000' WHERE instance_id='instanceA'").unwrap();
            assert!(matches!(reconcile_register_replay(connection, root, input.request_id,
                input.instance_id, input.request_bytes), Err(RegistryError::Unknown)));
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
