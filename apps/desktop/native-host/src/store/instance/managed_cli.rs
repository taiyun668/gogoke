//! Product-owned fixed official CLI copies. A staged copy is never READY.
//! The native host checks the original archive and executable again before
//! recording it; H must launch that exact image and report the result before
//! the PROBED copy can be bound to every quiescent legacy instance and marked READY.

use super::{legacy_fence,registry::{self,ProgramObservation}};
use crate::process::NativeLegacyHoldersGone;
use crate::root::RootLock;
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
use std::fs::{self, File};
use std::io::{self, Read};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};

const REPARSE_POINT: u32 = 0x400;
const CONTAINER: &str = "v37-managed-cli";
const STAGING: &str = "staging";

#[derive(Clone, Copy)]
pub(crate) struct VerifiedOfficialCli {
    pub(crate) driver: &'static str,
    pub(crate) version: &'static str,
    pub(crate) archive_sha256: &'static str,
    pub(crate) image_sha256: &'static str,
    pub(crate) image_relative: &'static str,
    pub(crate) raw_image: bool,
}

const OFFICIAL: [VerifiedOfficialCli; 8] = [
    VerifiedOfficialCli { driver: "codex", version: "0.160.0",
        archive_sha256: "1f4c46470317bf5fabb308ed3e39a80ff544069cedc2cb477005545e694058b9",
        image_sha256: "fdda5fa3cf3fb3d000b876720742857676293e4315e4b045fae6f8bd7e866d1d",
        image_relative: "package/vendor/x86_64-pc-windows-msvc/bin/codex.exe", raw_image: false },
    VerifiedOfficialCli { driver: "claude", version: "2.1.196",
        archive_sha256: "15f050721450ab208caaf2351e39aa1326c320d8fc071fb1ef5e176c85a3a9d1",
        image_sha256: "180d7b279455e8b89d4353a5146447be2f80b80fb0db14bdc6dd9cb98c0aef09",
        image_relative: "package/claude.exe", raw_image: false },
    VerifiedOfficialCli { driver: "opencode", version: "1.18.32",
        archive_sha256: "701f23388207a4d2e2ff46cda46e707474fa01cf2a1a1afa404f045c06e5eaa4",
        image_sha256: "cf664aa1da32b788f9b2699b84a9bb9be30b7e025693b90f9b85829d5fe4e252",
        image_relative: "package/bin/opencode.exe", raw_image: false },
    VerifiedOfficialCli { driver: "opencode", version: "1.18.31",
        archive_sha256: "8a300ee2b3210ccc6894ff27d21bdb0a60ee44107389c49b743b4caed10aa5cf",
        image_sha256: "0242a0dc705af67c90882b456a36b619883c1c786aad8fe071a1bc64e5d1d440",
        image_relative: "package/bin/opencode.exe", raw_image: false },
    VerifiedOfficialCli { driver: "opencode", version: "1.18.33",
        archive_sha256: "fc76bd4a0f258027594f6a8900e220fd06f7345cb5e67f73dc3b4f53ba3f580e",
        image_sha256: "52f60248a576b34c9a6dcaa27e0a7f08089af35bcdc0dfb10c04d3e00a98314c",
        image_relative: "package/bin/opencode.exe", raw_image: false },
    VerifiedOfficialCli { driver: "grok", version: "1.0.41",
        archive_sha256: "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        image_sha256: "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        image_relative: "grok.exe", raw_image: true },
    VerifiedOfficialCli { driver: "codex", version: "0.162.1",
        archive_sha256: "5d48824f11a512b0ecb351461c2e90a1040c80de1026bc4a80815d5121f3936d",
        image_sha256: "dd13bdb162a184fa04039488b234a8258ef9b76f84e926c505d5742b19249a6a",
        image_relative: "package/vendor/x86_64-pc-windows-msvc/bin/codex.exe", raw_image: false },
    VerifiedOfficialCli { driver: "claude", version: "2.1.299",
        archive_sha256: "3fed53036bdb0b501da290ff10cc3d8eafbfdf8852d8c0fbb0e7e9e4e0aa53c8",
        image_sha256: "295b49ec092f31a7de9c6ff286d9d8f120a4f029b9ad9a5ac0b12c35cbf4df2b",
        image_relative: "package/claude.exe", raw_image: false },
];

pub(crate) fn read_fixed_official_cli(driver: &str) -> Option<VerifiedOfficialCli> {
    OFFICIAL.iter().copied().find(|pin| pin.driver == driver)
}

/// A version is installable only when its archive and image are compiled into
/// the trusted catalog. An official release notice never becomes a pin.
pub(crate) fn read_official_cli_version(driver: &str, version: &str) -> Option<VerifiedOfficialCli> {
    OFFICIAL.iter().copied().find(|pin| pin.driver == driver && pin.version == version)
}

#[derive(Debug)]
pub(crate) enum ManagedCliError {
    Unsupported,
    Invalid,
    IdentityChanged,
    Busy,
    Observation(String),
    Io(io::Error),
    Store(OrchestrationError),
    Legacy(String),
    NativeGone(String),
}
impl From<io::Error> for ManagedCliError { fn from(error: io::Error) -> Self { Self::Io(error) } }
impl From<crate::store::atomic::AtomicError> for ManagedCliError {
    fn from(error: crate::store::atomic::AtomicError) -> Self {
        Self::Store(OrchestrationError::Atomic(error))
    }
}
impl From<crate::store::same_open::SameOpenError> for ManagedCliError {
    fn from(error: crate::store::same_open::SameOpenError) -> Self {
        Self::Store(OrchestrationError::Atomic(error.into()))
    }
}
impl From<OrchestrationError> for ManagedCliError {
    fn from(error: OrchestrationError) -> Self { Self::Store(error) }
}

pub(crate) struct ManagedCliCopy {
    pub(crate) driver: String,
    pub(crate) state: String,
    pub(crate) version: Option<String>,
    pub(crate) stage_name: Option<String>,
    pub(crate) previous_version: Option<String>,
    pub(crate) previous_image_sha256: Option<String>,
    pub(crate) previous_stage_name: Option<String>,
    pub(crate) verified_other_versions: Vec<String>,
    pub(crate) progress_bytes: i64,
    pub(crate) raw_error: Option<String>,
    pub(crate) checked_at: Option<String>,
    pub(crate) official_notice: Option<String>,
    pub(crate) revision: i64,
}

fn plain_dir(path: &Path) -> Result<(), ManagedCliError> {
    let item = fs::symlink_metadata(path)?;
    if !item.is_dir() || item.file_attributes() & REPARSE_POINT != 0 {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(())
}

fn create_plain_dir(path: &Path) -> Result<(), ManagedCliError> {
    match fs::create_dir(path) {
        Ok(()) => (),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => (),
        Err(error) => return Err(error.into()),
    }
    plain_dir(path)
}

/// Root returned to the signed Node service for a fixed official download.
/// The native product root pin remains the authority for this directory.
pub(crate) fn managed_cli_root(root: &RootLock) -> Result<PathBuf, ManagedCliError> {
    let base = &root.canonical_root().canonical_path;
    plain_dir(base)?;
    if crate::root::inspect_root(base).map_err(|error|
        ManagedCliError::Observation(format!("managed root observation: {error:?}")))?.identity
        != root.canonical_root().identity { return Err(ManagedCliError::IdentityChanged); }
    let managed = base.join(CONTAINER);
    create_plain_dir(&managed)?;
    let staging = managed.join(STAGING);
    create_plain_dir(&staging)?;
    Ok(staging)
}

fn sha256_file(path: &Path, expected: &str) -> Result<(), ManagedCliError> {
    let before = fs::symlink_metadata(path)?;
    if !before.is_file() || before.file_attributes() & REPARSE_POINT != 0 {
        return Err(ManagedCliError::IdentityChanged);
    }
    let mut file = File::open(path)?;
    let opened = file.metadata()?;
    if !opened.is_file() || opened.file_attributes() & REPARSE_POINT != 0 ||
        opened.len() != before.len() { return Err(ManagedCliError::IdentityChanged); }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    let path_after = fs::symlink_metadata(path)?;
    if after.len() != opened.len() || after.modified()? != opened.modified()? ||
        path_after.len() != before.len() || path_after.modified()? != before.modified()? ||
        path_after.file_attributes() & REPARSE_POINT != 0 {
        return Err(ManagedCliError::IdentityChanged);
    }
    use sha2::{Digest, Sha256};
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(())
}

fn safe_stage_name(name: &str, pin: VerifiedOfficialCli) -> bool {
    let prefix = format!("{}-{}-", pin.driver, pin.version);
    name.starts_with(&prefix) && name.len() > prefix.len() && name.len() <= 128 &&
        name.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.')) &&
        !name.contains("..")
}

/// Validate the exact private stage and original archive; no user global
/// npm, PATH, installer script, or credential path is touched.
pub(crate) fn inspect_staged_official_cli(root: &RootLock, driver: &str,
    stage_name: &str) -> Result<PathBuf, ManagedCliError> {
    let pin = read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    inspect_stage_at(&managed_cli_root(root)?, pin, stage_name)
}

pub(crate) fn inspect_staged_official_cli_version(root: &RootLock, driver: &str,
    version: &str, stage_name: &str) -> Result<PathBuf, ManagedCliError> {
    let pin = read_official_cli_version(driver, version).ok_or(ManagedCliError::Unsupported)?;
    inspect_stage_at(&managed_cli_root(root)?, pin, stage_name)
}

fn inspect_stage_at(staging: &Path, pin: VerifiedOfficialCli, stage_name: &str)
    -> Result<PathBuf, ManagedCliError> {
    if !safe_stage_name(stage_name, pin) { return Err(ManagedCliError::Invalid); }
    plain_dir(staging)?;
    let stage = staging.join(stage_name);
    plain_dir(&stage)?;
    let content = stage.join("content");
    plain_dir(&content)?;
    let image = pin.image_relative.split('/').fold(content.clone(), |path, part| path.join(part));
    let mut parent = content;
    let components: Vec<_> = pin.image_relative.split('/').collect();
    for part in &components[..components.len() - 1] {
        parent.push(part);
        plain_dir(&parent)?;
    }
    if !pin.raw_image { sha256_file(&stage.join("source.download"), pin.archive_sha256)?; }
    sha256_file(&image, pin.image_sha256)?;
    let observation = ProgramObservation::observe(&image, pin.version)
        .map_err(|error| ManagedCliError::Observation(format!("managed program observation: {error:?}")))?;
    if !observation.matches_pin(&format!("sha256:{}", pin.image_sha256), pin.version) {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(image)
}

fn verified_staging_from_db(db: &VerifiedDatabaseConnection<'_>)
    -> Result<PathBuf, ManagedCliError> {
    let root = db.path().parent().ok_or(ManagedCliError::IdentityChanged)?;
    plain_dir(root)?;
    if crate::root::inspect_root(root).map_err(|_|ManagedCliError::IdentityChanged)?.identity
        != *db.root_identity() { return Err(ManagedCliError::IdentityChanged); }
    let managed=root.join(CONTAINER);
    plain_dir(&managed)?;
    let staging=managed.join(STAGING);
    plain_dir(&staging)?;
    Ok(staging)
}

pub(crate) fn locate_ready_managed_program_from_db(db: &VerifiedDatabaseConnection<'_>,
    driver: &str, digest: &str, version: &str) -> Result<Option<PathBuf>, ManagedCliError> {
    let pin=read_official_cli_version(driver,version).ok_or(ManagedCliError::Unsupported)?;
    let row=Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(stage_name,''),COALESCE(image_sha256,''),COALESCE(version,''),COALESCE(archive_sha256,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1,driver)?;
    if !row.step_row()? { return Ok(None); }
    let state=row.column_text(0)?;
    let name=row.column_text(1)?;
    let stored_digest=row.column_text(2)?;
    let stored_version=row.column_text(3)?;
    let stored_archive=row.column_text(4)?;
    if row.step_row()? || state!="READY" || stored_digest!=pin.image_sha256 ||
        stored_version!=pin.version || stored_archive!=pin.archive_sha256 || version!=pin.version ||
        digest!=format!("sha256:{}",pin.image_sha256) {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(Some(inspect_stage_at(&verified_staging_from_db(db)?,pin,&name)?))
}

fn transaction<T>(db: &mut VerifiedDatabaseConnection<'_>,
    action: impl FnOnce(&VerifiedDatabaseConnection<'_>) -> Result<T, ManagedCliError>)
    -> Result<T, ManagedCliError> {
    db.execute("BEGIN IMMEDIATE")?;
    match action(db) {
        Ok(value) => {
            db.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?;
            Ok(value)
        },
        Err(error) => {
            db.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        },
    }
}

pub(crate) fn no_unsettled_instance_use(db: &VerifiedDatabaseConnection<'_>, driver: &str)
    -> Result<(), ManagedCliError> {
    let own_probe = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_coordination_process_custody WHERE profile_id=?1 AND (state!='STOPPED' OR stop_proof_hash IS NULL OR stop_proof_hash='') LIMIT 1")?;
    own_probe.bind_text(1, &format!("managed-cli-{driver}"))?;
    if own_probe.step_row()? { return Err(ManagedCliError::Busy); }
    let pin=read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    // A first copy of the identical fixed image adds a MANAGED source; it
    // does not replace any registered program. Historical global UNKNOWN
    // remains UNKNOWN and is not treated as a StopFact. This narrow scope is
    // decided from current registrations and copy progress on every call.
    if first_same_pin_copy(db,driver,pin)? {
        let unsettled=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_coordination_process_custody c
              JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id
             WHERE i.driver_id=?1 AND c.domain_id='global' AND
               (c.state NOT IN ('UNKNOWN','STOPPED') OR
                (c.state='STOPPED' AND (c.stop_proof_hash IS NULL OR c.stop_proof_hash='')))
             LIMIT 1")?;
        unsettled.bind_text(1,driver)?;
        if unsettled.step_row()? { return Err(ManagedCliError::Busy); }
    } else {
        no_unsettled_global_cli_custody(db,driver)?;
    }
    // Preparing a first managed copy cannot change an old instance's selected
    // program. A missing copy row alone does not prove that old registrations
    // have this pin; a genuine version change retains the strict barrier.
    let old=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) AND (i.program_digest!=?2 OR i.version!=?3) LIMIT 1")?;
    old.bind_text(1,driver)?;
    old.bind_text(2,&format!("sha256:{}",pin.image_sha256))?;
    old.bind_text(3,pin.version)?;
    let mut version_change=old.step_row()?;
    drop(old);
    let copy=Statement::prepare(db.as_ptr(),
        "SELECT COALESCE(version,''),COALESCE(previous_version,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    copy.bind_text(1,driver)?;
    if copy.step_row()? {
        let version=copy.column_text(0)?;
        let previous=copy.column_text(1)?;
        if copy.step_row()? {return Err(ManagedCliError::Busy)}
        if (!version.is_empty() && version!=pin.version) ||
            (!previous.is_empty() && previous!=pin.version) {
            version_change=true;
        }
    }
    drop(copy);
    if version_change {no_unsettled_version_change(db,driver)?;}
    Ok(())
}

/// Only the first copy with an unchanged fixed image may defer historical
/// global UNKNOWN retirement. A progressed copy must already identify these
/// exact bytes; a prior source or previous copy makes this a replacement.
pub(crate) fn is_first_same_pin_copy(db:&VerifiedDatabaseConnection<'_>,driver:&str)
    ->Result<bool,ManagedCliError> {
    let pin=read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    first_same_pin_copy(db,driver,pin)
}

/// The legacy default install path may prepare only an unbound first copy of
/// the default pin. A versioned replacement uses its separate explicit path.
pub(crate) fn require_default_first_copy(db:&VerifiedDatabaseConnection<'_>,driver:&str)
    ->Result<(),ManagedCliError> {
    let pin=read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    let source=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instance_program_sources s JOIN main.gogoke_v37_instances i ON i.instance_id=s.instance_id WHERE i.driver_id=?1 LIMIT 1")?;
    source.bind_text(1,driver)?;
    if source.step_row()? {return Err(ManagedCliError::Busy)}
    let instance=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instances WHERE driver_id=?1 AND (version!=?2 OR program_digest!=?3) LIMIT 1")?;
    instance.bind_text(1,driver)?;
    instance.bind_text(2,pin.version)?;
    instance.bind_text(3,&format!("sha256:{}",pin.image_sha256))?;
    if instance.step_row()? {return Err(ManagedCliError::Busy)}
    let copy=Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),previous_version IS NULL AND previous_image_sha256 IS NULL AND previous_stage_name IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    copy.bind_text(1,driver)?;
    if !copy.step_row()? {return Ok(())}
    let state=copy.column_text(0)?;
    let version=copy.column_text(1)?;
    let archive=copy.column_text(2)?;
    let image=copy.column_text(3)?;
    let no_previous=copy.column_text(4)?=="1";
    if copy.step_row()? || !no_previous ||
        !matches!(state.as_str(),"NOT_INSTALLED"|"DOWNLOADING"|"INSTALLING"|
            "INSTALL_FAILED"|"BLOCKED"|"STAGED"|"PROBED") ||
        !((version.is_empty()&&archive.is_empty()&&image.is_empty()) ||
          (version==pin.version&&archive==pin.archive_sha256&&image==pin.image_sha256)) {
        return Err(ManagedCliError::Busy);
    }
    Ok(())
}

#[derive(Clone,Copy,PartialEq,Eq)]
enum EffectiveManagedSource {Current,Previous}

/// The copy's current columns denote the effective source before stage and a
/// prepared target after stage. Every retained instance, including disabled,
/// must agree with one side of the original SOURCE/copy tuple.
fn effective_managed_source(db:&VerifiedDatabaseConnection<'_>,root:Option<&RootLock>,driver:&str,
    state:&str,
    current:(&str,&str,&str,&str),previous:Option<(&str,&str,&str)>)
    ->Result<Option<EffectiveManagedSource>,ManagedCliError> {
    let empty_current=current.0.is_empty()&&current.1.is_empty()&&
        current.2.is_empty()&&current.3.is_empty();
    if !empty_current {
        let current_pin=read_official_cli_version(driver,current.0)
            .ok_or(ManagedCliError::IdentityChanged)?;
        if current.1!=current_pin.archive_sha256 || current.2!=current_pin.image_sha256 ||
            !safe_stage_name(current.3,current_pin) {return Err(ManagedCliError::IdentityChanged)}
    } else if previous.is_some() {return Err(ManagedCliError::IdentityChanged)}
    if let Some((version,image,stage))=previous {
        let pin=read_official_cli_version(driver,version)
            .ok_or(ManagedCliError::IdentityChanged)?;
        if version==current.0 || image!=pin.image_sha256 ||
            !safe_stage_name(stage,pin) {return Err(ManagedCliError::IdentityChanged)}
    }
    let rows=Statement::prepare(db.as_ptr(),
        "SELECT i.instance_id,i.home_ref,i.version,i.program_digest,i.home_identity,COALESCE(CAST(p.enabled AS TEXT),''),COALESCE(s.source,''),COALESCE(s.version,''),COALESCE(s.program_digest,''),COALESCE(s.stage_name,''),COALESCE(s.home_identity,''),COALESCE(s.registration_request_id,'') FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id LEFT JOIN main.gogoke_v37_instance_program_sources s ON s.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) ORDER BY i.instance_id")?;
    rows.bind_text(1,driver)?;
    let mut shape=None;
    let mut unbound=Vec::new();
    while rows.step_row()? {
        let id=rows.column_text(0)?;
        let home_ref=rows.column_text(1)?;
        let instance_version=rows.column_text(2)?;
        let instance_digest=rows.column_text(3)?;
        let home=rows.column_text(4)?;
        let enabled=rows.column_text(5)?;
        let source=rows.column_text(6)?;
        let source_version=rows.column_text(7)?;
        let source_digest=rows.column_text(8)?;
        let source_stage=rows.column_text(9)?;
        let source_home=rows.column_text(10)?;
        let registration=rows.column_text(11)?;
        if source.is_empty() && source_version.is_empty() && source_digest.is_empty() &&
            source_stage.is_empty() && source_home.is_empty() && registration.is_empty() {
            unbound.push((id,home_ref,instance_version,instance_digest,home,enabled));
            continue;
        }
        if source!="MANAGED" || source_home!=home || home.is_empty() ||
            registration.is_empty() || source_version!=instance_version ||
            source_digest!=instance_digest {return Err(ManagedCliError::Busy)}
        let current_match=instance_version==current.0 &&
            instance_digest==format!("sha256:{}",current.2) && source_stage==current.3;
        let previous_match=previous.is_some_and(|(version,image,stage)|
            instance_version==version && instance_digest==format!("sha256:{image}") &&
            source_stage==stage);
        let row_shape=match (current_match,previous_match) {
            (true,false)=>EffectiveManagedSource::Current,
            (false,true)=>EffectiveManagedSource::Previous,
            _=>return Err(ManagedCliError::Busy),
        };
        if shape.is_some_and(|prior|prior!=row_shape) {return Err(ManagedCliError::Busy)}
        shape=Some(row_shape);
    }
    drop(rows);
    if !unbound.is_empty() && shape.is_some() {
        let fixed=read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
        if state!="READY" || shape!=Some(EffectiveManagedSource::Current) || previous.is_some() ||
            current.0!=fixed.version {return Err(ManagedCliError::Busy)}
        for (id,home_ref,version,digest,home,enabled) in unbound {
            if enabled!="0" || !registry::valid_id(&id) ||
                home_ref!=format!("instance-home-{id}") || home.is_empty() ||
                version!=fixed.version || digest!=format!("sha256:{}",fixed.image_sha256) {
                return Err(ManagedCliError::Busy);
            }
            let creation=Statement::prepare(db.as_ptr(),
                "SELECT phase,native_receipt_id FROM main.gogoke_v37_instance_operations WHERE target_id=?1 AND native_receipt_id IS NOT NULL")?;
            creation.bind_text(1,&id)?;
            if !creation.step_row()? || creation.column_text(0)?!="APPLIED" ||
                creation.column_text(1)?!=home || creation.step_row()? {
                return Err(ManagedCliError::Busy);
            }
            if let Some(root)=root {
                let observed=registry::observed_home(root,&id)
                    .map_err(|error|ManagedCliError::Observation(format!(
                        "disabled legacy home: {error:?}")))?;
                if observed.is_none_or(|identity|identity.opaque()!=home) {
                    return Err(ManagedCliError::IdentityChanged);
                }
            }
        }
        // This is the original first-copy migration's disabled legacy shape.
        // It is readable but cannot qualify a version switch.
        return Ok(None);
    }
    Ok(shape)
}

/// Read-only shape for the native status bridge. None conveys no qualifying
/// SOURCE evidence, never permission to activate or repeat a switch.
pub(crate) fn read_prepared_managed_replacement(db:&VerifiedDatabaseConnection<'_>,
    root:&RootLock,driver:&str)->Result<Option<bool>,ManagedCliError> {
    if read_fixed_official_cli(driver).is_none() {return Err(ManagedCliError::Unsupported)}
    let row=Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),previous_version IS NULL AND previous_image_sha256 IS NULL AND previous_stage_name IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1,driver)?;
    if !row.step_row()? {return Ok(None)}
    let state=row.column_text(0)?;
    let current=(row.column_text(1)?,row.column_text(2)?,
        row.column_text(3)?,row.column_text(4)?);
    let old=(row.column_text(5)?,row.column_text(6)?,row.column_text(7)?);
    let no_previous=row.column_text(8)?=="1";
    if row.step_row()? || (!no_previous &&
        (old.0.is_empty()||old.1.is_empty()||old.2.is_empty())) ||
        (current.0.is_empty() && !matches!(state.as_str(),
            "NOT_INSTALLED"|"DOWNLOADING"|"INSTALLING"|"INSTALL_FAILED"|"BLOCKED")) {
        return Err(ManagedCliError::IdentityChanged);
    }
    drop(row);
    let previous=if no_previous {None} else {
        Some((old.0.as_str(),old.1.as_str(),old.2.as_str()))
    };
    Ok(effective_managed_source(db,Some(root),driver,&state,
        (&current.0,&current.1,&current.2,&current.3),previous)?
        .map(|shape|shape==EffectiveManagedSource::Previous))
}

fn first_same_pin_copy(db:&VerifiedDatabaseConnection<'_>,driver:&str,
    pin:VerifiedOfficialCli)->Result<bool,ManagedCliError> {
    let source=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instance_program_sources s
          JOIN main.gogoke_v37_instances i ON i.instance_id=s.instance_id
         WHERE i.driver_id=?1 LIMIT 1")?;
    source.bind_text(1,driver)?;
    if source.step_row()? {return Ok(false)}
    let instances=Statement::prepare(db.as_ptr(),
        "SELECT version,program_digest FROM main.gogoke_v37_instances WHERE driver_id=?1")?;
    instances.bind_text(1,driver)?;
    let mut count=0usize;
    while instances.step_row()? {
        count+=1;
        if instances.column_text(0)?!=pin.version ||
            instances.column_text(1)?!=format!("sha256:{}",pin.image_sha256) {
            return Ok(false)
        }
    }
    if count==0 {return Ok(false)}
    let copy=Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(version,''),COALESCE(archive_sha256,''),
                COALESCE(image_sha256,''),COALESCE(previous_version,''),
                COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,'')
           FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    copy.bind_text(1,driver)?;
    if !copy.step_row()? {return Ok(true)}
    let state=copy.column_text(0)?;
    let version=copy.column_text(1)?;
    let archive=copy.column_text(2)?;
    let image=copy.column_text(3)?;
    let previous=(copy.column_text(4)?,copy.column_text(5)?,copy.column_text(6)?);
    if copy.step_row()? || previous.0.len()+previous.1.len()+previous.2.len()!=0 {
        return Ok(false)
    }
    let initial=matches!(state.as_str(),"NOT_INSTALLED"|"DOWNLOADING"|"INSTALLING");
    let progressed=matches!(state.as_str(),"STAGED"|"PROBED"|"BLOCKED");
    let empty=version.is_empty() && archive.is_empty() && image.is_empty();
    let same_pin=version==pin.version && archive==pin.archive_sha256 &&
        image==pin.image_sha256;
    // Progress from an exact-pin BLOCKED row preserves its three pin columns.
    // Any partially retained or different pin is still a replacement.
    Ok((initial && (empty || same_pin)) || (progressed && same_pin))
}

fn no_unsettled_version_change(db: &VerifiedDatabaseConnection<'_>, driver: &str)
    -> Result<(), ManagedCliError> {
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND (c.state NOT IN ('RELEASED','STOPPED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_owner_binding b JOIN main.gogoke_v37_instances i ON i.instance_id=b.instance_id LEFT JOIN main.gogoke_v37_h_claim c ON c.binding_id=b.binding_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND b.state='ACTIVE' AND (c.binding_id IS NULL OR c.state NOT IN ('STOPPED','RELEASED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE i.driver_id=?1 AND (e.phase NOT IN ('STOPPED','FAILED') OR (e.process_operation_id IS NOT NULL AND (e.stop_fact_id IS NULL OR c.state IS NULL OR c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR e.stop_fact_id!=c.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_stdin_journal j JOIN main.gogoke_v37_h_claim c ON c.domain_id=j.domain_id AND c.session_id=j.session_id JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id=?1 AND j.phase!='RECEIPTED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_operation o JOIN main.gogoke_v37_h_claim c ON c.domain_id=o.domain_id AND c.session_id=o.session_id JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id WHERE i.driver_id=?1 AND o.status!='APPLIED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_dos_alias a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 AND (a.phase IN ('INTENT','PREPARE_STARTED','UNKNOWN') OR a.cleanup_status IN ('NOT_REQUESTED','UNKNOWN')) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_homes h JOIN main.gogoke_v37_instances i ON i.instance_id=h.instance_id WHERE i.driver_id=?1 AND h.state NOT IN ('CLEANED','CLOSED') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_instance_operations o JOIN main.gogoke_v37_instances i ON i.instance_id=o.target_id WHERE i.driver_id=?1 AND o.phase!='APPLIED' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_objects o JOIN main.gogoke_v37_instances i ON i.instance_id=o.instance_id WHERE i.driver_id=?1 AND o.phase!='ACTIVE' LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_aliases a JOIN main.gogoke_v37_instances i ON i.instance_id=a.instance_id WHERE i.driver_id=?1 AND a.state IN ('PREPARING','REMOVE_PENDING','UNKNOWN') LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_credential_profiles p JOIN main.gogoke_v37_instances i ON i.instance_id=p.instance_id WHERE i.driver_id=?1 AND p.state IN ('GRANT_PENDING','REVOKE_PENDING','UNKNOWN') LIMIT 1",
    ] {
        let row = Statement::prepare(db.as_ptr(), sql)?;
        row.bind_text(1, driver)?;
        if row.step_row()? { return Err(ManagedCliError::Busy); }
    }
    Ok(())
}

/// Replacement never inherits the first-copy historical UNKNOWN exception.
/// A release fence can qualify adoption of the *same* image, not switching it.
pub(crate) fn no_unsettled_managed_version_change(
    db: &VerifiedDatabaseConnection<'_>, driver: &str) -> Result<(), ManagedCliError> {
    no_unsettled_instance_use(db, driver)?;
    no_unsettled_version_change(db, driver)?;
    let custody = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_coordination_process_custody c JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id WHERE i.driver_id=?1 AND c.domain_id='global' AND (c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR c.stop_proof_hash='') LIMIT 1")?;
    custody.bind_text(1, driver)?;
    if custody.step_row()? { return Err(ManagedCliError::Busy); }
    Ok(())
}

/// A historical global UNKNOWN is retired only by its already captured exact
/// legacy ACL fence and applied baseline, plus a fresh kernel observation of
/// those same original PID/creation pairs. This never writes STOPPED or a new
/// StopFact. Every other global row needs its actual durable stop proof.
pub(crate) fn no_unsettled_global_cli_custody(
    db:&VerifiedDatabaseConnection<'_>,driver:&str)->Result<(),ManagedCliError> {
    if read_fixed_official_cli(driver).is_none(){return Err(ManagedCliError::Unsupported)}
    let rows=Statement::prepare(db.as_ptr(),
        "SELECT i.instance_id,i.home_identity,c.operation_id,c.ticket,c.custodian_nonce,
           c.pid,c.creation_time_100ns,c.image_path,c.binary_digest_sha256,c.profile_id,
           c.domain_id,c.generation,c.state,COALESCE(c.stop_proof_hash,''),c.stop_proof_hash IS NULL
           FROM main.gogoke_coordination_process_custody c
           JOIN main.gogoke_v37_instances i ON i.instance_id=c.profile_id
          WHERE i.driver_id=?1 AND c.domain_id='global' ORDER BY i.instance_id,c.operation_id")?;
    rows.bind_text(1,driver)?;
    let mut uncertain=std::collections::BTreeMap::<String,(String,Vec<legacy_fence::LegacyCustodyRow>)>::new();
    while rows.step_row()? {
        let id=rows.column_text(0)?;
        let home=rows.column_text(1)?;
        let row=legacy_fence::LegacyCustodyRow {
            operation_id:rows.column_text(2)?,ticket:rows.column_text(3)?,
            custodian_nonce:rows.column_text(4)?,pid:rows.column_text(5)?,
            creation_time_100ns:rows.column_text(6)?,image_path:rows.column_text(7)?,
            binary_digest_sha256:rows.column_text(8)?,profile_id:rows.column_text(9)?,
            domain_id:rows.column_text(10)?,generation:rows.column_text(11)?,
            state:rows.column_text(12)?,stop_proof_hash:if rows.column_text(14)?=="1" {
                None
            }else{Some(rows.column_text(13)?)},
        };
        match row.state.as_str(){
            "STOPPED" if row.stop_proof_hash.as_deref().is_some_and(|value|!value.is_empty())=>(),
            "UNKNOWN"=>{
                let entry=uncertain.entry(id).or_insert_with(||(home.clone(),Vec::new()));
                if entry.0!=home {return Err(ManagedCliError::Busy)}
                entry.1.push(row);
            },
            _=>return Err(ManagedCliError::Busy),
        }
    }
    drop(rows);
    for (id,(home,unknown)) in uncertain {
        if driver!="codex" {return Err(ManagedCliError::Busy)}
        let fence=legacy_fence::read_applied_legacy_retirement(db,&id)
            .map_err(|error|ManagedCliError::Legacy(format!("fence: {error:?}")))?
            .ok_or(ManagedCliError::Busy)?;
        if fence.database_identity!=db.identity().opaque() ||
            fence.root_identity!=db.root_identity().opaque() || fence.home_identity!=home ||
            unknown.iter().any(|row| !fence.custody.contains(row)) {
            return Err(ManagedCliError::Busy);
        }
        let pairs=fence.custody.iter().map(legacy_fence::LegacyCustodyRow::native_pair)
            .collect::<std::result::Result<Vec<_>,_>>()
            .map_err(|error|ManagedCliError::Legacy(format!("original process identity: {error:?}")))?;
        let gone=NativeLegacyHoldersGone::observe(&pairs)
            .map_err(|error|ManagedCliError::NativeGone(format!("original holders: {error:?}")))?;
        gone.validate(&pairs)
            .map_err(|error|ManagedCliError::NativeGone(format!("original holders changed: {error:?}")))?;
    }
    Ok(())
}

/// Stores a verified stage, never READY. A version change requires Owner
/// action and exact native no-holder/no-intent checks in this transaction.
pub(crate) fn record_managed_cli_stage(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    owner: &OwnerIssuer, driver: &str, stage_name: &str)
    -> Result<(), ManagedCliError> {
    let pin = read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    require_default_first_copy(db,driver)?;
    record_managed_cli_stage_inner(db,root,owner,driver,pin.version,stage_name,true)
}

pub(crate) fn record_managed_cli_stage_for_version(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, driver: &str, version: &str,
    stage_name: &str) -> Result<(), ManagedCliError> {
    record_managed_cli_stage_inner(db,root,owner,driver,version,stage_name,false)
}

fn record_managed_cli_stage_inner(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, owner: &OwnerIssuer, driver: &str, version: &str,
    stage_name: &str, default_first_copy:bool) -> Result<(), ManagedCliError> {
    let pin = read_official_cli_version(driver, version).ok_or(ManagedCliError::Unsupported)?;
    let image=inspect_staged_official_cli_version(root, driver, version, stage_name)?;
    let downloaded=if pin.raw_image {image} else {
        managed_cli_root(root)?.join(stage_name).join("source.download")
    };
    let progress_bytes=i64::try_from(fs::symlink_metadata(&downloaded)?.len())
        .map_err(|_|ManagedCliError::Invalid)?;
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        if default_first_copy {require_default_first_copy(db,driver)?;}
        no_unsettled_instance_use(db, driver)?;
        // Recheck bytes under the same held root before selecting the copy.
        inspect_staged_official_cli_version(root, driver, version, stage_name)?;
        let prior = Statement::prepare(db.as_ptr(),
            "SELECT state,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),previous_version IS NULL AND previous_image_sha256 IS NULL AND previous_stage_name IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        prior.bind_text(1, driver)?;
        let previous = if prior.step_row()? {
            let state = prior.column_text(0)?;
            let value = (prior.column_text(1)?,prior.column_text(2)?,
                prior.column_text(3)?,prior.column_text(4)?);
            let old=(prior.column_text(5)?,prior.column_text(6)?,prior.column_text(7)?);
            let no_previous=prior.column_text(8)?=="1";
            if prior.step_row()? || matches!(state.as_str(),
                "STAGED"|"PROBED"|"PROBE_UNKNOWN"|"UNINSTALLING") ||
                (!no_previous && (old.0.is_empty()||old.1.is_empty()||old.2.is_empty())) {
                return Err(ManagedCliError::Busy);
            }
            if matches!(state.as_str(),"READY"|"UPGRADING"|"BLOCKED"|"UPGRADE_FAILED") {
                let previous=if no_previous {None} else {
                    Some((old.0.as_str(),old.1.as_str(),old.2.as_str()))
                };
                let effective=effective_managed_source(db,None,driver,&state,
                    (&value.0,&value.1,&value.2,&value.3),previous)?
                    .ok_or(ManagedCliError::Busy)?;
                let selected=match effective {
                    EffectiveManagedSource::Current if value.0!=pin.version=>
                        (value.0,value.2,value.3),
                    EffectiveManagedSource::Previous if value.0==pin.version &&
                        value.3==stage_name=>old,
                    _=>return Err(ManagedCliError::Busy),
                };
                inspect_staged_official_cli_version(root,driver,&selected.0,&selected.2)?;
                no_unsettled_managed_version_change(db,driver)?;
                Some(selected)
            } else if no_previous {
                require_default_first_copy(db,driver)?;
                if pin.version!=read_fixed_official_cli(driver).unwrap().version {
                    return Err(ManagedCliError::Busy);
                }
                None
            } else {return Err(ManagedCliError::Busy)}
        } else { None };
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,previous_version,previous_image_sha256,previous_stage_name,progress_bytes,revision) VALUES(?1,'STAGED',?2,?3,?4,?5,?6,?7,?8,?9,1) ON CONFLICT(driver_id) DO UPDATE SET state='STAGED',version=excluded.version,archive_sha256=excluded.archive_sha256,image_sha256=excluded.image_sha256,stage_name=excluded.stage_name,previous_version=excluded.previous_version,previous_image_sha256=excluded.previous_image_sha256,previous_stage_name=excluded.previous_stage_name,progress_bytes=excluded.progress_bytes,raw_error=NULL,revision=revision+1")?;
        write.bind_text(1, driver)?;
        write.bind_text(2, pin.version)?;
        write.bind_text(3, pin.archive_sha256)?;
        write.bind_text(4, pin.image_sha256)?;
        write.bind_text(5, stage_name)?;
        if let Some((version, digest, name)) = previous {
            write.bind_text(6, &version)?; write.bind_text(7, &digest)?; write.bind_text(8, &name)?;
        }
        write.bind_i64(9,progress_bytes)?;
        write.step_done()?;
        Ok(())
    })
}

/// Only the native H launch/probe caller may invoke this after the exact
/// image process returned its actual identity and launch result. PROBED is
/// not READY; F promotes it with all existing instance bindings atomically.
pub(crate) fn confirm_managed_cli_launch(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    driver: &str, stage_name: &str, observed_digest: &str, observed_version: &str)
    -> Result<(), ManagedCliError> {
    let pin = read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    confirm_managed_cli_launch_for_version(db,root,driver,pin.version,stage_name,
        observed_digest,observed_version)
}

pub(crate) fn confirm_managed_cli_launch_for_version(db: &mut VerifiedDatabaseConnection<'_>,
    root: &RootLock, driver: &str, version: &str, stage_name: &str,
    observed_digest: &str, observed_version: &str) -> Result<(), ManagedCliError> {
    let pin = read_official_cli_version(driver,version).ok_or(ManagedCliError::Unsupported)?;
    if observed_digest != format!("sha256:{}", pin.image_sha256) || observed_version != pin.version {
        return Err(ManagedCliError::IdentityChanged);
    }
    transaction(db, |db| {
        no_unsettled_instance_use(db, driver)?;
        inspect_staged_official_cli_version(root, driver, version, stage_name)?;
        let previous = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1 AND previous_version IS NOT NULL LIMIT 1")?;
        previous.bind_text(1,driver)?;
        if previous.step_row()? { no_unsettled_managed_version_change(db,driver)?; }
        let write = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_cli_copies SET state='PROBED',raw_error=NULL,revision=revision+1 WHERE driver_id=?1 AND state='STAGED' AND stage_name=?2 AND image_sha256=?3 AND version=?4")?;
        write.bind_text(1, driver)?;
        write.bind_text(2, stage_name)?;
        write.bind_text(3, pin.image_sha256)?;
        write.bind_text(4, pin.version)?;
        write.step_done()?;
        let check = Statement::prepare(db.as_ptr(), "SELECT state FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        check.bind_text(1, driver)?;
        if !check.step_row()? || check.column_text(0)? != "PROBED" || check.step_row()? {
            return Err(ManagedCliError::Busy);
        }
        Ok(())
    })
}

/// Preserve the original Windows/CLI error rather than synthesizing a state.
pub(crate) fn record_managed_cli_failure(db: &mut VerifiedDatabaseConnection<'_>,
    driver: &str, state: &str, raw_error: &str) -> Result<(), ManagedCliError> {
    if read_fixed_official_cli(driver).is_none() ||
        !matches!(state, "INSTALL_FAILED" | "UPGRADE_FAILED" | "BLOCKED" | "PROBE_UNKNOWN") ||
        raw_error.is_empty() || raw_error.len() > 16_384 {
        return Err(ManagedCliError::Invalid);
    }
    transaction(db, |db| {
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,raw_error,revision) VALUES(?1,?2,?3,1) ON CONFLICT(driver_id) DO UPDATE SET state=excluded.state,raw_error=excluded.raw_error,revision=revision+1")?;
        write.bind_text(1, driver)?; write.bind_text(2, state)?; write.bind_text(3, raw_error)?;
        write.step_done()?;
        Ok(())
    })
}

/// Persist actual received/extracted byte progress from the signed Node
/// downloader. It cannot certify archive, image, launch, or readiness.
pub(crate) fn record_managed_cli_progress(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, driver: &str, state: &str, bytes: i64)
    -> Result<(), ManagedCliError> {
    let pin=read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    record_managed_cli_progress_inner(db,owner,driver,pin.version,state,bytes,true)
}

pub(crate) fn record_managed_cli_progress_for_version(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, driver: &str, version: &str, state: &str, bytes: i64)
    -> Result<(), ManagedCliError> {
    record_managed_cli_progress_inner(db,owner,driver,version,state,bytes,false)
}

fn record_managed_cli_progress_inner(db:&mut VerifiedDatabaseConnection<'_>,
    owner:&OwnerIssuer,driver:&str,version:&str,state:&str,bytes:i64,
    default_first_copy:bool)->Result<(),ManagedCliError> {
    let pin=read_official_cli_version(driver,version).ok_or(ManagedCliError::Unsupported)?;
    if bytes < 0 ||
        !matches!(state, "DOWNLOADING" | "INSTALLING" | "UPGRADING") {
        return Err(ManagedCliError::Invalid);
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        if default_first_copy {require_default_first_copy(db,driver)?;}
        no_unsettled_instance_use(db, driver)?;
        let prior = Statement::prepare(db.as_ptr(),
            "SELECT state,progress_bytes,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),previous_version IS NULL AND previous_image_sha256 IS NULL AND previous_stage_name IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        prior.bind_text(1, driver)?;
        if prior.step_row()? {
            let old = prior.column_text(0)?;
            let count = prior.column_text(1)?.parse::<i64>().map_err(|_| ManagedCliError::Invalid)?;
            let current_version=prior.column_text(2)?;
            let current_archive=prior.column_text(3)?;
            let current_image=prior.column_text(4)?;
            let current_stage=prior.column_text(5)?;
            let previous=(prior.column_text(6)?,prior.column_text(7)?,prior.column_text(8)?);
            let no_previous=prior.column_text(9)?=="1";
            if prior.step_row()? || (old == state && bytes < count) ||
                matches!(old.as_str(),"STAGED"|"PROBED"|"PROBE_UNKNOWN"|"UNINSTALLING") ||
                (old=="READY"&&current_version==pin.version) ||
                (!no_previous && (previous.0.is_empty()||previous.1.is_empty()||previous.2.is_empty())) {
                return Err(ManagedCliError::Busy);
            }
            if !default_first_copy && matches!(old.as_str(),
                "READY"|"UPGRADING"|"BLOCKED"|"UPGRADE_FAILED") {
                let prior_tuple=if no_previous {None} else {
                    Some((previous.0.as_str(),previous.1.as_str(),previous.2.as_str()))
                };
                match effective_managed_source(db,None,driver,&old,
                    (&current_version,&current_archive,&current_image,&current_stage),prior_tuple)?
                    .ok_or(ManagedCliError::Busy)? {
                    EffectiveManagedSource::Current if current_version!=pin.version &&
                        state=="UPGRADING"=>(),
                    EffectiveManagedSource::Previous if current_version==pin.version &&
                        state=="UPGRADING"=>(),
                    _=>return Err(ManagedCliError::Busy),
                }
            } else if !default_first_copy && !matches!(old.as_str(),
                "NOT_INSTALLED"|"DOWNLOADING"|"INSTALLING"|"INSTALL_FAILED") {
                return Err(ManagedCliError::Busy);
            }
            if matches!(old.as_str(),"READY"|"UPGRADING") ||
                (!no_previous && matches!(old.as_str(),"BLOCKED"|"UPGRADE_FAILED")) {
                no_unsettled_managed_version_change(db,driver)?;
            }
        }
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,progress_bytes,revision) VALUES(?1,?2,?3,1) ON CONFLICT(driver_id) DO UPDATE SET state=excluded.state,progress_bytes=excluded.progress_bytes,revision=revision+1")?;
        write.bind_text(1, driver)?; write.bind_text(2, state)?; write.bind_i64(3, bytes)?;
        write.step_done()?;
        Ok(())
    })
}

/// Read-only official release metadata is an unverified notice. It never
/// changes the selected program, version, pin, or installable offer.
pub(crate) fn record_official_cli_notice(db: &mut VerifiedDatabaseConnection<'_>,
    owner: &OwnerIssuer, driver: &str, notice: Option<&str>, checked_at: &str)
    -> Result<(), ManagedCliError> {
    if read_fixed_official_cli(driver).is_none() || checked_at.is_empty() ||
        checked_at.len() > 64 || checked_at.chars().any(char::is_control) ||
        notice.is_some_and(|value| value.is_empty() || value.len() > 128 ||
            !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))) {
        return Err(ManagedCliError::Invalid);
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,checked_at,official_notice,revision) VALUES(?1,'NOT_INSTALLED',?2,?3,1) ON CONFLICT(driver_id) DO UPDATE SET checked_at=excluded.checked_at,official_notice=excluded.official_notice,revision=revision+1")?;
        write.bind_text(1, driver)?;
        write.bind_text(2, checked_at)?;
        if let Some(value) = notice { write.bind_text(3, value)?; }
        write.step_done()?;
        Ok(())
    })
}

pub(crate) fn read_managed_cli(db: &VerifiedDatabaseConnection<'_>, root: &RootLock, driver: &str)
    -> Result<Option<ManagedCliCopy>, ManagedCliError> {
    if read_fixed_official_cli(driver).is_none() { return Err(ManagedCliError::Unsupported); }
    let row = Statement::prepare(db.as_ptr(), "SELECT state,COALESCE(version,''),COALESCE(stage_name,''),COALESCE(previous_version,''),progress_bytes,COALESCE(raw_error,''),COALESCE(checked_at,''),COALESCE(official_notice,''),revision,COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1, driver)?;
    if !row.step_row()? { return Ok(None); }
    let option = |index| -> Result<Option<String>, ManagedCliError> {
        let text = row.column_text(index)?;
        Ok(if text.is_empty() { None } else { Some(text) })
    };
    let mut result = ManagedCliCopy { driver: driver.to_owned(), state: row.column_text(0)?,
        version: option(1)?, stage_name: option(2)?, previous_version: option(3)?,
        previous_image_sha256: option(9)?, previous_stage_name: option(10)?,
        verified_other_versions: Vec::new(),
        progress_bytes: row.column_text(4)?.parse().map_err(|_| ManagedCliError::Invalid)?,
        raw_error: option(5)?, checked_at: option(6)?, official_notice: option(7)?,
        revision: row.column_text(8)?.parse().map_err(|_| ManagedCliError::Invalid)? };
    let archive=option(11)?;
    let image=option(12)?;
    if row.step_row()? { return Err(ManagedCliError::Invalid); }
    result.verified_other_versions = OFFICIAL.iter()
        .filter(|pin| pin.driver == driver && Some(pin.version) != result.version.as_deref())
        .map(|pin| pin.version.to_owned()).collect();
    if let Some(previous)=result.previous_version.as_deref() {
        let valid=read_official_cli_version(driver,previous).is_some_and(|pin|
            result.previous_image_sha256.as_deref()==Some(pin.image_sha256) &&
            result.previous_stage_name.as_deref().is_some_and(|name|
                inspect_staged_official_cli_version(root,driver,previous,name).is_ok()));
        if !valid {
            result.previous_version=None;
            result.previous_image_sha256=None;
            result.previous_stage_name=None;
        }
    }
    if result.state == "READY" {
        match (result.version.as_deref(), result.stage_name.as_deref()) {
            (Some(version), Some(name)) => {
                let pin=read_official_cli_version(driver,version);
                if pin.is_none_or(|pin| archive.as_deref()!=Some(pin.archive_sha256) ||
                    image.as_deref()!=Some(pin.image_sha256)) {
                    result.state="BLOCKED".to_owned();
                    result.raw_error=Some("READY copy pin differs from trusted catalog".to_owned());
                } else if let Err(error)=inspect_staged_official_cli_version(root,driver,version,name) {
                    result.state="BLOCKED".to_owned();
                    result.raw_error=Some(format!("{error:?}"));
                }
            },
            _ => {
                result.state = "BLOCKED".to_owned();
                result.raw_error = Some("READY copy has no native path".to_owned());
            },
        }
    }
    Ok(Some(result))
}

/// A configured managed copy is authoritative. None means no lifecycle row
/// exists, not permission to fall back to a user-global CLI. H decides whether
/// an original legacy instance has separate explicit migration authority.
pub(crate) fn locate_ready_managed_program(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    driver: &str, digest: &str, version: &str) -> Result<Option<PathBuf>, ManagedCliError> {
    let pin = read_official_cli_version(driver,version).ok_or(ManagedCliError::Unsupported)?;
    let row = Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(stage_name,''),COALESCE(image_sha256,''),COALESCE(version,''),COALESCE(archive_sha256,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1, driver)?;
    if !row.step_row()? { return Ok(None); }
    let state = row.column_text(0)?;
    let stage_name = row.column_text(1)?;
    let image_sha256 = row.column_text(2)?;
    let stored_version = row.column_text(3)?;
    let stored_archive = row.column_text(4)?;
    if row.step_row()? || state != "READY" || image_sha256 != pin.image_sha256 ||
        stored_version != pin.version || stored_archive != pin.archive_sha256 || version != pin.version ||
        digest != format!("sha256:{}", pin.image_sha256) {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(Some(inspect_staged_official_cli_version(root, driver, version, &stage_name)?))
}

fn verify_plain_tree(path: &Path) -> Result<(), ManagedCliError> {
    let item = fs::symlink_metadata(path)?;
    if item.file_attributes() & REPARSE_POINT != 0 || item.is_symlink() {
        return Err(ManagedCliError::IdentityChanged);
    }
    if item.is_dir() {
        for entry in fs::read_dir(path)? { verify_plain_tree(&entry?.path())?; }
    } else if !item.is_file() { return Err(ManagedCliError::IdentityChanged); }
    Ok(())
}

/// Delete only a private CLI copy with no active instance and no unresolved
/// holder/intent. The original instance home, credentials and history are
/// outside this tree and never touched.
pub(crate) fn uninstall_managed_cli(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
    owner: &OwnerIssuer, driver: &str, expected_revision: i64)
    -> Result<(), ManagedCliError> {
    if read_fixed_official_cli(driver).is_none() || expected_revision < 1 {
        return Err(ManagedCliError::Invalid);
    }
    let stages = transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        no_unsettled_instance_use(db, driver)?;
        no_unsettled_global_cli_custody(db, driver)?;
        no_unsettled_version_change(db, driver)?;
        let instance = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) LIMIT 1")?;
        instance.bind_text(1, driver)?;
        if instance.step_row()? { return Err(ManagedCliError::Busy); }
        let current = Statement::prepare(db.as_ptr(),
            "SELECT state,COALESCE(stage_name,''),revision,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),previous_version IS NULL AND previous_image_sha256 IS NULL AND previous_stage_name IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        current.bind_text(1, driver)?;
        if !current.step_row()? { return Err(ManagedCliError::Invalid); }
        let state = current.column_text(0)?;
        let name = current.column_text(1)?;
        let revision = current.column_text(2)?.parse::<i64>().map_err(|_| ManagedCliError::Invalid)?;
        let version = current.column_text(3)?;
        let archive = current.column_text(4)?;
        let image = current.column_text(5)?;
        let previous_version = current.column_text(6)?;
        let previous_image = current.column_text(7)?;
        let previous_stage = current.column_text(8)?;
        let no_previous=current.column_text(9)?=="1";
        if current.step_row()? || revision != expected_revision ||
            !(state == "READY" || state == "STAGED" || state == "BLOCKED" || state == "UNINSTALLING") {
            return Err(ManagedCliError::Busy);
        }
        let pin = read_official_cli_version(driver,&version)
            .ok_or(ManagedCliError::IdentityChanged)?;
        if archive!=pin.archive_sha256 || image!=pin.image_sha256 || !safe_stage_name(&name,pin) {
            return Err(ManagedCliError::IdentityChanged);
        }
        let previous=if no_previous {None} else {
            let previous = read_official_cli_version(driver,&previous_version)
                .ok_or(ManagedCliError::IdentityChanged)?;
            if previous_image!=previous.image_sha256 || previous_version==version ||
                !safe_stage_name(&previous_stage,previous) || previous_stage==name {
                return Err(ManagedCliError::IdentityChanged);
            }
            Some((previous_version,previous_image,previous_stage))
        };
        if state=="READY" {
            inspect_staged_official_cli_version(root,driver,&version,&name)?;
            if let Some((previous_version,_,previous_stage))=&previous {
                inspect_staged_official_cli_version(root,driver,previous_version,previous_stage)?;
            }
        }
        if state != "UNINSTALLING" {
            let write = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_cli_copies SET state='UNINSTALLING',revision=revision+1 WHERE driver_id=?1 AND revision=?2")?;
            write.bind_text(1, driver)?; write.bind_i64(2, revision)?; write.step_done()?;
        }
        Ok(((version,archive,image,name),previous))
    })?;
    let staging = managed_cli_root(root)?;
    let mut names=vec![stages.0.3.as_str()];
    if let Some(previous)=&stages.1 {names.push(previous.2.as_str());}
    for name in &names {
        let target=staging.join(name);
        // Each present recorded target must resolve to one direct plain child.
        // An absent target also lets an interrupted uninstall continue.
        let target_exists=match fs::symlink_metadata(&target) {
            Ok(_)=>true,
            Err(error) if error.kind()==io::ErrorKind::NotFound=>false,
            Err(error)=>return Err(error.into()),
        };
        if target_exists {
            plain_dir(&target)?;
            let resolved_parent=fs::canonicalize(&staging)?;
            if fs::canonicalize(&target)?.parent()!=Some(resolved_parent.as_path()) {
                return Err(ManagedCliError::IdentityChanged);
            }
            verify_plain_tree(&target)?;
            fs::remove_dir_all(&target)?;
        }
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        no_unsettled_instance_use(db, driver)?;
        no_unsettled_global_cli_custody(db, driver)?;
        no_unsettled_version_change(db, driver)?;
        let instance = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) LIMIT 1")?;
        instance.bind_text(1, driver)?;
        if instance.step_row()? { return Err(ManagedCliError::Busy); }
        let recorded=Statement::prepare(db.as_ptr(),
            "SELECT state,COALESCE(version,''),COALESCE(archive_sha256,''),COALESCE(image_sha256,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),previous_version IS NULL AND previous_image_sha256 IS NULL AND previous_stage_name IS NULL FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        recorded.bind_text(1,driver)?;
        if !recorded.step_row()? || recorded.column_text(0)?!="UNINSTALLING" ||
            (recorded.column_text(1)?,recorded.column_text(2)?,
             recorded.column_text(3)?,recorded.column_text(4)?)!=stages.0.clone() {
            return Err(ManagedCliError::Busy);
        }
        let prior=(recorded.column_text(5)?,recorded.column_text(6)?,recorded.column_text(7)?);
        let prior_null=recorded.column_text(8)?=="1";
        if recorded.step_row()? || prior_null!=stages.1.is_none() ||
            stages.1.as_ref().is_some_and(|expected|expected!=&prior) {
            return Err(ManagedCliError::Busy);
        }
        drop(recorded);
        for name in &names {
            match fs::symlink_metadata(staging.join(name)) {
                Err(error) if error.kind()==io::ErrorKind::NotFound=>(),
                Ok(_)=>return Err(ManagedCliError::Busy),
                Err(error)=>return Err(error.into()),
            }
        }
        let write = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_cli_copies SET state='NOT_INSTALLED',version=NULL,archive_sha256=NULL,image_sha256=NULL,stage_name=NULL,previous_version=NULL,previous_image_sha256=NULL,previous_stage_name=NULL,progress_bytes=0,raw_error=NULL,revision=revision+1 WHERE driver_id=?1 AND state='UNINSTALLING'")?;
        write.bind_text(1, driver)?; write.step_done()?;
        Ok(())
    })
}

#[cfg(all(test, windows))]
mod first_copy_tests {
    use super::*;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn blocked_same_pin_progress_retains_first_copy_scope_but_partial_or_prior_pin_does_not() {
        // This affected cloud filter also exercises the registry writer and
        // original-registration replay through two synthetic durable hops.
        super::super::registry::tests::manual_repin_does_not_legitimize_changed_creation_pin_or_damaged_witness();
        for driver in ["codex","claude","opencode","grok"] {
            let fixed=read_fixed_official_cli(driver).unwrap();
            assert_eq!(read_official_cli_version(driver,fixed.version)
                .unwrap().image_sha256,fixed.image_sha256);
            assert!(read_official_cli_version(driver,"999.999.999").is_none());
            assert_eq!(OFFICIAL.iter().filter(|pin|
                pin.driver==driver && pin.version==fixed.version).count(),1);
            assert!(safe_stage_name(&format!("{driver}-{}-private",fixed.version),fixed));
            assert!(!safe_stage_name(&format!("{driver}-999.999.999-private"),fixed));
        }
        let current=read_fixed_official_cli("opencode").unwrap();
        let prior=read_official_cli_version("opencode","1.18.31").unwrap();
        assert_eq!(current.version,"1.18.32");
        assert_eq!(current.image_sha256,
            "cf664aa1da32b788f9b2699b84a9bb9be30b7e025693b90f9b85829d5fe4e252");
        assert_eq!(prior.archive_sha256,
            "8a300ee2b3210ccc6894ff27d21bdb0a60ee44107389c49b743b4caed10aa5cf");
        assert_eq!(prior.image_sha256,
            "0242a0dc705af67c90882b456a36b619883c1c786aad8fe071a1bc64e5d1d440");
        assert_eq!(prior.image_relative,"package/bin/opencode.exe");
        assert!(!prior.raw_image);
        assert!(read_official_cli_version("opencode","1.18.30").is_none());
        assert_eq!(OFFICIAL.iter().filter(|pin|pin.driver=="opencode").count(),3);
        let raw=b"{\"schema\":\"gogoke.37.managed-cli.v1\",\"command\":\"rollback\",\"requestId\":\"switchA\",\"driverId\":\"opencode\",\"targetVersion\":\"1.18.31\",\"expectedRevision\":\"7\"}";
        let request=super::super::registry::ManagedSwitchRequest{
            request_bytes:raw,request_id:"switchA",driver:"opencode",
            target_version:"1.18.31",expected_copy_revision:7};
        assert_eq!(super::super::registry::managed_request_command(&request).unwrap(),"rollback");
        assert!(super::super::registry::managed_request_command(
            &super::super::registry::ManagedSwitchRequest{target_version:"1.18.32",..request})
            .is_err());
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-first-copy-pin-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut db=create_new(&root,&database).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        let pin=read_fixed_official_cli("codex").unwrap();
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('first','codex','home-first','identity-first',?1,?2,'UNKNOWN','UNKNOWN',1)").unwrap();
        insert.bind_text(1,&format!("sha256:{}",pin.image_sha256)).unwrap();
        insert.bind_text(2,pin.version).unwrap();insert.step_done().unwrap();drop(insert);
        let insert=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,revision) VALUES('codex','BLOCKED',?1,?2,?3,1)").unwrap();
        insert.bind_text(1,pin.version).unwrap();insert.bind_text(2,pin.archive_sha256).unwrap();
        insert.bind_text(3,pin.image_sha256).unwrap();insert.step_done().unwrap();drop(insert);
        assert!(first_same_pin_copy(&db,"codex",pin).unwrap());
        // record_managed_cli_progress updates only these columns, preserving
        // the exact pin left by BLOCKED across a durable progress transition.
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET state='DOWNLOADING',progress_bytes=12,raw_error=NULL,revision=revision+1 WHERE driver_id='codex'").unwrap();
        assert!(first_same_pin_copy(&db,"codex",pin).unwrap());
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET archive_sha256='other' WHERE driver_id='codex'").unwrap();
        assert!(!first_same_pin_copy(&db,"codex",pin).unwrap());
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET archive_sha256=NULL WHERE driver_id='codex'").unwrap();
        assert!(!first_same_pin_copy(&db,"codex",pin).unwrap());
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET version=NULL,image_sha256=NULL WHERE driver_id='codex'").unwrap();
        assert!(first_same_pin_copy(&db,"codex",pin).unwrap());
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET previous_version='0.149.0' WHERE driver_id='codex'").unwrap();
        assert!(!first_same_pin_copy(&db,"codex",pin).unwrap());
        db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET previous_version=NULL WHERE driver_id='codex'").unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES('first','MANAGED','old-stage','sha256:old','0.149.0','identity-first','old-request',1)").unwrap();
        assert!(!first_same_pin_copy(&db,"codex",pin).unwrap());
        db.close_checked().unwrap();drop(root);
        fs::remove_dir_all(&path).unwrap();
    }
}

#[cfg(all(test,windows))]
mod lifecycle_repair_tests {
    use super::*;
    use crate::store::same_open::{create_new,route_b_test_guard};
    use std::time::{SystemTime,UNIX_EPOCH};

    fn fixture(run:impl FnOnce(&RootLock,&mut VerifiedDatabaseConnection<'_>,&OwnerIssuer)) {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path=std::env::temp_dir().join(format!("gogoke-f2-repair-{}-{nonce}",std::process::id()));
        fs::create_dir(&path).unwrap();
        let root=RootLock::acquire(&path).unwrap();
        let database=path.join("state.sqlite");
        let mut db=create_new(&root,&database).unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        let owner=crate::store::authority::initialize_profile(&mut db,&root).unwrap();
        crate::store::authority::initialize_process_custody_schema(&mut db).unwrap();
        crate::store::instance::initialize_credential_schema(&mut db).unwrap();
        crate::store::seat::initialize_schema(&mut db).unwrap();
        crate::store::session_transport::initialize_admission_schema(&mut db).unwrap();
        crate::store::session_transport::session_binding::initialize_schema(&mut db).unwrap();
        run(&root,&mut db,&owner);
        db.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(path).unwrap();
    }

    fn copy(db:&VerifiedDatabaseConnection<'_>)->Vec<String> {
        let row=Statement::prepare(db.as_ptr(),
            "SELECT state,COALESCE(version,''),COALESCE(stage_name,''),COALESCE(previous_version,''),COALESCE(previous_image_sha256,''),COALESCE(previous_stage_name,''),revision FROM main.gogoke_v37_instance_cli_copies WHERE driver_id='opencode'").unwrap();
        assert!(row.step_row().unwrap());
        (0..7).map(|index|row.column_text(index).unwrap()).collect()
    }

    fn bound_disabled_instance(db:&VerifiedDatabaseConnection<'_>,id:&str,
        version:&str,image:&str,stage:&str) {
        let instance=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES(?1,'opencode',?2,?3,?4,?5,'UNKNOWN','UNKNOWN',2)").unwrap();
        instance.bind_text(1,id).unwrap();
        instance.bind_text(2,&format!("home-{id}")).unwrap();
        instance.bind_text(3,&format!("identity-{id}")).unwrap();
        instance.bind_text(4,&format!("sha256:{image}")).unwrap();
        instance.bind_text(5,version).unwrap();instance.step_done().unwrap();drop(instance);
        let profile=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_profiles(instance_id,display_name,enabled,revision) VALUES(?1,'disabled',0,1)").unwrap();
        profile.bind_text(1,id).unwrap();profile.step_done().unwrap();drop(profile);
        let source=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES(?1,'MANAGED',?2,?3,?4,?5,'creation',1)").unwrap();
        source.bind_text(1,id).unwrap();
        source.bind_text(2,stage).unwrap();
        source.bind_text(3,&format!("sha256:{image}")).unwrap();
        source.bind_text(4,version).unwrap();
        source.bind_text(5,&format!("identity-{id}")).unwrap();source.step_done().unwrap();
    }

    #[test]
    fn settled_failed_target_retry_keeps_original_previous_tuple() {
        fixture(|root,db,owner| {
            let old=read_official_cli_version("opencode","1.18.31").unwrap();
            let target=read_official_cli_version("opencode","1.18.32").unwrap();
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,previous_version,previous_image_sha256,previous_stage_name,raw_error,revision) VALUES('opencode','BLOCKED',?1,?2,?3,'opencode-1.18.32-target',?4,?5,'opencode-1.18.31-source','original probe exit',4)").unwrap();
            for (index,value) in [target.version,target.archive_sha256,target.image_sha256,
                old.version,old.image_sha256].iter().enumerate(){
                insert.bind_text((index+1) as i32,value).unwrap();
            }
            insert.step_done().unwrap();drop(insert);
            bound_disabled_instance(db,"instanceA",old.version,old.image_sha256,"opencode-1.18.31-source");
            let before=copy(db);
            assert_eq!(read_prepared_managed_replacement(db,root,"opencode").unwrap(),Some(true));
            assert!(record_managed_cli_progress_for_version(db,owner,"opencode","1.18.31",
                "UPGRADING",0).is_err());
            assert_eq!(copy(db),before);
            record_managed_cli_progress_for_version(db,owner,"opencode","1.18.32",
                "UPGRADING",0).unwrap();
            let after=copy(db);
            assert_eq!(&after[1..6],&before[1..6]);
            assert_eq!(after[0],"UPGRADING");
            assert_eq!(after[6],"5");
            assert_eq!(read_prepared_managed_replacement(db,root,"opencode").unwrap(),Some(true));
            let error=Statement::prepare(db.as_ptr(),
                "SELECT raw_error FROM main.gogoke_v37_instance_cli_copies WHERE driver_id='opencode'").unwrap();
            assert!(error.step_row().unwrap());
            assert_eq!(error.column_text(0).unwrap(),"original probe exit");
            db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET state='PROBE_UNKNOWN' WHERE driver_id='opencode'").unwrap();
            assert!(record_managed_cli_progress_for_version(db,owner,"opencode","1.18.32",
                "UPGRADING",0).is_err());
        });
    }

    #[test]
    fn prestage_failure_retries_from_effective_current_source() {
        fixture(|root,db,owner| {
            let current=read_official_cli_version("opencode","1.18.31").unwrap();
            let historical=read_official_cli_version("opencode","1.18.32").unwrap();
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,previous_version,previous_image_sha256,previous_stage_name,raw_error,revision) VALUES('opencode','UPGRADE_FAILED',?1,?2,?3,'opencode-1.18.31-source',?4,?5,'opencode-1.18.32-historical','original download failure',6)").unwrap();
            for (index,value) in [current.version,current.archive_sha256,current.image_sha256,
                historical.version,historical.image_sha256].iter().enumerate(){
                insert.bind_text((index+1) as i32,value).unwrap();
            }
            insert.step_done().unwrap();drop(insert);
            bound_disabled_instance(db,"instanceA",current.version,current.image_sha256,"opencode-1.18.31-source");
            let before=copy(db);
            assert_eq!(read_prepared_managed_replacement(db,root,"opencode").unwrap(),Some(false));
            bound_disabled_instance(db,"instanceB",historical.version,historical.image_sha256,
                "opencode-1.18.32-historical");
            assert!(read_prepared_managed_replacement(db,root,"opencode").is_err());
            assert!(record_managed_cli_progress_for_version(db,owner,"opencode",historical.version,
                "UPGRADING",0).is_err());
            assert_eq!(copy(db),before);
            db.execute("DELETE FROM main.gogoke_v37_instance_program_sources WHERE instance_id='instanceB'").unwrap();
            db.execute("DELETE FROM main.gogoke_v37_instance_profiles WHERE instance_id='instanceB'").unwrap();
            db.execute("DELETE FROM main.gogoke_v37_instances WHERE instance_id='instanceB'").unwrap();
            assert_eq!(read_prepared_managed_replacement(db,root,"opencode").unwrap(),Some(false));
            record_managed_cli_progress_for_version(db,owner,"opencode",historical.version,
                "UPGRADING",0).unwrap();
            let after=copy(db);
            assert_eq!(&after[1..6],&before[1..6]);
            assert_eq!(after[0],"UPGRADING");
            assert_eq!(after[6],"7");
            let error=Statement::prepare(db.as_ptr(),
                "SELECT raw_error FROM main.gogoke_v37_instance_cli_copies WHERE driver_id='opencode'").unwrap();
            assert!(error.step_row().unwrap());
            assert_eq!(error.column_text(0).unwrap(),"original download failure");
        });
    }

    #[test]
    fn status_source_shape_is_null_without_source_and_rejects_partial_previous() {
        fixture(|root,db,_| {
            assert_eq!(read_prepared_managed_replacement(db,root,"opencode").unwrap(),None);
            let pin=read_fixed_official_cli("opencode").unwrap();
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,revision) VALUES('opencode','STAGED',?1,?2,?3,'opencode-1.18.32-first',1)").unwrap();
            insert.bind_text(1,pin.version).unwrap();
            insert.bind_text(2,pin.archive_sha256).unwrap();
            insert.bind_text(3,pin.image_sha256).unwrap();
            insert.step_done().unwrap();drop(insert);
            assert_eq!(read_prepared_managed_replacement(db,root,"opencode").unwrap(),None);
            db.execute("UPDATE main.gogoke_v37_instance_cli_copies SET previous_version='1.18.31' WHERE driver_id='opencode'").unwrap();
            assert!(read_prepared_managed_replacement(db,root,"opencode").is_err());
        });
    }

    #[test]
    fn default_begin_rejects_nondefault_ready_without_changing_copy() {
        fixture(|root,db,owner| {
            let old=read_official_cli_version("opencode","1.18.31").unwrap();
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,revision) VALUES('opencode','READY',?1,?2,?3,'opencode-1.18.31-source',8)").unwrap();
            insert.bind_text(1,old.version).unwrap();
            insert.bind_text(2,old.archive_sha256).unwrap();
            insert.bind_text(3,old.image_sha256).unwrap();
            insert.step_done().unwrap();drop(insert);
            let instance=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','opencode','homeA','identityA',?1,?2,'UNKNOWN','UNKNOWN',2)").unwrap();
            instance.bind_text(1,&format!("sha256:{}",old.image_sha256)).unwrap();
            instance.bind_text(2,old.version).unwrap();instance.step_done().unwrap();drop(instance);
            let source=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES('instanceA','MANAGED','opencode-1.18.31-source',?1,?2,'identityA','creation',1)").unwrap();
            source.bind_text(1,&format!("sha256:{}",old.image_sha256)).unwrap();
            source.bind_text(2,old.version).unwrap();source.step_done().unwrap();drop(source);
            let before=copy(db);
            assert!(require_default_first_copy(db,"opencode").is_err());
            assert!(record_managed_cli_progress(db,owner,"opencode","DOWNLOADING",0).is_err());
            let stage=format!("opencode-{}-unobserved",read_fixed_official_cli("opencode").unwrap().version);
            assert!(matches!(record_managed_cli_stage(db,root,owner,"opencode",&stage),
                Err(ManagedCliError::Busy)));
            assert_eq!(copy(db),before);
            let bound=Statement::prepare(db.as_ptr(),
                "SELECT i.version,s.version,s.stage_name FROM main.gogoke_v37_instances i JOIN main.gogoke_v37_instance_program_sources s ON s.instance_id=i.instance_id WHERE i.instance_id='instanceA'").unwrap();
            assert!(bound.step_row().unwrap());
            assert_eq!((bound.column_text(0).unwrap(),bound.column_text(1).unwrap(),
                bound.column_text(2).unwrap()),
                (old.version.to_owned(),old.version.to_owned(),"opencode-1.18.31-source".to_owned()));
        });
    }

    #[test]
    fn uninstall_removes_only_two_recorded_stages_and_resumes_after_one_is_absent() {
        fixture(|root,db,owner| {
            let current=read_official_cli_version("opencode","1.18.31").unwrap();
            let previous=read_official_cli_version("opencode","1.18.32").unwrap();
            let staging=managed_cli_root(root).unwrap();
            let current_name="opencode-1.18.31-current";
            let previous_name="opencode-1.18.32-previous";
            let sentinel=staging.join("unrecorded-sentinel");
            for name in [current_name,previous_name,"unrecorded-sentinel"] {
                let stage=staging.join(name);
                fs::create_dir(&stage).unwrap();
                fs::write(stage.join("file"),b"owned cleanup fixture").unwrap();
            }
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,previous_version,previous_image_sha256,previous_stage_name,revision) VALUES('opencode','UNINSTALLING',?1,?2,?3,?4,?5,?6,?7,9)").unwrap();
            for (index,value) in [current.version,current.archive_sha256,current.image_sha256,
                current_name,previous.version,previous.image_sha256,previous_name].iter().enumerate(){
                insert.bind_text((index+1) as i32,value).unwrap();
            }
            insert.step_done().unwrap();drop(insert);
            fs::remove_dir_all(staging.join(current_name)).unwrap();
            uninstall_managed_cli(db,root,owner,"opencode",9).unwrap();
            assert!(!staging.join(current_name).exists());
            assert!(!staging.join(previous_name).exists());
            assert!(sentinel.join("file").exists());
            assert_eq!(copy(db)[0],"NOT_INSTALLED");
        });
    }

    #[test]
    fn uninstall_clears_both_present_registered_stages_but_not_unrecorded_cache() {
        fixture(|root,db,owner| {
            let current=read_official_cli_version("opencode","1.18.31").unwrap();
            let previous=read_official_cli_version("opencode","1.18.32").unwrap();
            let staging=managed_cli_root(root).unwrap();
            let names=["opencode-1.18.31-current","opencode-1.18.32-previous"];
            for name in [names[0],names[1],"unrecorded-sentinel"] {
                let stage=staging.join(name);
                fs::create_dir(&stage).unwrap();
                fs::write(stage.join("file"),b"owned cleanup fixture").unwrap();
            }
            let insert=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,version,archive_sha256,image_sha256,stage_name,previous_version,previous_image_sha256,previous_stage_name,revision) VALUES('opencode','BLOCKED',?1,?2,?3,?4,?5,?6,?7,3)").unwrap();
            for (index,value) in [current.version,current.archive_sha256,current.image_sha256,
                names[0],previous.version,previous.image_sha256,names[1]].iter().enumerate(){
                insert.bind_text((index+1) as i32,value).unwrap();
            }
            insert.step_done().unwrap();drop(insert);
            uninstall_managed_cli(db,root,owner,"opencode",3).unwrap();
            assert!(names.iter().all(|name|!staging.join(name).exists()));
            assert!(staging.join("unrecorded-sentinel").join("file").exists());
            let after=copy(db);
            assert_eq!(after[0],"NOT_INSTALLED");
            assert!(after[1..6].iter().all(String::is_empty));
        });
    }
}
