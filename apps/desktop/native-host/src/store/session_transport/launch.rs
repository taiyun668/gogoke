//! Native launch evidence for the product's existing E/F/H composition.
//! Logical IDs select stored facts; they never supply a path or permission.
use super::runtime::{self, ClaimObservation, InstancePin, SessionPhase};
use super::provider_evidence::commands;
use super::session_binding::{self, Provenance, SessionBinding};
use crate::process::{AppContainerProfile, CompatModule, DirectoryRoots, NativeBinding, PrepareRequest, ProcessLaunch};
use crate::root::{RootIdentity, RootLock};
use crate::store::authority::{self, OwnerIssuer, ProductIdentitySnapshot};
use crate::store::instance::{self, InstanceLaunchHomes, ResolvedDirectory};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, NativeOrigin, PermissionTier, Seat, State};
use crate::store::seat::HostEscalationProof;
use crate::store::inbox::host_rule::{self as host_rule, HostRecipient};
use crate::store::worktree::{self, ResolvedBinding};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::os::windows::fs::MetadataExt;

// Model guidance only: LPAC and the bound native assets still enforce access.
// Keep the official base instructions and the original tool failure evidence.
const CODEX_WINDOWS_SHELL_ENVIRONMENT: &str =
    "This gogoke session runs inside a Windows LPAC profile. For shell tools, \
     explicitly use exec_command with shell=\"cmd.exe\" and login=false, and use CMD syntax. \
     This host does not provision a PowerShell runtime for the session. \
     Do not invoke PowerShell or add unprovisioned executable assets. \
     Use the existing native file tools for file edits. If a tool fails, preserve and \
     report its original error and exit code; do not automatically retry it. \
     These instructions do not grant permissions; the native LPAC boundary remains authoritative.";

fn evidence<T, E: std::fmt::Debug>(value: Result<T, E>) -> Result<T, String> {
    value.map_err(|error| format!("native session launch: {error:?}"))
}

fn evidence_at<T, E: std::fmt::Debug>(stage: &str, value: Result<T, E>) -> Result<T, String> {
    value.map_err(|error| format!("native session launch [{stage}]: {error:?}"))
}

/// Only this module can construct the witness. Retain it through stop; the
/// compatibility roots and F's .git file guard must outlive the actual child.
pub(crate) struct LaunchEvidence {
    identity: ProductIdentitySnapshot,
    seat: Seat,
    claim: ClaimObservation,
    pin: InstancePin,
    homes: InstanceLaunchHomes,
    private_history: Option<instance::PrivateHistoryReceipt>,
    credential: Option<super::credential_launch::CredentialLaunch>,
    grok_home: Option<super::grok_home_launch::GrokHomeLaunch>,
    launch_request_id: String,
    directory: LaunchDirectory,
    profile: AppContainerProfile,
    profile_name: String,
    program: PathBuf,
    program_identity: RootIdentity,
    code_mode: Option<super::codex_component::BoundCodexComponent>,
    module: Option<Arc<CompatModule>>,
    directory_roots: Option<Arc<DirectoryRoots>>,
    tier: PermissionTier,
    resume_old: Option<ClaimObservation>,
    resume_request_id: Option<String>,
    launch_admission: Option<seat::NativeLeadAdmission>,
    host_guard: Option<(HostEscalationProof,HostRecipient)>,
}

/// Constructed only after the current E/F/H physical write check.
pub(crate) struct GrokPermissionScope { basis: String, target: String }
impl GrokPermissionScope {
    pub(crate) fn basis(&self) -> &str { &self.basis }
    pub(crate) fn target(&self) -> &str { &self.target }
}

fn physical_grok_write_target(root:&RootLock,profile:&AppContainerProfile,
    worktree:&Path,identity:&RootIdentity,target:&Path)->bool {
    let Some(spelling)=target.to_str() else {return false};
    let local=spelling.strip_prefix(r"\\?\").unwrap_or(spelling);
    let bytes=local.as_bytes();
    if bytes.len()<4 || !bytes[0].is_ascii_alphabetic() || bytes[1]!=b':'
        || bytes[2]!=b'\\' || local.contains('/') {return false;}
    let normalized=if spelling.starts_with(r"\\?\") {target.to_path_buf()}
        else {PathBuf::from(format!(r"\\?\{spelling}"))};
    let target=normalized.as_path();
    if !target.is_absolute() || target.components().any(|component| match component {
        std::path::Component::CurDir|std::path::Component::ParentDir=>true,
        std::path::Component::Normal(name)=>name.to_string_lossy().eq_ignore_ascii_case(".git")
            || name.to_string_lossy().contains(':'),
        _=>false,
    }) {return false;}
    let Ok(relative)=target.strip_prefix(worktree) else {return false};
    if relative.as_os_str().is_empty() {return false;}
    let Some(parent)=target.parent() else {return false};
    let Some(leaf)=target.file_name() else {return false};
    let Some(relative_parent)=relative.parent() else {return false};
    let Ok(canonical_root)=std::fs::canonicalize(worktree) else {return false};
    let Ok(canonical_parent)=std::fs::canonicalize(parent) else {return false};
    if canonical_parent!=canonical_root.join(relative_parent)
        || !canonical_parent.starts_with(&canonical_root) {return false;}
    // This existing verifier rejects a changed F FileID, inherited ACL
    // mismatch, existing reparse descendants and multiply linked files.
    if profile.verify_bound_tree_grant(worktree,identity,true).is_err() {return false;}
    let Ok(parent_identity)=crate::root::inspect_root(parent) else {return false};
    let Ok(_held_parent)=DirectoryRoots::prepare(root,
        &[(parent.to_path_buf(),parent_identity.identity)]) else {return false};
    match std::fs::symlink_metadata(target) {
        Ok(meta) => {
            if !meta.is_file() || meta.file_attributes() & 0x400 != 0 {return false;}
            let Ok(actual)=std::fs::canonicalize(target) else {return false};
            actual==canonical_parent.join(leaf)
        },
        Err(error) if error.kind()==std::io::ErrorKind::NotFound=>true,
        Err(_)=>false,
    }
}

#[cfg(all(test,windows))]
mod grok_permission_tests {
    use super::*;
    use std::time::{SystemTime,UNIX_EPOCH};

    #[test]
    fn grok_permission_write_target_stays_in_the_bound_physical_f_tree() {
        let stamp=SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base=std::env::temp_dir().join(format!("gogoke-grok-permission-{stamp}-{}",std::process::id()));
        std::fs::create_dir(&base).unwrap();
        let root=RootLock::acquire(&base).unwrap();
        let base=root.canonical_root().canonical_path.clone();
        let f=base.join("registered-f");
        let peer=base.join("peer-f");
        let home=base.join("private-home");
        std::fs::create_dir(&f).unwrap();
        std::fs::create_dir(&peer).unwrap();
        std::fs::create_dir(&home).unwrap();
        let existing=f.join("source.txt");
        std::fs::write(&existing,b"synthetic source").unwrap();
        std::fs::write(f.join(".git"),b"synthetic pointer").unwrap();
        let profile=AppContainerProfile::derived_for_test("Gogoke37.GrokPermissionPhysical").unwrap();
        let identity=crate::root::inspect_root(&f).unwrap().identity;
        profile.grant_bound_tree(&f,&identity,true).unwrap();
        assert!(physical_grok_write_target(&root,&profile,&f,&identity,&existing));
        let dos=existing.to_string_lossy().strip_prefix(r"\\?\").unwrap().to_owned();
        assert!(physical_grok_write_target(&root,&profile,&f,&identity,Path::new(&dos)));
        assert!(physical_grok_write_target(&root,&profile,&f,&identity,&f.join("new-source.txt")));
        for denied in [peer.join("source.txt"),home.join("auth.json"),
            f.join(".git"),f.join(".git").join("config"),f.join("source.txt:stream"),
            base.join("registered-f-peer").join("source.txt")] {
            assert!(!physical_grok_write_target(&root,&profile,&f,&identity,&denied),
                "outside, git and alternate stream paths are not F writes");
        }
        let outside=peer.join("outside.txt");
        std::fs::write(&outside,b"synthetic peer").unwrap();
        let hardlink=f.join("hardlink.txt");
        std::fs::hard_link(&outside,&hardlink).unwrap();
        assert!(!physical_grok_write_target(&root,&profile,&f,&identity,&hardlink));
        std::fs::remove_file(&hardlink).unwrap();
        let link=f.join("reparse.txt");
        if std::os::windows::fs::symlink_file(&outside,&link).is_ok() {
            assert!(!physical_grok_write_target(&root,&profile,&f,&identity,&link));
            std::fs::remove_file(&link).unwrap();
        }
        drop(root);
        std::fs::remove_dir_all(base).unwrap();
    }
}

/// Project source access and the global secretary's private cwd are different
/// native grants. There is no repository-shaped placeholder for the latter.
enum LaunchDirectory {
    ProjectWorktrees { repository_id:String, worktree:ResolvedBinding,
        group:Vec<ResolvedBinding> },
    SecretarySessionHome(ResolvedDirectory),
}

impl LaunchDirectory {
    fn cwd(&self)->&Path {
        match self {
            Self::ProjectWorktrees {worktree,..}=>&worktree.path,
            Self::SecretarySessionHome(home)=>&home.path,
        }
    }
    fn identity(&self)->&RootIdentity {
        match self {
            Self::ProjectWorktrees {worktree,..}=>&worktree.identity,
            Self::SecretarySessionHome(home)=>&home.identity,
        }
    }
}

fn verify_host_guard(db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    host:Option<(&HostEscalationProof,&HostRecipient)>)->Result<(),String> {
    if let Some((proof,choice))=host {
        unsafe extern "C" {fn sqlite3_get_autocommit(database:*mut std::ffi::c_void)->i32;}
        let own_transaction=unsafe {sqlite3_get_autocommit(db.as_ptr())}!=0;
        if own_transaction {evidence(db.execute("BEGIN IMMEDIATE"))?;}
        let checked=evidence(host_rule::revalidate_host_recipient_in_transaction(db,owner,proof,choice));
        if own_transaction {
            match checked {
                Ok(_)=>evidence(db.execute("COMMIT"))?,
                Err(primary)=>{
                    if let Err(rollback)=db.execute("ROLLBACK") {
                        return Err(format!("{primary}; host guard rollback: {rollback:?}"));
                    }
                    return Err(primary);
                },
            }
        } else {checked?;}
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum VerificationPhase { PreActivation, Active }

// F's native pin, rather than a wire setting, selects the only launch recipes
// supported here. Agy has no qualified binary observation yet.
fn supported_driver_version(pin: &InstancePin) -> bool {
    matches!((pin.driver_id.as_str(), pin.version.as_str()),
        ("codex", "0.160.0") | ("claude", "2.1.196") |
        ("opencode", "1.18.32") | ("grok", "1.0.41"))
}

fn launch_homes(db: &VerifiedDatabaseConnection<'_>, root: &RootLock,
    profile: &AppContainerProfile, claim: &ClaimObservation, pin: &InstancePin)
    -> Result<InstanceLaunchHomes, String> {
    if pin.driver_id == "codex" {
        evidence(instance::resolve_codex_session_launch_homes(db, root, profile,
            &claim.instance_id, &claim.home_id, &claim.domain_id,
            &claim.session_id, &claim.generation))
    } else {
        evidence(instance::resolve_provider_session_launch_homes(db, root, profile,
            &claim.instance_id, &claim.home_id, &claim.domain_id,
            &claim.session_id, &claim.generation, &pin.driver_id))
    }
}

impl LaunchEvidence {
    pub(crate) fn instance_id(&self) -> &str { &self.claim.instance_id }
    pub(crate) fn driver_id(&self) -> &str { &self.pin.driver_id }
    pub(crate) fn driver_version(&self) -> &str { &self.pin.version }
    /// The first native open's sealed E/F/H selection. The caller persists it
    /// only after verify_in_transaction and the original H process binding.
    pub(crate) fn initial_session_binding(&self) -> Result<SessionBinding, String> {
        if self.resume_old.is_some() {
            return Err("native session binding: resume cannot mint an initial relationship".into());
        }
        Ok(SessionBinding {
            domain_id: self.claim.domain_id.clone(),
            session_id: self.claim.session_id.clone(),
            seat_id: self.seat.seat_id.clone(),
            seat_incarnation: self.seat.incarnation.clone(),
            seat_authorization_generation: self.seat.generation,
            selected_instance_id: self.claim.instance_id.clone(),
            provenance: Provenance::NativeV2,
        })
    }
    /// After H records the first prepared process/episode and A registers the
    /// session, choose provenance from the original sealed H admission facts.
    /// The User/open caller cannot supply or change that choice.
    pub(crate) fn record_initial_session_binding_in_transaction(&self,
        db:&VerifiedDatabaseConnection<'_>,operation:&str)->Result<(),String> {
        let mut binding=self.initial_session_binding()?;
        if evidence(seat::get(db,&self.seat.domain_id,&self.seat.seat_id))?.as_ref()
            !=Some(&self.seat) {
            return Err("native initial relationship: sealed E seat changed".into());
        }
        let registration=evidence(crate::store::ledger::read_registered_session(db,
            &self.claim.session_id))?.ok_or("native initial relationship: A registration absent")?;
        if registration.domain_id!=self.claim.domain_id
            || registration.session_id!=self.claim.session_id
            || registration.seat_id!=self.seat.seat_id
            || (registration.purpose==crate::store::ledger::SessionPurpose::Secretary)
                !=matches!(&self.directory,LaunchDirectory::SecretarySessionHome(_)) {
            return Err("native initial relationship: A registration changed".into());
        }
        let original=crate::store::atomic::Statement::prepare(db.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_h_claim a
              JOIN main.gogoke_v37_h_owner_binding o ON o.binding_id=a.binding_id
                AND o.instance_id=a.instance_id AND o.domain_id=a.domain_id
                AND o.owner_id=a.session_id AND o.generation=a.generation
                AND o.kind='SESSION' AND o.state='ACTIVE'
              JOIN main.gogoke_v37_instance_homes h ON h.home_id=a.home_id
                AND h.instance_id=a.instance_id AND h.domain_id=a.domain_id
                AND h.owner_id=a.session_id AND h.generation=a.generation
                AND h.kind='SESSION' AND h.state='ACTIVE'
              JOIN main.gogoke_v37_h_process_episode e ON e.domain_id=a.domain_id
                AND e.session_id=a.session_id AND e.generation=a.generation
                AND e.process_operation_id=a.process_operation_id
                AND e.instance_id=a.instance_id AND e.home_id=a.home_id
                AND e.binding_id=a.binding_id AND e.old_generation IS NULL
              JOIN main.gogoke_v37_h_generation g ON g.domain_id=e.domain_id
                AND g.session_id=e.session_id AND g.generation=e.generation
                AND g.request_id=e.request_id AND g.process_operation_id=e.process_operation_id
              JOIN main.gogoke_coordination_process_custody c
                ON c.operation_id=e.process_operation_id AND c.domain_id=e.domain_id
                AND c.generation=e.generation AND c.profile_id=e.instance_id
             WHERE a.domain_id=?1 AND a.session_id=?2 AND a.instance_id=?3
               AND a.home_id=?4 AND a.binding_id=?5 AND a.generation=?6
               AND a.revision=?7 AND a.process_operation_id=?8
               AND a.state='COMMITTED' AND e.phase='PREPARED'
               AND e.seat_id=?9 AND e.seat_incarnation=?10 AND c.state='PREPARED'")
            .map_err(|error|format!("native initial relationship query: {error:?}"))?;
        for (index,value) in [self.claim.domain_id.as_str(),self.claim.session_id.as_str(),
            self.claim.instance_id.as_str(),self.claim.home_id.as_str(),
            self.claim.binding_id.as_str(),self.claim.generation.as_str()].iter().enumerate() {
            original.bind_text((index+1) as i32,value)
                .map_err(|error|format!("native initial relationship bind: {error:?}"))?;
        }
        original.bind_i64(7,self.claim.revision)
            .map_err(|error|format!("native initial relationship revision: {error:?}"))?;
        for (index,value) in [operation,self.seat.seat_id.as_str(),self.seat.incarnation.as_str()]
            .iter().enumerate() {
            original.bind_text((index+8) as i32,value)
                .map_err(|error|format!("native initial relationship custody bind: {error:?}"))?;
        }
        if !original.step_row().map_err(|error|
            format!("native initial relationship read: {error:?}"))?
            || original.step_row().map_err(|error|
                format!("native initial relationship duplicate: {error:?}"))? {
            return Err("native initial relationship: original H process/episode absent".into());
        }
        drop(original);
        match evidence(session_binding::read_pending(db,&binding.domain_id,&binding.session_id))? {
            Some(pending) if pending==binding =>
                evidence(session_binding::insert_native_in_transaction(db,&binding)),
            Some(_) => Err("native initial relationship: pending selection changed".into()),
            None => {
                binding.provenance=Provenance::LegacyV1;
                evidence(session_binding::insert_legacy_initial_in_transaction(db,&binding))
            },
        }
    }
    // A vendor protocol setting, never an OS grant. The same sealed tier has
    // already selected and verified the LPAC capability set and directory ACLs.
    pub(crate) fn network_access(&self) -> bool { self.tier == PermissionTier::NetworkedWrite }
    pub(crate) fn host_tools_enabled(&self) -> bool {
        seat::orchestration_scope(&self.seat).is_ok()
    }

    pub(crate) fn observe(
        db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
        domain_id: &str, seat_id: &str, session_id: &str,
        repository_id: &str, worktree_id: &str,
        request_id: &str, retained: &mut Vec<super::credential_launch::CredentialPreparationCustody>,
    ) -> Result<Self, String> {
        Self::observe_with_origin(db,root,owner,&NativeOrigin::user(owner),
            domain_id,seat_id,session_id,repository_id,worktree_id,request_id,retained)
    }

    pub(crate) fn observe_with_origin(
        db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,host:&OwnerIssuer,
        origin:&NativeOrigin<'_>,domain_id:&str,seat_id:&str,session_id:&str,
        repository_id:&str,worktree_id:&str,
        request_id:&str,retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>,
    )->Result<Self,String> {
        Self::observe_with_guard(db,root,host,origin,domain_id,seat_id,session_id,
            repository_id,worktree_id,request_id,None,false,retained)
    }

    pub(crate) fn observe_host(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
        owner:&OwnerIssuer,domain_id:&str,seat_id:&str,session_id:&str,
        repository_id:&str,worktree_id:&str,proof:&HostEscalationProof,
        choice:&HostRecipient,request_id:&str,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>)->Result<Self,String> {
        Self::observe_with_guard(db,root,owner,&NativeOrigin::user(owner),domain_id,seat_id,
            session_id,repository_id,worktree_id,request_id,Some((proof,choice)),false,retained)
    }

    pub(crate) fn observe_secretary(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
        owner:&OwnerIssuer,seat_id:&str,session_id:&str,request_id:&str,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>)->Result<Self,String> {
        Self::observe_with_guard(db,root,owner,&NativeOrigin::user(owner),"global",seat_id,
            session_id,"","",request_id,None,true,retained)
    }

    fn observe_with_guard(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
        host:&OwnerIssuer,origin:&NativeOrigin<'_>,domain_id:&str,seat_id:&str,session_id:&str,
        repository_id:&str,worktree_id:&str,
        request_id:&str,
        guard:Option<(&HostEscalationProof,&HostRecipient)>,
        secretary:bool,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>)->Result<Self,String> {
        verify_host_guard(db,host,guard)?;
        let identity = evidence(authority::read_product_identity(db, host))?;
        let seat = evidence(seat::get(db, domain_id, seat_id))?
            .ok_or("native session launch: missing seat")?;
        if seat.state != State::Busy { return Err("native session launch: seat is not busy".into()); }
        let claim = evidence(runtime::observe_claim(db, origin,
            domain_id, seat_id, session_id))?.ok_or("native session launch: missing claim")?;
        if claim.phase != SessionPhase::Committed || claim.process_operation_id.is_some() {
            return Err("native session launch: claim is not an unused committed reservation".into());
        }
        let admission=match origin {
            NativeOrigin::Lead(admission)=>Some((*admission).clone()),
            NativeOrigin::User(_)=>None,
        };
        Self::build(db,root,host,identity,seat,claim,repository_id,worktree_id,None,None,admission,guard,secretary,request_id,retained)
    }

    pub(crate) fn observe_resume(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        owner: &OwnerIssuer, domain_id: &str, seat_id: &str, session_id: &str,
        repository_id: &str, worktree_id: &str, request_id: &str,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>) -> Result<Self,String> {
        Self::observe_resume_with_guard(db,root,owner,domain_id,seat_id,session_id,
            repository_id,worktree_id,request_id,None,retained)
    }

    pub(crate) fn observe_host_resume(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
        owner:&OwnerIssuer,domain_id:&str,seat_id:&str,session_id:&str,
        repository_id:&str,worktree_id:&str,request_id:&str,
        proof:&HostEscalationProof,choice:&HostRecipient,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>)->Result<Self,String> {
        Self::observe_resume_with_guard(db,root,owner,domain_id,seat_id,session_id,
            repository_id,worktree_id,request_id,Some((proof,choice)),retained)
    }

    fn observe_resume_with_guard(db:&mut VerifiedDatabaseConnection<'_>,root:&RootLock,
        owner:&OwnerIssuer,domain_id:&str,seat_id:&str,session_id:&str,
        repository_id:&str,worktree_id:&str,request_id:&str,
        guard:Option<(&HostEscalationProof,&HostRecipient)>,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>)->Result<Self,String> {
        verify_host_guard(db,owner,guard)?;
        let identity=evidence(authority::read_product_identity(db,owner))?;
        let seat=evidence(seat::get(db,domain_id,seat_id))?.ok_or("native resume: seat absent")?;
        if seat.state!=State::Busy {return Err("native resume: seat not busy".into());}
        let old=evidence(runtime::observe_claim(db,&NativeOrigin::user(owner),
            domain_id,seat_id,session_id))?.ok_or("native resume: claim absent")?;
        if old.phase!=SessionPhase::Stopped || old.process_operation_id.is_none() {
            return Err("native resume: old generation not stopped".into());
        }
        let old_process=old.process_operation_id.as_deref()
            .ok_or("native resume: old process absent")?;
        let custody=crate::store::atomic::Statement::prepare(db.as_ptr(),
            "SELECT c.binary_digest_sha256 FROM main.gogoke_coordination_process_custody c
              JOIN main.gogoke_v37_h_claim a ON a.process_operation_id=c.operation_id
                AND a.domain_id=c.domain_id AND a.generation=c.generation
              WHERE c.operation_id=?1 AND c.domain_id=?2 AND c.generation=?3
                AND c.state='STOPPED' AND c.stop_proof_hash=a.stop_fact_id
                AND a.stop_fact_id IS NOT NULL")
            .map_err(|error|format!("native resume old custody: {error:?}"))?;
        custody.bind_text(1,old_process).map_err(|error|format!("native resume old custody: {error:?}"))?;
        custody.bind_text(2,domain_id).map_err(|error|format!("native resume old custody: {error:?}"))?;
        custody.bind_text(3,&old.generation).map_err(|error|format!("native resume old custody: {error:?}"))?;
        if !custody.step_row().map_err(|error|format!("native resume old custody: {error:?}"))? {
            return Err("native resume: old custody proof absent".into());
        }
        let old_digest=custody.column_text(0).map_err(|error|
            format!("native resume old digest: {error:?}"))?;
        if custody.step_row().map_err(|error|format!("native resume duplicate custody: {error:?}"))? {
            return Err("native resume: duplicate old custody".into());
        }
        let pin=evidence(runtime::current_instance_pin(db,&old.instance_id))?;
        if !supported_driver_version(&pin) || pin.digest!=old_digest {
            return Err("native resume: trusted pinned binary changed".into());
        }
        let row=crate::store::atomic::Statement::prepare(db.as_ptr(),
            "SELECT generation,home_id,binding_id,instance_id FROM main.gogoke_v37_h_process_episode
              WHERE domain_id=?1 AND request_id=?2 AND session_id=?3
                AND old_generation=?4 AND phase='INTENT' AND process_operation_id IS NULL")
            .map_err(|error|format!("native resume candidate: {error:?}"))?;
        for (index,value) in [domain_id,request_id,session_id,old.generation.as_str()].iter().enumerate() {
            row.bind_text((index+1) as i32,value).map_err(|error|format!("native resume candidate: {error:?}"))?;
        }
        if !row.step_row().map_err(|error|format!("native resume candidate: {error:?}"))? {
            return Err("native resume candidate absent".into());
        }
        let generation=row.column_text(0).map_err(|error|format!("native resume generation: {error:?}"))?;
        let home_id=row.column_text(1).map_err(|error|format!("native resume home: {error:?}"))?;
        let binding_id=row.column_text(2).map_err(|error|format!("native resume binding: {error:?}"))?;
        let instance_id=row.column_text(3).map_err(|error|format!("native resume instance: {error:?}"))?;
        if row.step_row().map_err(|error|format!("native resume duplicate: {error:?}"))?
            || instance_id!=old.instance_id {return Err("native resume candidate conflict".into());}
        let candidate=ClaimObservation {generation,home_id,binding_id,instance_id,
            phase:SessionPhase::Committed,process_operation_id:None,..old.clone()};
        let secretary=evidence(crate::store::ledger::read_registered_session(db,session_id))?
            .is_some_and(|registered|registered.domain_id==domain_id && registered.seat_id==seat_id
                && registered.purpose==crate::store::ledger::SessionPurpose::Secretary);
        Self::build(db,root,owner,identity,seat,candidate,repository_id,worktree_id,
            Some(old),Some(request_id.to_owned()),None,guard,secretary,request_id,retained)
    }

    fn build(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
        identity: ProductIdentitySnapshot, seat: Seat, claim: ClaimObservation,
        repository_id: &str, worktree_id: &str, resume_old: Option<ClaimObservation>,
        resume_request_id: Option<String>,launch_admission:Option<seat::NativeLeadAdmission>,
        host_guard:Option<(&HostEscalationProof,&HostRecipient)>,secretary:bool,request_id:&str,
        retained:&mut Vec<super::credential_launch::CredentialPreparationCustody>) -> Result<Self,String> {
        verify_host_guard(db,owner,host_guard)?;
        let domain_id=&claim.domain_id;
        let session_id=&claim.session_id;
        let seat_id=&seat.seat_id;
        if secretary {
            if domain_id!="global" || !repository_id.is_empty() || !worktree_id.is_empty()
                || launch_admission.is_some() || host_guard.is_some() {
                return Err("native secretary launch: project or delegated authority supplied".into());
            }
            let current=evidence(seat::require_secretary_session(db,owner,seat_id,&seat.incarnation))?;
            if current!=seat {return Err("native secretary launch: designated seat changed".into());}
        }
        let tier=evidence(seat::permission_tier(&seat))?;
        let pin = evidence(runtime::current_instance_pin(db, &claim.instance_id))?;
        if !supported_driver_version(&pin) {
            return Err("native session launch: unsupported pinned driver/version".into());
        }
        let suffix = crate::store::digest::sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            root.canonical_root().identity.opaque(), domain_id, session_id,
            seat.incarnation, claim.generation).as_bytes());
        let profile_name = format!("Gogoke37.Session.{}", &suffix[..40]);
        verify_host_guard(db,owner,host_guard)?;
        let profile = evidence(AppContainerProfile::ensure_for_cli(&profile_name,
            tier == PermissionTier::NetworkedWrite))?;
        let homes = launch_homes(db, root, &profile, &claim, &pin)?;
        let private_history = if pin.driver_id == "codex" {
            let input = instance::PrivateHistoryLaunch {
                instance_id:&claim.instance_id,domain_id:&claim.domain_id,
                session_id:&claim.session_id,seat_id:&seat.seat_id,
                seat_incarnation:&seat.incarnation,binding_id:&claim.binding_id,
                generation:&claim.generation,request_id,
            };
            Some(if let Some(old)=&resume_old {
                let prior=evidence(instance::read_private_history_generation(db,
                    &old.binding_id,&old.generation))?
                    .ok_or("native resume: original private history absent; preserve legacy history")?;
                let source=prior.source.as_ref()
                    .ok_or("native resume: original private history source absent")?;
                if old.process_operation_id.as_deref()!=Some(source.process_operation_id.as_str()) {
                    return Err("native resume: private history custody changed".into());
                }
                let stopped_source=crate::store::atomic::Statement::prepare(db.as_ptr(),
                    "SELECT 1 FROM main.gogoke_coordination_process_custody
                      WHERE operation_id=?1 AND ticket=?2 AND custodian_nonce=?3
                        AND domain_id=?4 AND generation=?5 AND state='STOPPED'
                        AND stop_proof_hash IS NOT NULL AND stop_proof_hash<>''")
                    .map_err(|error|format!("native history original custody: {error:?}"))?;
                for (index,value) in [source.process_operation_id.as_str(),source.ticket.as_str(),
                    source.custodian_nonce.as_str(),old.domain_id.as_str(),old.generation.as_str()].iter().enumerate() {
                    stopped_source.bind_text((index+1) as i32,value)
                        .map_err(|error|format!("native history original custody bind: {error:?}"))?;
                }
                if !stopped_source.step_row().map_err(|error|format!("native history original custody read: {error:?}"))?
                    || stopped_source.step_row().map_err(|error|format!("native history duplicate custody: {error:?}"))? {
                    return Err("native history: original stopped custody absent".into());
                }
                drop(stopped_source);
                let stopped=instance::StoppedPrivateHistory {history_id:&prior.history_id,
                    binding_id:&prior.binding_id,generation:&prior.generation,
                    request_id:&prior.request_id,source};
                evidence(instance::resume_private_history(db,root,&input,&stopped))?
            } else {evidence(instance::create_initial_private_history(db,root,&input))?})
        } else {None};
        let credential=if let Some(history)=&private_history {
            super::credential_launch::CredentialLaunch::prepare(db,root,&profile,history,request_id,retained)?
        } else {None};
        let grok_home=if pin.driver_id=="grok" {
            Some(super::grok_home_launch::GrokHomeLaunch::prepare(db,root,&profile,
                &profile_name,&claim,&pin,&homes.instance,&seat.seat_id,&seat.incarnation,request_id)?)
        } else {None};
        // Retain the exact credential witness across all remaining fallible
        // preparation. No process factory has been called in this builder.
        let prepared = (|| -> Result<_, String> {
        let directory=if secretary {
            LaunchDirectory::SecretarySessionHome(homes.session.clone())
        } else {
            let worktree=evidence(worktree::resolve_for_launch(db, root, worktree_id,
                repository_id, domain_id, seat_id, &seat.incarnation, seat.generation))?;
            let group=evidence(worktree::resolve_group_for_launch(db, root, &worktree))?;
            LaunchDirectory::ProjectWorktrees {repository_id:repository_id.into(),worktree,group}
        };
        // F's stored instance/tier/generation describe creation provenance.
        // Current execution authority comes from the E seat and H claim;
        // a legitimate idle instance rebind does not change the worktree.
        let program = evidence(instance::locate_bound_instance_program(db,&claim.instance_id,&pin.driver_id, &pin.digest, &pin.version))?;
        let program_identity = evidence_at("capture-pinned-program-identity",
            AppContainerProfile::capture_catalog_program_identity(&program))?;
        // Runtime home writes are separate from workspace permission. No
        // public parent, other session, source tree or common Git dir is granted.
        verify_host_guard(db,owner,host_guard)?;
        let model_home=private_history.as_ref().map(|history|&history.directory).unwrap_or(&homes.instance);
        if let Some(grok)=&grok_home {
            grok.verify(db,&profile,true)?;
        } else if let Some(credential)=&credential {
            evidence_at("grant-registered-credential-tree", profile.grant_registered_credential_tree(&model_home.path,&model_home.identity,
                &credential.binding,&credential.alias,true))?;
        } else {
            evidence_at("grant-model-home", profile.grant_bound_tree(&model_home.path, &model_home.identity, true))?;
        }
        verify_host_guard(db,owner,host_guard)?;
        evidence_at("grant-session-home", profile.grant_bound_tree(&homes.session.path, &homes.session.identity, true))?;
        let writable = matches!(tier, PermissionTier::IsolatedWrite | PermissionTier::NetworkedWrite);
        if let LaunchDirectory::ProjectWorktrees {group,..}=&directory {
            for (member_index,member) in group.iter().enumerate() {
                verify_host_guard(db,owner,host_guard)?;
                evidence_at(&format!("grant-worktree-member-{member_index}"),
                    profile.grant_bound_tree(&member.path,&member.identity,writable))?;
            }
        }
        verify_host_guard(db,owner,host_guard)?;
        evidence_at("grant-pinned-program", profile.grant_bound_catalog_program(&program, &program_identity))?;
        let code_mode = if pin.driver_id == "codex" {
            verify_host_guard(db,owner,host_guard)?;
            Some(super::codex_component::BoundCodexComponent::prepare(&program, &profile)?)
        } else { None };
        let mut roots = vec![
            (model_home.path.clone(), model_home.identity.clone()),
            (homes.session.path.clone(), homes.session.identity.clone()),
        ];
        if let LaunchDirectory::ProjectWorktrees {group,..}=&directory {
            roots.extend(group.iter().map(|member|(member.path.clone(),member.identity.clone())));
        }
        let (module, directory_roots) = if pin.driver_id == "codex" {
            verify_host_guard(db,owner,host_guard)?;
            (Some(evidence(CompatModule::prepare_with_roots(root, &roots, &profile, &profile_name))?), None)
        } else if pin.driver_id == "claude" && pin.digest == format!("sha256:{}", gogoke_lpac_path_compat::OBSERVED_CLAUDE_SHA256) {
            verify_host_guard(db,owner,host_guard)?;
            let directories = Arc::new(evidence_at("prepare-compat-directory-roots", DirectoryRoots::prepare(root, &roots))?);
            let module = evidence(CompatModule::prepare_claude_observation(root, &profile, &profile_name))?;
            (Some(module), Some(directories))
        } else {
            verify_host_guard(db,owner,host_guard)?;
            (None, Some(Arc::new(evidence_at("prepare-compat-directory-roots", DirectoryRoots::prepare(root, &roots))?)))
        };
        Ok((directory, program, program_identity, code_mode, module, directory_roots))
        })();
        let (directory, program, program_identity, code_mode, module, directory_roots) =
            match prepared {
                Ok(prepared) => prepared,
                Err(original) => {
                    let cleanup = if let Some(grok)=&grok_home {
                        grok.revoke_uncreated(db,root,&profile)
                    } else if let Some(credential)=&credential {
                        credential.revoke_uncreated(db,root,&profile,request_id)
                    } else {Ok(())};
                    return Err(format!("{original}; uncreated credential settlement: {cleanup:?}"));
                }
            };
        let observed = Self { identity, seat, claim, pin, homes, private_history,
            credential,grok_home,
            launch_request_id:request_id.into(),directory,
            profile, profile_name, program, program_identity, code_mode, module, directory_roots, tier,
            resume_old,resume_request_id,launch_admission,
            host_guard:host_guard.map(|(proof,choice)|(proof.clone(),choice.clone())) };
        if let Err(original) = observed.verify(db, root, owner, None) {
            let cleanup = observed.revoke_uncreated_credential(db,root);
            return Err(format!("{original}; uncreated credential settlement: {cleanup:?}"));
        }
        Ok(observed)
    }

    /// Recheck immediately before prepare and again before activate. A saved
    /// witness is not a grant; current Owner/E/F/H facts must still agree.
    pub(crate) fn verify(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, expected_operation: Option<&str>) -> Result<(), String> {
        let identity = evidence(authority::read_product_identity(db, owner))?;
        self.verify_snapshot(db, root, owner, expected_operation, self.claim.revision,
            identity, VerificationPhase::PreActivation)
    }

    pub(crate) fn verify_in_transaction(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, expected_operation: Option<&str>) -> Result<(), String> {
        let identity = evidence(authority::read_product_identity_in_current_transaction(db, owner))?;
        self.verify_snapshot(db, root, owner, expected_operation, self.claim.revision,
            identity, VerificationPhase::PreActivation)
    }

    /// Called only after the exact prepared child has been activated.
    pub(crate) fn verify_active_in_transaction(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, expected_operation: Option<&str>) -> Result<(), String> {
        let identity = evidence(authority::read_product_identity_in_current_transaction(db, owner))?;
        self.verify_snapshot(db, root, owner, expected_operation, self.claim.revision,
            identity, VerificationPhase::Active)
    }

    pub(crate) fn verify_live(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, operation: &str, revision: i64) -> Result<(), String> {
        let identity = evidence(authority::read_product_identity(db, owner))?;
        self.verify_snapshot(db, root, owner, Some(operation), revision,
            identity, VerificationPhase::Active)
    }

    pub(crate) fn verify_live_in_transaction(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, operation: &str, revision: i64) -> Result<(), String> {
        let identity = evidence(authority::read_product_identity_in_current_transaction(db, owner))?;
        self.verify_snapshot(db, root, owner, Some(operation), revision,
            identity, VerificationPhase::Active)
    }

    /// Permission for one pinned Grok Write target, derived only from the
    /// current E/H witness and F's registered physical worktree group.
    /// `None` is a host denial, never an alternate HOME or source grant.
    pub(crate) fn grok_write_target(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, operation: &str, revision: i64,
        raw_path: &str) -> Result<Option<GrokPermissionScope>, String> {
        self.verify_live(db,root,owner,operation,revision)?;
        if self.pin.driver_id!="grok" || !matches!(self.tier,
            PermissionTier::IsolatedWrite|PermissionTier::NetworkedWrite) {
            return Ok(None);
        }
        let LaunchDirectory::ProjectWorktrees {group,..}=&self.directory else {return Ok(None)};
        let target=Path::new(raw_path);
        for member in group {
            if !physical_grok_write_target(root,&self.profile,&member.path,&member.identity,target) {
                continue;
            }
            return Ok(Some(GrokPermissionScope { basis:format!("{}\n{}\n{:?}\n{}",
                member.worktree_id,member.identity.opaque(),self.tier,
                crate::store::digest::sha256_hex(raw_path.as_bytes())),
                target:raw_path.to_owned() }));
        }
        Ok(None)
    }

    pub(crate) fn adopt_resume(&mut self, db: &VerifiedDatabaseConnection<'_>,
        owner: &OwnerIssuer, operation: &str) -> Result<(),String> {
        if self.resume_old.is_none() {return Err("native resume evidence already adopted".into());}
        let current_seat=evidence(seat::get(db,&self.seat.domain_id,&self.seat.seat_id))?
            .ok_or("native resume seat disappeared")?;
        let initial=evidence(session_binding::read(db,&self.claim.domain_id,&self.claim.session_id))?;
        let pending=evidence(session_binding::read_pending(db,&self.claim.domain_id,&self.claim.session_id))?;
        let native=match (initial.as_ref(),pending.as_ref()) {
            (Some(binding),Some(selection)) if binding.provenance==Provenance::NativeV2
                && binding==selection => {
                if current_seat!=self.seat
                    || binding.seat_id!=self.seat.seat_id
                    || binding.seat_incarnation!=self.seat.incarnation
                    || binding.seat_authorization_generation!=self.seat.generation
                    || binding.selected_instance_id!=self.claim.instance_id {
                    return Err("native resume: sealed E selection changed".into());
                }
                true
            },
            (None,None)=>false,
            (Some(binding),None) if binding.provenance==Provenance::LegacyV1=>false,
            _=>return Err("native resume: original relationship changed".into()),
        };
        let next_claim=evidence(runtime::observe_claim(db,&NativeOrigin::user(owner),
            &self.claim.domain_id,&self.seat.seat_id,&self.claim.session_id))?
            .ok_or("native resume claim disappeared")?;
        if next_claim.process_operation_id.as_deref()!=Some(operation)
            || next_claim.generation!=self.claim.generation
            || next_claim.instance_id!=self.claim.instance_id
            || next_claim.home_id!=self.claim.home_id
            || next_claim.binding_id!=self.claim.binding_id
            || next_claim.phase!=SessionPhase::Committed {
            return Err("native resume operation not current".into());
        }
        if !native {self.seat=current_seat;}
        self.claim=next_claim;
        self.resume_old=None;
        self.resume_request_id=None;
        Ok(())
    }

    fn verify_snapshot(&self, db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        owner: &OwnerIssuer, expected_operation: Option<&str>, revision: i64,
        identity: ProductIdentitySnapshot, phase: VerificationPhase) -> Result<(), String> {
        verify_host_guard(db,owner,self.host_guard.as_ref().map(|(p,c)|(p,c)))?;
        if identity != self.identity
            || evidence(seat::get(db, &self.seat.domain_id, &self.seat.seat_id))?.as_ref() != Some(&self.seat)
            || evidence(runtime::current_instance_pin(db, &self.claim.instance_id))? != self.pin {
            return Err("native session launch: current identity/seat/pin changed".into());
        }
        if matches!(&self.directory,LaunchDirectory::SecretarySessionHome(_)) {
            let current=evidence(seat::require_secretary_session(db,owner,
                &self.seat.seat_id,&self.seat.incarnation))?;
            if current!=self.seat {return Err("native secretary launch: current designation changed".into());}
        }
        let current = evidence(runtime::observe_claim_bound(db,
            &self.claim.domain_id, &self.seat.seat_id, &self.claim.session_id))?
            .ok_or("native session launch: claim no longer current")?;
        if matches!(phase,VerificationPhase::PreActivation) {
            if let Some(admission)=&self.launch_admission {
                let same=evidence(runtime::observe_claim(db,&NativeOrigin::lead(admission),
                    &self.claim.domain_id,&self.seat.seat_id,&self.claim.session_id))?
                    .ok_or("native child launch: original reservation no longer current")?;
                if same!=current {return Err("native child launch: reservation identity changed".into());}
            }
        }
        let claim = if let Some(old)=&self.resume_old {
            if &current!=old || old.phase!=SessionPhase::Stopped
                || old.revision!=revision || old.process_operation_id.is_none() {
                return Err("native resume: stopped admission changed".into());
            }
            let request_id=self.resume_request_id.as_deref()
                .ok_or("native resume request identity absent")?;
            let candidate=crate::store::atomic::Statement::prepare(db.as_ptr(),
                "SELECT 1 FROM main.gogoke_v37_h_process_episode
                  WHERE domain_id=?1 AND session_id=?2 AND request_id=?3
                    AND old_generation=?4 AND generation=?5 AND instance_id=?6
                    AND home_id=?7 AND binding_id=?8
                    AND COALESCE(process_operation_id,'')=?9
                    AND phase IN ('INTENT','PREPARED','ACTIVE')")
                .map_err(|error|format!("native resume candidate verify: {error:?}"))?;
            let operation=expected_operation.unwrap_or("");
            for (index,value) in [self.claim.domain_id.as_str(),self.claim.session_id.as_str(),
                request_id,old.generation.as_str(),self.claim.generation.as_str(),
                self.claim.instance_id.as_str(),self.claim.home_id.as_str(),
                self.claim.binding_id.as_str(),operation].iter().enumerate() {
                candidate.bind_text((index+1) as i32,value)
                    .map_err(|error|format!("native resume candidate bind: {error:?}"))?;
            }
            if !candidate.step_row().map_err(|error|format!("native resume candidate check: {error:?}"))?
                || candidate.step_row().map_err(|error|format!("native resume duplicate candidate: {error:?}"))? {
                return Err("native resume candidate changed".into());
            }
            self.claim.clone()
        } else {current};
        // prepare persistence may attach a native custody operation; all
        // reservation facts and revisions remain fixed until activation.
        if claim.instance_id != self.claim.instance_id || claim.home_id != self.claim.home_id
            || claim.binding_id != self.claim.binding_id || claim.generation != self.claim.generation
            || claim.revision != revision || claim.phase != SessionPhase::Committed
            || (self.resume_old.is_none()
                && claim.process_operation_id.as_deref() != expected_operation) {
            return Err("native session launch: current reservation changed".into());
        }
        let homes = launch_homes(db, root, &self.profile, &claim, &self.pin)?;
        if homes != self.homes { return Err("native session launch: physical homes changed".into()); }
        if let Some(history)=&self.private_history {
            evidence(instance::verify_private_history(db,root,history,&self.history_input()))?;
        }
        if let Some(credential)=&self.credential {credential.verify(db,root,&self.profile)?;}
        // A owns immutable purpose and session membership. First open creates
        // this registration before activation; absence must never imply WORK.
        if expected_operation.is_some() {
            let registration=evidence(crate::store::ledger::read_registered_session(db,
                &self.claim.session_id))?.ok_or("native session launch: A registration absent")?;
            if registration.domain_id!=self.claim.domain_id || registration.seat_id!=self.seat.seat_id
                || (registration.purpose==crate::store::ledger::SessionPurpose::Secretary)
                    !=matches!(&self.directory,LaunchDirectory::SecretarySessionHome(_)) {
                return Err("native session launch: A registration binding changed".into());
            }
            if self.resume_old.is_some() && registration.purpose==crate::store::ledger::SessionPurpose::FormalReview {
                return Err("native resume: formal review continuation refused".into());
            }
        }
        if let LaunchDirectory::ProjectWorktrees {repository_id,worktree,group}=&self.directory {
            let current=evidence(worktree::resolve_for_launch(db,root,&worktree.worktree_id,
                repository_id,&self.seat.domain_id,&self.seat.seat_id,
                &self.seat.incarnation,self.seat.generation))?;
            if current.identity!=worktree.identity || current.pointer_hash!=worktree.pointer_hash
                || current.pointer_identity!=worktree.pointer_identity
                || current.common_identity!=worktree.common_identity {
                return Err("native session launch: physical worktree changed".into());
            }
            let current_group=evidence(worktree::resolve_group_for_launch(db,root,&current))?;
            if current_group.len()!=group.len() || current_group.iter().zip(group)
                .any(|(now,original)|now.worktree_id!=original.worktree_id
                    || now.identity!=original.identity || now.pointer_hash!=original.pointer_hash
                    || now.pointer_identity!=original.pointer_identity
                    || now.common_identity!=original.common_identity) {
                return Err("native session launch: physical worktree group changed".into());
            }
        } else if self.directory.cwd()!=self.homes.session.path
            || self.directory.identity()!=&self.homes.session.identity {
            return Err("native secretary launch: session home identity changed".into());
        }
        let program = evidence(instance::locate_bound_instance_program(db,&self.claim.instance_id,&self.pin.driver_id, &self.pin.digest, &self.pin.version))?;
        if program != self.program { return Err("native session launch: program path changed".into()); }
        if let Some(grok)=&self.grok_home {
            grok.verify(db,&self.profile,matches!(phase,VerificationPhase::PreActivation))?;
        }
        if let Some(credential)=&self.credential {
            match phase {
                VerificationPhase::PreActivation=>{
                    evidence(self.profile.verify_bound_credential_tree_grant(&self.model_home().path,
                        &self.model_home().identity,&credential.binding,&credential.alias,true))?;
                },
                VerificationPhase::Active=>{
                    evidence(self.profile.verify_bound_directory_grant(&self.model_home().path,
                        &self.model_home().identity,true))?;
                },
            }
        } else if self.grok_home.is_none() {
            match phase {
                VerificationPhase::PreActivation=>{evidence(self.profile.verify_bound_tree_grant(
                    &self.model_home().path,&self.model_home().identity,true))?;},
                VerificationPhase::Active=>{evidence(self.profile.verify_bound_directory_grant(
                    &self.model_home().path,&self.model_home().identity,true))?;},
            }
        }
        for (path, identity, writable) in [(&self.homes.session.path, &self.homes.session.identity, true)] {
            match phase {
                VerificationPhase::PreActivation =>
                    evidence(self.profile.verify_bound_tree_grant(path, identity, writable))?,
                VerificationPhase::Active =>
                    evidence(self.profile.verify_bound_directory_grant(path, identity, writable))?,
            };
        }
        let writable = matches!(self.tier, PermissionTier::IsolatedWrite | PermissionTier::NetworkedWrite);
        if let LaunchDirectory::ProjectWorktrees {group,..}=&self.directory {
            for member in group {
                match phase {
                    VerificationPhase::PreActivation => evidence(self.profile.verify_bound_tree_grant(
                        &member.path, &member.identity, writable))?,
                    VerificationPhase::Active => evidence(self.profile.verify_bound_directory_grant(
                        &member.path, &member.identity, writable))?,
                };
            }
        }
        evidence(self.profile.verify_bound_catalog_program_grant(&self.program, &self.program_identity))?;
        if let Some(code_mode) = &self.code_mode { code_mode.verify(&self.profile)?; }
        if let Some(module) = &self.module {
            let mut mapping = Vec::new();
            module.extend_environment(&mut mapping);
            evidence(module.validate_launch(Some(&self.profile_name), Some(&mapping)))?;
            if let Some(roots) = &self.directory_roots { evidence(roots.verify())?; }
            Ok(())
        } else {
            evidence(self.directory_roots.as_ref()
                .ok_or("native session launch: physical directory custody absent")?.verify())
        }
    }

    pub(crate) fn cwd(&self) -> &Path { self.directory.cwd() }

    fn model_home(&self)->&instance::ResolvedDirectory {
        self.private_history.as_ref().map(|history|&history.directory).unwrap_or(&self.homes.instance)
    }

    pub(crate) fn file_credentials_bound(&self)->bool {self.credential.is_some()}

    /// Root calls this only outside an H transaction, after its original
    /// admission/custody guard and before the next pure in-transaction verify.
    pub(crate) fn refresh_grok_readiness(&self,db:&mut VerifiedDatabaseConnection<'_>,
        _root:&RootLock,owner:&OwnerIssuer,
        custody:Option<&crate::process::PreparedCustody>)->Result<(),String>{
        evidence(authority::read_product_identity(db,owner))?;
        if let Some((proof,choice))=&self.host_guard {
            verify_host_guard(db,owner,Some((proof,choice)))?;
        }
        if let Some(grok)=&self.grok_home {
            let resume=self.resume_old.as_ref().map(|old|self.resume_request_id.as_deref()
                .map(|request|(old,request)).ok_or("Grok private HOME: resume request absent"))
                .transpose()?;
            grok.refresh_readiness(db,&self.profile,custody,resume)?;
        }
        evidence(authority::read_product_identity(db,owner))?;
        Ok(())
    }

    pub(crate) fn bind_grok_process(&self,db:&mut VerifiedDatabaseConnection<'_>,
        operation:&str,custody:&crate::process::PreparedCustody)->Result<(),String>{
        if let Some(grok)=&self.grok_home {grok.bind_process(db,operation,custody)?;}
        Ok(())
    }

    pub(crate) fn revoke_uncreated_credential(&self,db:&mut VerifiedDatabaseConnection<'_>,
        root:&RootLock)->Result<(),String> {
        if let Some(credential)=&self.credential {
            credential.revoke_uncreated(db,root,&self.profile,&self.launch_request_id)?;
        }
        if let Some(grok)=&self.grok_home {grok.revoke_uncreated(db,root,&self.profile)?;}
        Ok(())
    }

    pub(crate) fn revoke_stopped_credential(&self,db:&mut VerifiedDatabaseConnection<'_>,
        root:&RootLock,operation:&str,custody:&crate::process::PreparedCustody)->Result<(),String> {
        if let Some(credential)=&self.credential {
            credential.revoke(db,root,&self.profile,operation,custody)?;
        }
        if let Some(grok)=&self.grok_home {
            grok.revoke_stopped(db,root,&self.profile,operation,custody)?;
        }
        Ok(())
    }

    fn history_input(&self)->instance::PrivateHistoryLaunch<'_> {
        instance::PrivateHistoryLaunch {instance_id:&self.claim.instance_id,
            domain_id:&self.claim.domain_id,session_id:&self.claim.session_id,
            seat_id:&self.seat.seat_id,seat_incarnation:&self.seat.incarnation,
            binding_id:&self.claim.binding_id,generation:&self.claim.generation,
            request_id:&self.launch_request_id}
    }

    pub(crate) fn bind_original_history_source(&self,db:&VerifiedDatabaseConnection<'_>,
        custody:&crate::process::PreparedCustody,operation:&str)->Result<(),String> {
        if let Some(history)=&self.private_history {
            if custody.binding.domain_id!=self.claim.domain_id
                || custody.binding.generation!=self.claim.generation {
                return Err("native history: original custody binding changed".into());
            }
            evidence(instance::bind_private_history_continuation_in_transaction(db,history,
                &instance::PrivateHistorySource {process_operation_id:operation.into(),
                    ticket:custody.ticket.opaque().into(),custodian_nonce:custody.custodian_nonce.clone()}))?;
        }
        Ok(())
    }
    pub(crate) fn clear_host_guard(&mut self) {self.host_guard=None;}
    pub(crate) fn seat_id(&self) -> &str { &self.seat.seat_id }
    pub(crate) fn seat_incarnation(&self) -> &str { &self.seat.incarnation }
    pub(crate) fn permission_seat_generation(&self)->i64 {self.seat.generation}
    pub(crate) fn permission_seat_revision(&self)->i64 {self.seat.revision}
    pub(crate) fn permission_tier_name(&self)->&'static str {
        match self.tier {
            PermissionTier::ReadOnly=>"READ_ONLY",
            PermissionTier::NoNetwork=>"NO_NETWORK",
            PermissionTier::IsolatedWrite=>"ISOLATED_WRITE",
            PermissionTier::NetworkedWrite=>"NETWORKED_WRITE",
        }
    }

    pub(crate) fn verify_observed_cwd(&self, observed: &str) -> Result<(), String> {
        let path=Path::new(observed);
        if !path.is_absolute() || observed.contains('\0') {
            return Err(format!("native thread cwd is not absolute: observed={observed:?}"));
        }
        // Only known DOS/verbatim spellings of the already-bound local path
        // are inspected. Provider output cannot make this host open a new
        // target (including a remote share) merely to compare identities.
        let spelling=|value:&str| {
            let value=value.replace('/',"\\");
            value.strip_prefix("\\\\?\\").unwrap_or(&value).to_ascii_lowercase()
        };
        if spelling(observed)!=spelling(&self.directory.cwd().to_string_lossy()) {
            return Err(format!("native thread cwd outside bound path spellings: expected={:?}; observed={observed:?}",self.directory.cwd()));
        }
        let returned=crate::root::inspect_root(path).map_err(|error|
            format!("native thread cwd observation: expected={:?}; observed={observed:?}; error={error:?}",self.directory.cwd()))?;
        if &returned.identity != self.directory.identity() {
            return Err(format!("native thread physical cwd mismatch: expected={:?} identity={:?}; observed={observed:?} identity={:?}",
                self.directory.cwd(),self.directory.identity(),returned.identity));
        }
        Ok(())
    }

    pub(crate) fn settings(&self) -> Result<(String, String), String> {
        use crate::store::atomic::{Json, JsonString, Parser};
        let Json::Object(settings) = evidence(Parser::parse(self.seat.settings_json.as_deref()
            .ok_or("native session launch: seat settings missing")?))? else {
            return Err("native session launch: seat settings not object".into());
        };
        let field = |name: &str| -> Result<String, String> {
            match settings.get(&JsonString::from_str(name)) {
                Some(Json::String(value)) => value.to_well_formed_string()
                    .filter(|value| !value.is_empty() && !value.contains('\0'))
                    .ok_or_else(|| format!("native session launch: invalid {name}")),
                _ => Err(format!("native session launch: missing {name}")),
            }
        };
        Ok((field("model")?, evidence(seat::seat_effort(&self.seat))?))
    }

    pub(crate) fn request(&self) -> Result<PrepareRequest, String> {
        let system_root = evidence(std::env::var("SystemRoot"))?;
        if !Path::new(&system_root).is_absolute() || system_root.contains('\0') {
            return Err("native session launch: invalid SystemRoot".into());
        }
        let mut environment = vec![("SystemRoot".into(), system_root.clone()), ("WINDIR".into(), system_root)];
        let runtime = self.homes.session.path.to_string_lossy().into_owned();
        let instance_home = self.model_home().path.to_string_lossy().into_owned();
        if self.pin.driver_id == "codex" {
            for name in ["HOME", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "TEMP", "TMP"] {
                environment.push((name.into(), runtime.clone()));
            }
            environment.push(("CODEX_HOME".into(), instance_home));
        } else {
            // The registered instance root owns vendor credentials/config; the
            // ACTIVE generation's runtime home owns only temporary files.
            for name in ["HOME", "USERPROFILE"] {
                environment.push((name.into(), instance_home.clone()));
            }
            let app_data = self.homes.instance.path.join("AppData");
            environment.push(("APPDATA".into(), app_data.join("Roaming").to_string_lossy().into_owned()));
            environment.push(("LOCALAPPDATA".into(), app_data.join("Local").to_string_lossy().into_owned()));
            for name in ["TEMP", "TMP"] { environment.push((name.into(), runtime.clone())); }
            match self.pin.driver_id.as_str() {
                "claude" => {
                    environment.push(("CLAUDE_CONFIG_DIR".into(), instance_home));
                    // This requests the documented setting; effective memory
                    // behavior remains NOT_RUN until the pinned CLI is tested.
                    environment.push(("CLAUDE_CODE_DISABLE_AUTO_MEMORY".into(), "1".into()));
                }
                "opencode" => {
                    let home = &self.homes.instance.path;
                    for (name, path) in [
                        ("XDG_CONFIG_HOME", home.join(".config")),
                        ("XDG_DATA_HOME", home.join(".local").join("share")),
                        ("XDG_CACHE_HOME", home.join(".cache")),
                        ("XDG_STATE_HOME", home.join(".local").join("state")),
                        ("OPENCODE_CONFIG_DIR", home.join(".opencode")),
                        ("OPENCODE_CONFIG", home.join(".opencode").join("opencode.json")),
                    ] { environment.push((name.into(), path.to_string_lossy().into_owned())); }
                    for (name, value) in [
                        ("OPENCODE_CONFIG_CONTENT", "{}"),
                        ("OPENCODE_DISABLE_CLAUDE_CODE", "1"),
                        ("OPENCODE_DISABLE_CLAUDE_CODE_PROMPT", "1"),
                        ("OPENCODE_DISABLE_CLAUDE_CODE_SKILLS", "1"),
                    ] { environment.push((name.into(), value.into())); }
                }
                "grok" => environment.push(("GROK_HOME".into(), instance_home)),
                _ => return Err("native session launch: unsupported pinned driver/version".into()),
            }
        }
        if let Some(module) = &self.module { module.extend_environment(&mut environment); }
        let mut launch = ProcessLaunch::new(self.program.clone());
        launch.arguments = if self.pin.driver_id == "codex" { vec!["-c".into(), "features.memories=false".into(),
            "-c".into(), "memories.generate_memories=false".into(),
            "-c".into(), "memories.use_memories=false".into(),
            "-c".into(), "agents.enabled=false".into(),
            "-c".into(), "features.multi_agent_v2=false".into(),
            "-c".into(), "features.default_mode_request_user_input=true".into(),
            "-c".into(), "tools.experimental_request_user_input.enabled=true".into(),
            "-c".into(), format!("developer_instructions={}", crate::store::atomic::Json::String(
                crate::store::atomic::JsonString::from_str(CODEX_WINDOWS_SHELL_ENVIRONMENT)).canonical()),
            "-c".into(), format!("sqlite_home={}", crate::store::atomic::Json::String(
                crate::store::atomic::JsonString::from_str(&runtime)).canonical()),
            "-c".into(), format!("log_dir={}", crate::store::atomic::Json::String(
                crate::store::atomic::JsonString::from_str(&runtime)).canonical()),
            "app-server".into()] } else { match self.pin.driver_id.as_str() {
                "claude" => {
                    let (model,effort)=self.settings()?;
                    let mut args=evidence(commands::claude_launch_args(&model,&effort,None))?;
                    // The fixed 2.1.196 stdio permission host is required for
                    // AskUserQuestion control_request frames. This flag does
                    // not grant approval for ordinary tool permissions.
                    args.push("--permission-prompt-tool".into());
                    args.push("stdio".into());
                    // Reuse Room's tier mapping: approved workspace edits
                    // must not become an extra Owner permission prompt.
                    // The outer LPAC grants remain the execution boundary.
                    args.push("--permission-mode".into());
                    args.push(if matches!(self.tier,
                        PermissionTier::IsolatedWrite|PermissionTier::NetworkedWrite) {
                        "acceptEdits".into()
                    } else { "plan".into() });
                    // The fixed Agent SDK exposes the same debug-file option.
                    // Keep the CLI's own diagnostics in the original private
                    // runtime HOME; H does not read credentials or log data.
                    args.push("--debug-file".into());
                    args.push(self.homes.session.path.join("claude-startup-debug.log")
                        .to_string_lossy().into_owned());
                    args
                },
                // The pinned top-level --pure switch disables external plugins;
                // it does not by itself prove memory isolation or model choice.
                "opencode" => vec!["--pure".into(), "acp".into()],
                "grok" => {
                    let (model, effort) = self.settings()?;
                    evidence(commands::grok_launch_args(&model, &effort))?
                },
                _ => return Err("native session launch: unsupported pinned driver/version".into()),
            }};
        if self.file_credentials_bound() {
            launch.arguments.splice(0..0,["-c".to_owned(),"cli_auth_credentials_store=\"file\"".to_owned()]);
        }
        launch.current_directory = Some(self.directory.cwd().to_path_buf());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.environment = Some(environment);
        launch.app_container_profile = Some(self.profile_name.clone());
        launch.app_container_internet_client = self.tier == PermissionTier::NetworkedWrite;
        launch.app_container_cli_identity_services = true;
        launch.path_compat = self.module.clone();
        launch.directory_roots = self.directory_roots.clone();
        if let LaunchDirectory::ProjectWorktrees {group,..}=&self.directory {
            let guards=group.iter().map(|member|evidence(member.retained_pointer()))
                .collect::<Result<Vec<_>,String>>()?;
            launch.worktree_guard=Some(Arc::new(guards));
        }
        Ok(PrepareRequest { launch, binding: NativeBinding {
            binary_digest_sha256: self.pin.digest.clone(), profile_id: self.claim.instance_id.clone(),
            domain_id: self.claim.domain_id.clone(), generation: self.claim.generation.clone(),
        } })
    }
}
