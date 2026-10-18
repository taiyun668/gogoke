//! Exact per-instance program source. New managed registrations bind their
//! private copy. Older registrations require explicit quiescent migration;
//! matching executable digests alone never supply a source.

use super::{catalog, managed_cli, registry};
use crate::root::RootLock;
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
use std::path::PathBuf;
use crate::process::{CredentialAliasScope, CredentialBinding};

#[derive(Debug)]
pub(crate) enum ProgramSourceError {
    Invalid,
    Conflict,
    Managed(managed_cli::ManagedCliError),
    Legacy(catalog::CatalogError),
    Registry(registry::RegistryError),
    Store(OrchestrationError),
    Credential(String),
    Observation(String),
}

/// Full mutable copy/source state is kept apart from the immutable historical
/// execution snapshot. Length framing preserves NULL and empty string via
/// SQLite's quote(), and binds every legal self-write to the next action step.
pub(crate) fn managed_version_fingerprints(db:&VerifiedDatabaseConnection<'_>,
    root:&RootLock,driver:&str)->Result<(String,String),ProgramSourceError> {
    let observed=crate::root::inspect_root(&root.canonical_root().canonical_path)
        .map_err(|error|ProgramSourceError::Observation(format!("version root observation: {error:?}")))?;
    if observed.identity!=root.canonical_root().identity ||
        db.root_identity()!=&observed.identity {
        return Err(ProgramSourceError::Conflict);
    }
    fn rows(db:&VerifiedDatabaseConnection<'_>,driver:&str,sql:&str,columns:i32)
        ->Result<String,ProgramSourceError> {
        let q=Statement::prepare(db.as_ptr(),sql)?;
        q.bind_text(1,driver)?;
        let mut bytes=Vec::new();
        while q.step_row()? {
            bytes.extend_from_slice(b"row:");
            for n in 0..columns {
                let value=q.column_text(n)?;
                bytes.extend_from_slice(value.len().to_string().as_bytes());
                bytes.push(b':');bytes.extend_from_slice(value.as_bytes());
            }
        }
        Ok(crate::store::digest::content_hash(&bytes))
    }
    let copy=rows(db,driver,
        "SELECT quote(driver_id),quote(state),quote(version),quote(archive_sha256),
            quote(image_sha256),quote(stage_name),quote(previous_version),
            quote(previous_image_sha256),quote(previous_stage_name),quote(progress_bytes),
            quote(raw_error),quote(checked_at),quote(official_notice),quote(revision)
         FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1",14)?;
    let source=rows(db,driver,
        "SELECT quote(i.instance_id),quote(i.driver_id),quote(i.home_ref),quote(i.home_identity),
            quote(i.program_digest),quote(i.version),quote(i.install_state),quote(i.login_state),quote(i.revision),
            quote(s.instance_id),quote(s.source),quote(s.stage_name),quote(s.program_digest),
            quote(s.version),quote(s.home_identity),quote(s.registration_request_id),quote(s.revision),
            quote(p.instance_id),quote(p.display_name),quote(p.enabled),quote(p.connected_model_source),
            quote(p.tombstoned),quote(p.revision)
         FROM main.gogoke_v37_instances i
         LEFT JOIN main.gogoke_v37_instance_program_sources s ON s.instance_id=i.instance_id
         LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id
         WHERE i.driver_id=?1 ORDER BY i.instance_id",23)?;
    Ok((copy,source))
}

/// Called inside the Owner's immediate transaction after ProductDatabase has
/// qualified every selected instance's original H resources. A deliberately
/// disabled legacy instance remains unbound and cannot launch after READY.
pub(crate) fn migrate_quiescent_legacy_instances(db:&VerifiedDatabaseConnection<'_>,
    root:&RootLock,owner:&OwnerIssuer,driver:&str,stage_name:&str,qualified:&[String])
    ->Result<usize,ProgramSourceError>{
    let pin=managed_cli::read_fixed_official_cli(driver).ok_or(ProgramSourceError::Invalid)?;
    let copy=managed_cli::read_managed_cli(db,root,driver)?
        .ok_or(ProgramSourceError::Conflict)?;
    if copy.state!="PROBED"||copy.stage_name.as_deref()!=Some(stage_name)||
        copy.version.as_deref()!=Some(pin.version){return Err(ProgramSourceError::Conflict)}
    managed_cli::inspect_staged_official_cli(root,driver,stage_name)?;
    managed_cli::no_unsettled_instance_use(db,driver)?;
    struct Candidate {
        id:String,digest:String,version:String,home:String,registration:String,
        _credential:Option<std::sync::Arc<CredentialBinding>>,
    }
    let rows=Statement::prepare(db.as_ptr(),
        "SELECT i.instance_id,i.program_digest,i.version,i.home_identity,i.login_state,COALESCE(CAST(p.enabled AS TEXT),'') FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) ORDER BY i.instance_id")?;
    rows.bind_text(1,driver)?;
    let mut candidates=Vec::new();
    while rows.step_row()?{
        let id=rows.column_text(0)?;
        let digest=rows.column_text(1)?;
        let version=rows.column_text(2)?;
        let home=rows.column_text(3)?;
        let login=rows.column_text(4)?;
        let enabled=rows.column_text(5)?;
        if digest!=format!("sha256:{}",pin.image_sha256)||version!=pin.version||
            registry::observed_home(root,&id)?.is_none_or(|identity|identity.opaque()!=home){
            return Err(ProgramSourceError::Conflict);
        }
        let registration=registry::verified_creation_request(db,root,&id)?;
        if enabled=="0" {continue}
        if !matches!(enabled.as_str(),""|"1") {return Err(ProgramSourceError::Conflict)}
        let _credential=if driver=="codex"{
            let object=super::credential_registry::read_credential_object(db,&id)
                .map_err(|error|ProgramSourceError::Credential(format!("credential object: {error:?}")))?;
            if object.is_none()&&login=="LOGGED_IN" {return Err(ProgramSourceError::Conflict)}
            if let Some(object)=object{
                let backend=super::credential_registry::read_usable_credential_backend(db,&id)
                    .map_err(|error|ProgramSourceError::Credential(format!("credential backend: {error:?}")))?;
                if backend.home_identity.opaque()!=home||object.home_identity.opaque()!=home{
                    return Err(ProgramSourceError::Conflict);
                }
                let source=root.canonical_root().canonical_path.join("v37-instances").join(&id).join("auth.json");
                let aliases=super::credential_registry::read_credential_aliases(db,&id)
                    .map_err(|error|ProgramSourceError::Credential(format!("credential aliases: {error:?}")))?;
                let mut scopes=Vec::new();
                for alias in aliases{
                    match alias.state.as_str(){
                        "ACTIVE"|"DORMANT"=>{
                            if alias.source_file_identity!=object.file_identity {return Err(ProgramSourceError::Conflict)}
                            let directory=super::private_history::resolve_private_history_directory(db,root,&alias.history_id)
                                .map_err(|error|ProgramSourceError::Credential(format!("alias directory: {error:?}")))?;
                            if directory.identity!=alias.directory_identity{return Err(ProgramSourceError::Conflict)}
                            scopes.push(CredentialAliasScope{root:directory.path,root_identity:directory.identity});
                        },
                        "REMOVED"=>(),
                        _=>return Err(ProgramSourceError::Conflict),
                    }
                }
                Some(CredentialBinding::open_registered(root,&source,&object.home_identity,
                    &object.file_identity,&scopes).map_err(|error|
                    ProgramSourceError::Credential(format!("original credential metadata: {error:?}")))?)
            }else{None}
        }else{
            // Claude/OpenCode use the registered physical home and the exact
            // H launch selectors, not a separately registered credential
            // FileID. Grok's H launch independently reobserves its original
            // HOME/auth identity. Moving identical executable bytes creates no
            // new credential claim; preserve those existing checks at launch.
            None
        };
        candidates.push(Candidate{id,digest,version,home,registration,_credential});
    }
    drop(rows);
    if candidates.iter().map(|candidate|&candidate.id).ne(qualified.iter()) {
        return Err(ProgramSourceError::Conflict);
    }
    check_owner_in_current_transaction(db,owner)?;
    managed_cli::no_unsettled_instance_use(db,driver)?;
    managed_cli::inspect_staged_official_cli(root,driver,stage_name)?;
    {
        for candidate in &candidates{
            let row=Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id=?2 AND program_digest=?3 AND version=?4 AND home_identity=?5")?;
            for (index,value) in [candidate.id.as_str(),driver,candidate.digest.as_str(),
                candidate.version.as_str(),candidate.home.as_str()].iter().enumerate(){
                row.bind_text((index+1) as i32,value)?;
            }
            if !row.step_row()?||row.step_row()?||registry::observed_home(root,&candidate.id)?
                .is_none_or(|identity|identity.opaque()!=candidate.home){
                return Err(ProgramSourceError::Conflict);
            }
            registry::verified_creation_request(db,root,&candidate.id)?;
            let prior=Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_instance_program_sources WHERE instance_id=?1")?;
            prior.bind_text(1,&candidate.id)?;
            if prior.step_row()?{return Err(ProgramSourceError::Conflict)}
            let write=Statement::prepare(db.as_ptr(),
                "INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES(?1,'MANAGED',?2,?3,?4,?5,?6,1)")?;
            for (index,value) in [candidate.id.as_str(),stage_name,candidate.digest.as_str(),
                candidate.version.as_str(),candidate.home.as_str(),candidate.registration.as_str()]
                .iter().enumerate(){write.bind_text((index+1) as i32,value)?;}
            write.step_done()?;
        }
        let update=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_cli_copies SET state='READY',revision=revision+1 WHERE driver_id=?1 AND state='PROBED' AND stage_name=?2 AND version=?3 AND image_sha256=?4")?;
        update.bind_text(1,driver)?;update.bind_text(2,stage_name)?;
        update.bind_text(3,pin.version)?;update.bind_text(4,pin.image_sha256)?;
        update.step_done()?;
        let check=Statement::prepare(db.as_ptr(),
            "SELECT state FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        check.bind_text(1,driver)?;
        if !check.step_row()?||check.column_text(0)?!="READY"||check.step_row()?{
            return Err(ProgramSourceError::Conflict);
        }
        Ok(candidates.len())
    }
}
impl From<crate::store::atomic::AtomicError> for ProgramSourceError {
    fn from(error: crate::store::atomic::AtomicError)->Self{
        Self::Store(OrchestrationError::Atomic(error))
    }
}
impl From<crate::store::same_open::SameOpenError> for ProgramSourceError {
    fn from(error: crate::store::same_open::SameOpenError)->Self{
        Self::Store(OrchestrationError::Atomic(error.into()))
    }
}
impl From<OrchestrationError> for ProgramSourceError {
    fn from(error:OrchestrationError)->Self{Self::Store(error)}
}
impl From<managed_cli::ManagedCliError> for ProgramSourceError {
    fn from(error:managed_cli::ManagedCliError)->Self{Self::Managed(error)}
}
impl From<registry::RegistryError> for ProgramSourceError {
    fn from(error:registry::RegistryError)->Self{Self::Registry(error)}
}

pub(crate) fn bind_managed_instance_program(db:&mut VerifiedDatabaseConnection<'_>,
    root:&RootLock,owner:&OwnerIssuer,instance_id:&str,stage_name:&str,
    registration_request_id:&str)->Result<(),ProgramSourceError>{
    if !registry::valid_id(instance_id)||stage_name.is_empty()||registration_request_id.is_empty(){
        return Err(ProgramSourceError::Invalid);
    }
    let instance=Statement::prepare(db.as_ptr(),
        "SELECT driver_id,program_digest,version,home_identity FROM main.gogoke_v37_instances WHERE instance_id=?1")?;
    instance.bind_text(1,instance_id)?;
    if !instance.step_row()?{return Err(ProgramSourceError::Conflict)}
    let driver=instance.column_text(0)?;
    let digest=instance.column_text(1)?;
    let version=instance.column_text(2)?;
    let home=instance.column_text(3)?;
    if instance.step_row()?||registry::observed_home(root,instance_id)?
        .is_none_or(|identity|identity.opaque()!=home){
        return Err(ProgramSourceError::Conflict);
    }
    drop(instance);
    let copy=managed_cli::read_managed_cli(db,root,&driver)?
        .ok_or(ProgramSourceError::Conflict)?;
    if copy.state!="READY"||copy.stage_name.as_deref()!=Some(stage_name)||
        copy.version.as_deref()!=Some(version.as_str())||
        managed_cli::locate_ready_managed_program(db,root,&driver,&digest,&version)?.is_none(){
        return Err(ProgramSourceError::Conflict);
    }
    db.execute("BEGIN IMMEDIATE")?;
    let result=(||{
        check_owner_in_current_transaction(db,owner)?;
        let retired=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instance_profiles WHERE instance_id=?1 AND tombstoned=1")?;
        retired.bind_text(1,instance_id)?;
        if retired.step_row()? {return Err(ProgramSourceError::Conflict)}
        let registration=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instance_operations WHERE request_id=?1 AND target_id=?2 AND phase='APPLIED'")?;
        registration.bind_text(1,registration_request_id)?;
        registration.bind_text(2,instance_id)?;
        if !registration.step_row()?||registration.step_row()?{
            return Err(ProgramSourceError::Conflict);
        }
        let current=Statement::prepare(db.as_ptr(),
            "SELECT source,stage_name,program_digest,version,home_identity,registration_request_id FROM main.gogoke_v37_instance_program_sources WHERE instance_id=?1")?;
        current.bind_text(1,instance_id)?;
        if current.step_row()?{
            let exact=current.column_text(0)?=="MANAGED"&&current.column_text(1)?==stage_name&&
                current.column_text(2)?==digest&&current.column_text(3)?==version&&
                current.column_text(4)?==home&&current.column_text(5)?==registration_request_id&&
                !current.step_row()?;
            return if exact{Ok(())}else{Err(ProgramSourceError::Conflict)};
        }
        let row=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id=?2 AND program_digest=?3 AND version=?4 AND home_identity=?5")?;
        for (index,value) in [instance_id,driver.as_str(),digest.as_str(),version.as_str(),home.as_str()]
            .iter().enumerate(){row.bind_text((index+1) as i32,value)?;}
        if !row.step_row()?||row.step_row()?{return Err(ProgramSourceError::Conflict)}
        if registry::observed_home(root,instance_id)?
            .is_none_or(|identity|identity.opaque()!=home){
            return Err(ProgramSourceError::Conflict);
        }
        managed_cli::locate_ready_managed_program(db,root,&driver,&digest,&version)?
            .ok_or(ProgramSourceError::Conflict)?;
        let write=Statement::prepare(db.as_ptr(),
            "INSERT INTO main.gogoke_v37_instance_program_sources(instance_id,source,stage_name,program_digest,version,home_identity,registration_request_id,revision) VALUES(?1,'MANAGED',?2,?3,?4,?5,?6,1)")?;
        for (index,value) in [instance_id,stage_name,digest.as_str(),version.as_str(),home.as_str(),registration_request_id]
            .iter().enumerate(){write.bind_text((index+1) as i32,value)?;}
        write.step_done()?;
        Ok(())
    })();
    match result {
        Ok(())=>db.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)
            .map_err(ProgramSourceError::Store),
        Err(error)=>{
            db.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
            Err(error)
        },
    }
}

/// Explicit USER version change after the exact target stage was H-probed.
/// The old immutable stage remains in place for a later separately authorized
/// rollback. No registration, HOME, credential, or historical H row is reset.
pub(crate) fn activate_quiescent_managed_version(
    db:&mut VerifiedDatabaseConnection<'_>, root:&RootLock, owner:&OwnerIssuer,
    driver:&str, target_version:&str, target_stage:&str, expected_copy_revision:i64,
    request_id:&str, request_bytes:&[u8],
) -> Result<usize,ProgramSourceError> {
    if expected_copy_revision<1 || target_stage.is_empty() {
        return Err(ProgramSourceError::Invalid);
    }
    let request=registry::ManagedSwitchRequest{request_bytes,request_id,driver,
        target_version,expected_copy_revision};
    registry::managed_request_command(&request)?;
    managed_cli::read_official_cli_version(driver,target_version)
        .ok_or(ProgramSourceError::Invalid)?;
    managed_cli::inspect_staged_official_cli_version(root,driver,target_version,target_stage)?;
    db.execute("BEGIN IMMEDIATE")?;
    let result=activate_quiescent_managed_version_in_current_transaction(db,root,owner,
        driver,target_version,target_stage,expected_copy_revision,request_id,request_bytes);
    match result {
        Ok(count)=>{db.execute("COMMIT").map_err(OrchestrationError::CommitUnknownWithCause)?; Ok(count)},
        Err(error)=>{db.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?; Err(error)},
    }
}

/// The F writer for an existing Owner-held immediate transaction. The caller
/// owns COMMIT/ROLLBACK and must keep its private live-memory gates in that
/// same synchronous ProductDatabase operation.
pub(crate) fn activate_quiescent_managed_version_in_current_transaction(
    db:&VerifiedDatabaseConnection<'_>, root:&RootLock, owner:&OwnerIssuer,
    driver:&str, target_version:&str, target_stage:&str, expected_copy_revision:i64,
    request_id:&str, request_bytes:&[u8],
) -> Result<usize,ProgramSourceError> {
    activate_managed_version_in_current_transaction(db,root,owner,driver,target_version,
        target_stage,expected_copy_revision,request_id,request_bytes,None,None)
}

/// A bound action shares the original F switch writer and its receipts. The
/// action does not authorize a second source writer or alter old H/C rows.
pub(crate) fn activate_managed_version_action_in_current_transaction(
    db:&VerifiedDatabaseConnection<'_>, root:&RootLock, owner:&OwnerIssuer,
    driver:&str, target_version:&str, target_stage:&str, expected_copy_revision:i64,
    request_id:&str, request_bytes:&[u8], action:&super::ActionRecord,
    releases:&dyn managed_cli::ClaudeReleaseVerifier,
) -> Result<usize,ProgramSourceError> {
    if action.request_id!=request_id || action.driver!=driver ||
        action.target_version!=target_version || action.stage_name!=target_stage ||
        action.phase!=super::ActionPhase::Probed {
        return Err(ProgramSourceError::Conflict);
    }
    activate_managed_version_in_current_transaction(db,root,owner,driver,target_version,
        target_stage,expected_copy_revision,request_id,request_bytes,Some(action),Some(releases))
}

fn activate_managed_version_in_current_transaction(
    db:&VerifiedDatabaseConnection<'_>, root:&RootLock, owner:&OwnerIssuer,
    driver:&str, target_version:&str, target_stage:&str, expected_copy_revision:i64,
    request_id:&str, request_bytes:&[u8], action:Option<&super::ActionRecord>,
    releases:Option<&dyn managed_cli::ClaudeReleaseVerifier>,
) -> Result<usize,ProgramSourceError> {
    if expected_copy_revision<1 || target_stage.is_empty() {
        return Err(ProgramSourceError::Invalid);
    }
    let request=registry::ManagedSwitchRequest{request_bytes,request_id,driver,
        target_version,expected_copy_revision};
    registry::managed_request_command(&request)?;
    let target=managed_cli::read_official_cli_version(driver,target_version)
        .ok_or(ProgramSourceError::Invalid)?;
    // The target must be one of the exact compiled versions and its stage
    // must still contain the pinned official bytes.
    managed_cli::inspect_staged_official_cli_version(root,driver,target_version,target_stage)?;
        check_owner_in_current_transaction(db,owner)?;
        registry::managed_request_command(&request)?;
        if let Some(action)=action {
            if let Some(releases)=releases {
                managed_cli::no_unsettled_managed_version_change_for_action_with_releases(
                    db,root,driver,action,releases)?;
            } else {
                managed_cli::no_unsettled_managed_version_change_for_action(db,root,driver,action)?;
            }
        } else {
            let pending=Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_instance_version_actions WHERE open_driver_id=?1 LIMIT 1")?;
            pending.bind_text(1,driver)?;
            if pending.step_row()? {return Err(ProgramSourceError::Conflict);}
            managed_cli::no_unsettled_managed_version_change(db,driver)?;
        }
        let copy=Statement::prepare(db.as_ptr(),
            "SELECT state,version,archive_sha256,image_sha256,stage_name,previous_version,previous_image_sha256,previous_stage_name,revision FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
        copy.bind_text(1,driver)?;
        if !copy.step_row()? {return Err(ProgramSourceError::Conflict)}
        let state=copy.column_text(0)?;
        let version=copy.column_text(1)?;
        let archive=copy.column_text(2)?;
        let image=copy.column_text(3)?;
        let stage=copy.column_text(4)?;
        let old_version=copy.column_text(5)?;
        let old_image=copy.column_text(6)?;
        let old_stage=copy.column_text(7)?;
        let revision=copy.column_text(8)?.parse::<i64>()
            .map_err(registry::RegistryError::RevisionParse)?;
        if copy.step_row()? || state!="PROBED" || version!=target.version ||
            archive!=target.archive_sha256 || image!=target.image_sha256 ||
            stage!=target_stage || revision!=expected_copy_revision ||
            old_version.is_empty() || old_version==target.version {
            return Err(ProgramSourceError::Conflict);
        }
        drop(copy);
        let old=managed_cli::read_official_cli_version(driver,&old_version)
            .ok_or(ProgramSourceError::Conflict)?;
        if old_image!=old.image_sha256 || old_stage.is_empty() {
            return Err(ProgramSourceError::Conflict);
        }
        managed_cli::inspect_staged_official_cli_version(root,driver,target_version,target_stage)?;
        managed_cli::inspect_staged_official_cli_version(root,driver,&old_version,&old_stage)?;
        let old_digest=format!("sha256:{}",old.image_sha256);
        let new_digest=format!("sha256:{}",target.image_sha256);
        let rows=Statement::prepare(db.as_ptr(),
            "SELECT i.instance_id,i.home_identity,i.version,i.program_digest,COALESCE(s.source,''),COALESCE(s.stage_name,''),COALESCE(s.version,''),COALESCE(s.program_digest,''),COALESCE(s.home_identity,''),COALESCE(s.registration_request_id,'') FROM main.gogoke_v37_instances i LEFT JOIN main.gogoke_v37_instance_profiles p ON p.instance_id=i.instance_id LEFT JOIN main.gogoke_v37_instance_program_sources s ON s.instance_id=i.instance_id WHERE i.driver_id=?1 AND (p.tombstoned IS NULL OR p.tombstoned=0) ORDER BY i.instance_id")?;
        rows.bind_text(1,driver)?;
        let mut instances=Vec::new();
        while rows.step_row()? {
            let id=rows.column_text(0)?;
            let home=rows.column_text(1)?;
            if rows.column_text(2)?!=old_version || rows.column_text(3)?!=old_digest ||
                rows.column_text(4)?!="MANAGED" || rows.column_text(5)?!=old_stage ||
                rows.column_text(6)?!=old_version || rows.column_text(7)?!=old_digest ||
                rows.column_text(8)?!=home ||
                registry::observed_home(root,&id)?.is_none_or(|identity|identity.opaque()!=home) ||
                rows.column_text(9)?!=registry::verified_creation_request(db,root,&id)? {
                return Err(ProgramSourceError::Conflict);
            }
            instances.push((id,home));
        }
        drop(rows);
        // One USER switch has one durable per-instance witness. A zero-instance
        // switch needs a separate driver-level journal and cannot claim APPLIED
        // through this instance-bound interface.
        if instances.is_empty() {return Err(ProgramSourceError::Conflict)}
        // No external process can race the held same-open transaction. Each
        // mutation still compares the exact prior source and instance tuple.
        for (id,home) in &instances {
            let before=Statement::prepare(db.as_ptr(),
                "SELECT revision FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id=?2 AND version=?3 AND program_digest=?4 AND home_identity=?5")?;
            for (index,value) in [id.as_str(),driver,old_version.as_str(),old_digest.as_str(),home.as_str()]
                .iter().enumerate(){before.bind_text((index+1) as i32,value)?;}
            if !before.step_row()? {return Err(ProgramSourceError::Conflict)}
            let before_revision=before.column_text(0)?.parse::<i64>()
                .map_err(registry::RegistryError::RevisionParse)?;
            if before.step_row()? || before_revision<1 || before_revision==i64::MAX {
                return Err(ProgramSourceError::Conflict);
            }
            drop(before);
            let instance=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instances SET version=?1,program_digest=?2,install_state='UNKNOWN',login_state='UNKNOWN',revision=revision+1 WHERE instance_id=?3 AND driver_id=?4 AND version=?5 AND program_digest=?6 AND home_identity=?7")?;
            for (index,value) in [target.version,new_digest.as_str(),id.as_str(),driver,
                old_version.as_str(),old_digest.as_str(),home.as_str()].iter().enumerate() {
                instance.bind_text((index+1) as i32,value)?;
            }
            instance.step_done()?;
            let changed=Statement::prepare(db.as_ptr(),"SELECT changes()")?;
            if !changed.step_row()? || changed.column_text(0)?!="1" || changed.step_row()? {
                return Err(ProgramSourceError::Conflict);
            }
            let source=Statement::prepare(db.as_ptr(),
                "UPDATE main.gogoke_v37_instance_program_sources SET stage_name=?1,version=?2,program_digest=?3,revision=revision+1 WHERE instance_id=?4 AND source='MANAGED' AND stage_name=?5 AND version=?6 AND program_digest=?7 AND home_identity=?8")?;
            for (index,value) in [target_stage,target.version,new_digest.as_str(),id.as_str(),
                old_stage.as_str(),old_version.as_str(),old_digest.as_str(),home.as_str()].iter().enumerate() {
                source.bind_text((index+1) as i32,value)?;
            }
            source.step_done()?;
            let changed=Statement::prepare(db.as_ptr(),"SELECT changes()")?;
            if !changed.step_row()? || changed.column_text(0)?!="1" || changed.step_row()? {
                return Err(ProgramSourceError::Conflict);
            }
            registry::record_managed_switch_witness(db,owner,&request,
                &registry::ManagedSwitchInstance{
                    instance_id:id,home,old_digest:&old_digest,old_version:&old_version,
                    old_stage:&old_stage,new_digest:&new_digest,new_stage:target_stage,
                    before_revision,after_revision:before_revision+1,
                })?;
            registry::verified_creation_request(db,root,id)?;
        }
        let ready=Statement::prepare(db.as_ptr(),
            "UPDATE main.gogoke_v37_instance_cli_copies SET state='READY',revision=revision+1 WHERE driver_id=?1 AND state='PROBED' AND version=?2 AND archive_sha256=?3 AND image_sha256=?4 AND stage_name=?5 AND previous_version=?6 AND previous_image_sha256=?7 AND previous_stage_name=?8 AND revision=?9")?;
        for (index,value) in [driver,target.version,target.archive_sha256,target.image_sha256,
            target_stage,old_version.as_str(),old.image_sha256,old_stage.as_str()].iter().enumerate() {
            ready.bind_text((index+1) as i32,value)?;
        }
        ready.bind_i64(9,expected_copy_revision)?;
        ready.step_done()?;
        let changed=Statement::prepare(db.as_ptr(),"SELECT changes()")?;
        if !changed.step_row()? || changed.column_text(0)?!="1" || changed.step_row()? {
            return Err(ProgramSourceError::Conflict);
        }
        Ok(instances.len())
}

/// H uses the stored source, never a caller path. An unbound instance cannot
/// silently adopt a managed copy merely because its old digest matches.
pub(crate) fn locate_bound_instance_program(db:&VerifiedDatabaseConnection<'_>,
    instance_id:&str,driver:&str,digest:&str,version:&str)->Result<PathBuf,ProgramSourceError>{
    if !registry::valid_id(instance_id){return Err(ProgramSourceError::Invalid)}
    let retired=Statement::prepare(db.as_ptr(),
        "SELECT tombstoned FROM main.gogoke_v37_instance_profiles WHERE instance_id=?1")?;
    retired.bind_text(1,instance_id)?;
    if retired.step_row()? {
        let state=retired.column_text(0)?;
        if retired.step_row()? || state=="1" { return Err(ProgramSourceError::Conflict); }
    }
    let source=Statement::prepare(db.as_ptr(),
        "SELECT source,stage_name,program_digest,version,home_identity FROM main.gogoke_v37_instance_program_sources WHERE instance_id=?1")?;
    source.bind_text(1,instance_id)?;
    if source.step_row()?{
        let kind=source.column_text(0)?;
        let stage=source.column_text(1)?;
        let pinned=source.column_text(2)?;
        let pinned_version=source.column_text(3)?;
        let home=source.column_text(4)?;
        if source.step_row()?||kind!="MANAGED"||pinned!=digest||pinned_version!=version{
            return Err(ProgramSourceError::Conflict);
        }
        let row=Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_instances WHERE instance_id=?1 AND driver_id=?2 AND program_digest=?3 AND version=?4 AND home_identity=?5")?;
        for (index,value) in [instance_id,driver,digest,version,home.as_str()]
            .iter().enumerate(){row.bind_text((index+1) as i32,value)?;}
        if !row.step_row()?||row.step_row()?{return Err(ProgramSourceError::Conflict)}
        let copy=Statement::prepare(db.as_ptr(),
            "SELECT stage_name FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1 AND state='READY'")?;
        copy.bind_text(1,driver)?;
        if !copy.step_row()?||copy.column_text(0)?!=stage||copy.step_row()?{
            return Err(ProgramSourceError::Conflict);
        }
        return managed_cli::locate_ready_managed_program_from_db(db,driver,digest,version)?
            .ok_or(ProgramSourceError::Conflict);
    }
    // No source row means no launch, including before the first managed copy.
    // Registration must use the independent official managed CLI path.
    Err(ProgramSourceError::Conflict)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::root::RootLock;
    use crate::store::same_open::{create_new, route_b_test_guard};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn registered_instance_without_source_conflicts_before_global_observation() {
        let _guard=route_b_test_guard();
        let nonce=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder=std::env::temp_dir().join(format!(
            "gogoke-managed-source-only-{}-{nonce}",std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root=RootLock::acquire(&folder).unwrap();
        let database=folder.join("state.sqlite");
        let mut db=create_new(&root,&database).unwrap();
        super::super::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:fixed','0.160.0','INSTALLED','LOGGED_IN',1)").unwrap();
        assert!(matches!(locate_bound_instance_program(&db,"instanceA","codex",
            "sha256:fixed","0.160.0"),Err(ProgramSourceError::Conflict)));
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(database).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }
}
