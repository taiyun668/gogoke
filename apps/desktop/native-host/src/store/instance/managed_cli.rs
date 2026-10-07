//! Product-owned fixed official CLI copies. A staged copy is never READY.
//! The native host checks the original archive and executable again before
//! recording it; H must launch that exact image and report the result before
//! the PROBED copy can be bound to every quiescent legacy instance and marked READY.

use super::{legacy_fence,registry::ProgramObservation};
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

const OFFICIAL: [VerifiedOfficialCli; 4] = [
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
    VerifiedOfficialCli { driver: "grok", version: "1.0.41",
        archive_sha256: "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        image_sha256: "ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25",
        image_relative: "grok.exe", raw_image: true },
];

pub(crate) fn read_fixed_official_cli(driver: &str) -> Option<VerifiedOfficialCli> {
    OFFICIAL.iter().copied().find(|pin| pin.driver == driver)
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
    #[cfg(test)]
    let measured_read = std::time::Instant::now();
    file.read_to_end(&mut bytes)?;
    #[cfg(test)]
    eprintln!("native_timing producer=managed_file_read bytes={} elapsed_us={}",
        bytes.len(), measured_read.elapsed().as_micros());
    let after = file.metadata()?;
    let path_after = fs::symlink_metadata(path)?;
    if after.len() != opened.len() || after.modified()? != opened.modified()? ||
        path_after.len() != before.len() || path_after.modified()? != before.modified()? ||
        path_after.file_attributes() & REPARSE_POINT != 0 {
        return Err(ManagedCliError::IdentityChanged);
    }
    if crate::store::digest::sha256_hex(&bytes) != expected {
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
    inspect_stage_at(&managed_cli_root(root)?, driver, stage_name)
}

fn inspect_stage_at(staging: &Path, driver: &str, stage_name: &str)
    -> Result<PathBuf, ManagedCliError> {
    let pin = read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
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
    let pin=read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    let row=Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(stage_name,''),COALESCE(image_sha256,''),COALESCE(version,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1,driver)?;
    if !row.step_row()? { return Ok(None); }
    let state=row.column_text(0)?;
    let name=row.column_text(1)?;
    let stored_digest=row.column_text(2)?;
    let stored_version=row.column_text(3)?;
    if row.step_row()? || state!="READY" || stored_digest!=pin.image_sha256 ||
        stored_version!=pin.version || version!=pin.version ||
        digest!=format!("sha256:{}",pin.image_sha256) {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(Some(inspect_stage_at(&verified_staging_from_db(db)?,driver,&name)?))
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
    no_unsettled_global_cli_custody(db,driver)?;
    for sql in [
        "SELECT 1 FROM main.gogoke_v37_h_claim c JOIN main.gogoke_v37_instances i ON i.instance_id=c.instance_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND (c.state NOT IN ('RELEASED','STOPPED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_owner_binding b JOIN main.gogoke_v37_instances i ON i.instance_id=b.instance_id LEFT JOIN main.gogoke_v37_h_claim c ON c.binding_id=b.binding_id LEFT JOIN main.gogoke_coordination_process_custody p ON p.operation_id=c.process_operation_id AND p.domain_id=c.domain_id AND p.generation=c.generation WHERE i.driver_id=?1 AND b.state='ACTIVE' AND (c.binding_id IS NULL OR c.state NOT IN ('STOPPED','RELEASED') OR (c.state='STOPPED' AND (c.stop_fact_id IS NULL OR p.state IS NULL OR p.state!='STOPPED' OR p.stop_proof_hash IS NULL OR c.stop_fact_id!=p.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_process_episode e JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id LEFT JOIN main.gogoke_coordination_process_custody c ON c.operation_id=e.process_operation_id WHERE i.driver_id=?1 AND (e.phase NOT IN ('STOPPED','FAILED') OR (e.process_operation_id IS NOT NULL AND (e.stop_fact_id IS NULL OR c.state IS NULL OR c.state!='STOPPED' OR c.stop_proof_hash IS NULL OR e.stop_fact_id!=c.stop_proof_hash))) LIMIT 1",
        "SELECT 1 FROM main.gogoke_v37_h_generation_change g JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=g.old_process_operation_id JOIN main.gogoke_v37_instances i ON i.instance_id=e.instance_id WHERE i.driver_id=?1 AND g.stage NOT IN ('APPLIED','CANCELLED','UNSUPPORTED') LIMIT 1",
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
    let image=inspect_staged_official_cli(root, driver, stage_name)?;
    let downloaded=if pin.raw_image {image} else {
        managed_cli_root(root)?.join(stage_name).join("source.download")
    };
    let progress_bytes=i64::try_from(fs::symlink_metadata(&downloaded)?.len())
        .map_err(|_|ManagedCliError::Invalid)?;
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        no_unsettled_instance_use(db, driver)?;
        // Recheck bytes under the same held root before selecting the copy.
        inspect_staged_official_cli(root, driver, stage_name)?;
        let prior = Statement::prepare(db.as_ptr(),
            "SELECT state,COALESCE(version,''),COALESCE(image_sha256,''),COALESCE(stage_name,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        prior.bind_text(1, driver)?;
        let previous = if prior.step_row()? {
            let state = prior.column_text(0)?;
            let value = (prior.column_text(1)?, prior.column_text(2)?, prior.column_text(3)?);
            if prior.step_row()? || matches!(state.as_str(), "STAGED" | "PROBED" | "PROBE_UNKNOWN" | "UNINSTALLING") ||
                (matches!(state.as_str(), "READY" | "UPGRADING") && value.0 == pin.version) {
                return Err(ManagedCliError::Busy);
            }
            if matches!(state.as_str(), "READY" | "UPGRADING") { Some(value) } else { None }
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
    if observed_digest != format!("sha256:{}", pin.image_sha256) || observed_version != pin.version {
        return Err(ManagedCliError::IdentityChanged);
    }
    transaction(db, |db| {
        no_unsettled_instance_use(db, driver)?;
        inspect_staged_official_cli(root, driver, stage_name)?;
        let write = Statement::prepare(db.as_ptr(), "UPDATE main.gogoke_v37_instance_cli_copies SET state='PROBED',raw_error=NULL,revision=revision+1 WHERE driver_id=?1 AND state='STAGED' AND stage_name=?2 AND image_sha256=?3")?;
        write.bind_text(1, driver)?;
        write.bind_text(2, stage_name)?;
        write.bind_text(3, pin.image_sha256)?;
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
    if read_fixed_official_cli(driver).is_none() || bytes < 0 ||
        !matches!(state, "DOWNLOADING" | "INSTALLING" | "UPGRADING") {
        return Err(ManagedCliError::Invalid);
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        no_unsettled_instance_use(db, driver)?;
        let prior = Statement::prepare(db.as_ptr(),
            "SELECT state,progress_bytes,COALESCE(version,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        prior.bind_text(1, driver)?;
        if prior.step_row()? {
            let old = prior.column_text(0)?;
            let count = prior.column_text(1)?.parse::<i64>().map_err(|_| ManagedCliError::Invalid)?;
            let version=prior.column_text(2)?;
            if prior.step_row()? || (old == state && bytes < count) ||
                matches!(old.as_str(),"STAGED"|"PROBED"|"PROBE_UNKNOWN"|"UNINSTALLING") ||
                (old=="READY"&&version==read_fixed_official_cli(driver)
                    .ok_or(ManagedCliError::Unsupported)?.version) {
                return Err(ManagedCliError::Busy);
            }
        }
        let write = Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_cli_copies(driver_id,state,progress_bytes,revision) VALUES(?1,?2,?3,1) ON CONFLICT(driver_id) DO UPDATE SET state=excluded.state,progress_bytes=excluded.progress_bytes,raw_error=NULL,revision=revision+1")?;
        write.bind_text(1, driver)?; write.bind_text(2, state)?; write.bind_i64(3, bytes)?;
        write.step_done()?;
        Ok(())
    })
}

/// Read-only official release metadata is an unverified notice. It never
/// changes the selected program, version, pin, or installable offer.
pub(crate) fn record_official_cli_notice(db: &mut VerifiedDatabaseConnection<'_>,
    driver: &str, notice: Option<&str>, checked_at: &str) -> Result<(), ManagedCliError> {
    if read_fixed_official_cli(driver).is_none() || checked_at.is_empty() ||
        checked_at.len() > 64 || checked_at.chars().any(char::is_control) ||
        notice.is_some_and(|value| value.is_empty() || value.len() > 128 ||
            !value.bytes().all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))) {
        return Err(ManagedCliError::Invalid);
    }
    transaction(db, |db| {
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
    let row = Statement::prepare(db.as_ptr(), "SELECT state,COALESCE(version,''),COALESCE(stage_name,''),COALESCE(previous_version,''),progress_bytes,COALESCE(raw_error,''),COALESCE(checked_at,''),COALESCE(official_notice,''),revision FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1, driver)?;
    if !row.step_row()? { return Ok(None); }
    let option = |index| -> Result<Option<String>, ManagedCliError> {
        let text = row.column_text(index)?;
        Ok(if text.is_empty() { None } else { Some(text) })
    };
    let mut result = ManagedCliCopy { driver: driver.to_owned(), state: row.column_text(0)?,
        version: option(1)?, stage_name: option(2)?, previous_version: option(3)?,
        progress_bytes: row.column_text(4)?.parse().map_err(|_| ManagedCliError::Invalid)?,
        raw_error: option(5)?, checked_at: option(6)?, official_notice: option(7)?,
        revision: row.column_text(8)?.parse().map_err(|_| ManagedCliError::Invalid)? };
    if row.step_row()? { return Err(ManagedCliError::Invalid); }
    if result.state == "READY" {
        match result.stage_name.as_deref() {
            Some(name) => if let Err(error) = inspect_staged_official_cli(root, driver, name) {
                result.state = "BLOCKED".to_owned();
                result.raw_error = Some(format!("{error:?}"));
            },
            None => {
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
    let pin = read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
    let row = Statement::prepare(db.as_ptr(),
        "SELECT state,COALESCE(stage_name,''),COALESCE(image_sha256,''),COALESCE(version,'') FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    row.bind_text(1, driver)?;
    if !row.step_row()? { return Ok(None); }
    let state = row.column_text(0)?;
    let stage_name = row.column_text(1)?;
    let image_sha256 = row.column_text(2)?;
    let stored_version = row.column_text(3)?;
    if row.step_row()? || state != "READY" || image_sha256 != pin.image_sha256 ||
        stored_version != pin.version || version != pin.version ||
        digest != format!("sha256:{}", pin.image_sha256) {
        return Err(ManagedCliError::IdentityChanged);
    }
    Ok(Some(inspect_staged_official_cli(root, driver, &stage_name)?))
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
    let stage_name = transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        no_unsettled_instance_use(db, driver)?;
        let instance = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) LIMIT 1")?;
        instance.bind_text(1, driver)?;
        if instance.step_row()? { return Err(ManagedCliError::Busy); }
        let current = Statement::prepare(db.as_ptr(),
            "SELECT state,COALESCE(stage_name,''),revision FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        current.bind_text(1, driver)?;
        if !current.step_row()? { return Err(ManagedCliError::Invalid); }
        let state = current.column_text(0)?;
        let name = current.column_text(1)?;
        let revision = current.column_text(2)?.parse::<i64>().map_err(|_| ManagedCliError::Invalid)?;
        if current.step_row()? || revision != expected_revision ||
            !(state == "READY" || state == "STAGED" || state == "BLOCKED" || state == "UNINSTALLING") {
            return Err(ManagedCliError::Busy);
        }
        let pin = read_fixed_official_cli(driver).ok_or(ManagedCliError::Unsupported)?;
        if !safe_stage_name(&name, pin) { return Err(ManagedCliError::IdentityChanged); }
        if state != "UNINSTALLING" {
            let write = Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_cli_copies SET state='UNINSTALLING',revision=revision+1 WHERE driver_id=?1 AND revision=?2")?;
            write.bind_text(1, driver)?; write.bind_i64(2, revision)?; write.step_done()?;
        }
        Ok(name)
    })?;
    let staging = managed_cli_root(root)?;
    let target = staging.join(&stage_name);
    // Canonical parent equality verifies the resolved absolute deletion target
    // is one direct child of the product-owned staging directory.
    let target_exists = match fs::symlink_metadata(&target) {
        Ok(_) => true,
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if target_exists {
        plain_dir(&target)?;
        let resolved_parent = fs::canonicalize(&staging)?;
        if fs::canonicalize(&target)?.parent() != Some(resolved_parent.as_path()) {
            return Err(ManagedCliError::IdentityChanged);
        }
        verify_plain_tree(&target)?;
        fs::remove_dir_all(&target)?;
    }
    transaction(db, |db| {
        check_owner_in_current_transaction(db, owner)?;
        no_unsettled_instance_use(db, driver)?;
        let instance = Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) LIMIT 1")?;
        instance.bind_text(1, driver)?;
        if instance.step_row()? { return Err(ManagedCliError::Busy); }
        match fs::symlink_metadata(&target) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Ok(_) => return Err(ManagedCliError::Busy),
            Err(error) => return Err(error.into()),
        }
        let write = Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_cli_copies SET state='NOT_INSTALLED',version=NULL,archive_sha256=NULL,image_sha256=NULL,stage_name=NULL,previous_version=NULL,previous_image_sha256=NULL,previous_stage_name=NULL,progress_bytes=0,raw_error=NULL,revision=revision+1 WHERE driver_id=?1 AND state='UNINSTALLING'")?;
        write.bind_text(1, driver)?; write.step_done()?;
        Ok(())
    })
}
