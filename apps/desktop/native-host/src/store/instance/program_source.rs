//! Exact per-instance program source. New managed registrations bind their
//! private copy; old registrations remain legacy until explicit quiescent
//! migration, even when the old and new executable digests happen to match.

use super::{catalog, managed_cli, registry};
use crate::root::RootLock;
use crate::store::atomic::Statement;
use crate::store::authority::{check_owner_in_current_transaction, OwnerIssuer};
use crate::store::orchestration::OrchestrationError;
use crate::store::same_open::VerifiedDatabaseConnection;
use std::path::PathBuf;

#[derive(Debug)]
pub(crate) enum ProgramSourceError {
    Invalid,
    Conflict,
    Managed(managed_cli::ManagedCliError),
    Legacy(catalog::CatalogError),
    Registry(registry::RegistryError),
    Store(OrchestrationError),
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
    // Once a managed lifecycle row exists, every new/unmigrated instance is
    // ineligible. Old global bytes remain available only before that point.
    let managed=Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_instance_cli_copies WHERE driver_id=?1")?;
    managed.bind_text(1,driver)?;
    if managed.step_row()?{return Err(ProgramSourceError::Conflict)}
    catalog::locate_pinned_program(driver,digest,version).map_err(ProgramSourceError::Legacy)
}
