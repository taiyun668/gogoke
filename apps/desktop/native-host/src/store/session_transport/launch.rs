//! Native launch evidence for the product's existing E/F/H composition.
//! Logical IDs select stored facts; they never supply a path or permission.
use super::runtime::{self, ClaimObservation, InstancePin, SessionPhase};
use super::provider_evidence::commands;
use crate::process::{AppContainerProfile, CompatModule, DirectoryRoots, NativeBinding, PrepareRequest, ProcessLaunch};
use crate::root::{RootIdentity, RootLock};
use crate::store::authority::{self, OwnerIssuer, ProductIdentitySnapshot};
use crate::store::instance::{self, InstanceLaunchHomes};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, NativeOrigin, PermissionTier, Seat, State};
use crate::store::worktree::{self, ResolvedBinding};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// Only this module can construct the witness. Retain it through stop; the
/// compatibility roots and F's .git file guard must outlive the actual child.
pub(crate) struct LaunchEvidence {
    identity: ProductIdentitySnapshot,
    seat: Seat,
    claim: ClaimObservation,
    pin: InstancePin,
    homes: InstanceLaunchHomes,
    repository_id: String,
    worktree: ResolvedBinding,
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
    // A vendor protocol setting, never an OS grant. The same sealed tier has
    // already selected and verified the LPAC capability set and directory ACLs.
    pub(crate) fn network_access(&self) -> bool { self.tier == PermissionTier::NetworkedWrite }

    pub(crate) fn observe(
        db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
        domain_id: &str, seat_id: &str, session_id: &str,
        repository_id: &str, worktree_id: &str,
    ) -> Result<Self, String> {
        let identity = evidence(authority::read_product_identity(db, owner))?;
        let seat = evidence(seat::get(db, domain_id, seat_id))?
            .ok_or("native session launch: missing seat")?;
        if seat.state != State::Busy { return Err("native session launch: seat is not busy".into()); }
        let claim = evidence(runtime::observe_claim(db, &NativeOrigin::user(owner),
            domain_id, seat_id, session_id))?.ok_or("native session launch: missing claim")?;
        if claim.phase != SessionPhase::Committed || claim.process_operation_id.is_some() {
            return Err("native session launch: claim is not an unused committed reservation".into());
        }
        Self::build(db,root,owner,identity,seat,claim,repository_id,worktree_id,None,None)
    }

    pub(crate) fn observe_resume(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        owner: &OwnerIssuer, domain_id: &str, seat_id: &str, session_id: &str,
        repository_id: &str, worktree_id: &str, request_id: &str) -> Result<Self,String> {
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
        Self::build(db,root,owner,identity,seat,candidate,repository_id,worktree_id,
            Some(old),Some(request_id.to_owned()))
    }

    fn build(db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
        identity: ProductIdentitySnapshot, seat: Seat, claim: ClaimObservation,
        repository_id: &str, worktree_id: &str, resume_old: Option<ClaimObservation>,
        resume_request_id: Option<String>) -> Result<Self,String> {
        let domain_id=&claim.domain_id;
        let session_id=&claim.session_id;
        let seat_id=&seat.seat_id;
        let tier=evidence(seat::permission_tier(&seat))?;
        let pin = evidence(runtime::current_instance_pin(db, &claim.instance_id))?;
        if !supported_driver_version(&pin) {
            return Err("native session launch: unsupported pinned driver/version".into());
        }
        let suffix = crate::store::digest::sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            root.canonical_root().identity.opaque(), domain_id, session_id,
            seat.incarnation, claim.generation).as_bytes());
        let profile_name = format!("Gogoke37.Session.{}", &suffix[..40]);
        let profile = evidence(AppContainerProfile::ensure_for_cli(&profile_name,
            tier == PermissionTier::NetworkedWrite))?;
        let homes = launch_homes(db, root, &profile, &claim, &pin)?;
        let worktree = evidence(worktree::resolve_for_launch(db, root, worktree_id,
            repository_id, domain_id, seat_id, &seat.incarnation, seat.generation))?;
        // F's stored instance/tier/generation describe creation provenance.
        // Current execution authority comes from the E seat and H claim;
        // a legitimate idle instance rebind does not change the worktree.
        let program = evidence(instance::locate_pinned_program(&pin.driver_id, &pin.digest, &pin.version))?;
        let program_identity = evidence(AppContainerProfile::capture_program_identity(&program))?;
        // Runtime home writes are separate from workspace permission. No
        // public parent, other session, source tree or common Git dir is granted.
        evidence(profile.grant_bound_tree(&homes.instance.path, &homes.instance.identity, true))?;
        evidence(profile.grant_bound_tree(&homes.session.path, &homes.session.identity, true))?;
        let writable = matches!(tier, PermissionTier::IsolatedWrite | PermissionTier::NetworkedWrite);
        evidence(profile.grant_bound_tree(&worktree.path, &worktree.identity, writable))?;
        evidence(profile.grant_bound_program(&program, &program_identity))?;
        let code_mode = if pin.driver_id == "codex" {
            Some(super::codex_component::BoundCodexComponent::prepare(&program, &profile)?)
        } else { None };
        let roots = [
            (homes.instance.path.clone(), homes.instance.identity.clone()),
            (homes.session.path.clone(), homes.session.identity.clone()),
            (worktree.path.clone(), worktree.identity.clone()),
        ];
        let (module, directory_roots) = if pin.driver_id == "codex" {
            (Some(evidence(CompatModule::prepare_with_roots(root, &roots, &profile, &profile_name))?), None)
        } else {
            (None, Some(Arc::new(evidence(DirectoryRoots::prepare(root, &roots))?)))
        };
        let observed = Self { identity, seat, claim, pin, homes, repository_id: repository_id.into(),
            worktree, profile, profile_name, program, program_identity, code_mode, module, directory_roots, tier,
            resume_old,resume_request_id };
        observed.verify(db, root, owner, None)?;
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

    pub(crate) fn adopt_resume(&mut self, db: &VerifiedDatabaseConnection<'_>,
        owner: &OwnerIssuer, operation: &str) -> Result<(),String> {
        if self.resume_old.is_none() {return Err("native resume evidence already adopted".into());}
        self.seat=evidence(seat::get(db,&self.seat.domain_id,&self.seat.seat_id))?
            .ok_or("native resume seat disappeared")?;
        self.claim=evidence(runtime::observe_claim(db,&NativeOrigin::user(owner),
            &self.claim.domain_id,&self.seat.seat_id,&self.claim.session_id))?
            .ok_or("native resume claim disappeared")?;
        if self.claim.process_operation_id.as_deref()!=Some(operation) {
            return Err("native resume operation not current".into());
        }
        self.resume_old=None;
        self.resume_request_id=None;
        Ok(())
    }

    fn verify_snapshot(&self, db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock,
        owner: &OwnerIssuer, expected_operation: Option<&str>, revision: i64,
        identity: ProductIdentitySnapshot, phase: VerificationPhase) -> Result<(), String> {
        if identity != self.identity
            || evidence(seat::get(db, &self.seat.domain_id, &self.seat.seat_id))?.as_ref() != Some(&self.seat)
            || evidence(runtime::current_instance_pin(db, &self.claim.instance_id))? != self.pin {
            return Err("native session launch: current identity/seat/pin changed".into());
        }
        let current = evidence(runtime::observe_claim(db, &NativeOrigin::user(owner),
            &self.claim.domain_id, &self.seat.seat_id, &self.claim.session_id))?
            .ok_or("native session launch: claim no longer current")?;
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
        let worktree = evidence(worktree::resolve_for_launch(db, root, &self.worktree.worktree_id,
            &self.repository_id, &self.seat.domain_id, &self.seat.seat_id,
            &self.seat.incarnation, self.seat.generation))?;
        if worktree.identity != self.worktree.identity || worktree.pointer_hash != self.worktree.pointer_hash
            || worktree.pointer_identity != self.worktree.pointer_identity
            || worktree.common_identity != self.worktree.common_identity {
            return Err("native session launch: physical worktree changed".into());
        }
        let program = evidence(instance::locate_pinned_program(&self.pin.driver_id, &self.pin.digest, &self.pin.version))?;
        if program != self.program { return Err("native session launch: program path changed".into()); }
        for (path, identity, writable) in [
            (&self.homes.instance.path, &self.homes.instance.identity, true),
            (&self.homes.session.path, &self.homes.session.identity, true),
            (&self.worktree.path, &self.worktree.identity,
                matches!(self.tier, PermissionTier::IsolatedWrite | PermissionTier::NetworkedWrite)),
        ] {
            match phase {
                VerificationPhase::PreActivation =>
                    evidence(self.profile.verify_bound_tree_grant(path, identity, writable))?,
                VerificationPhase::Active =>
                    evidence(self.profile.verify_bound_directory_grant(path, identity, writable))?,
            };
        }
        evidence(self.profile.verify_bound_program_grant(&self.program, &self.program_identity))?;
        if let Some(code_mode) = &self.code_mode { code_mode.verify(&self.profile)?; }
        if let Some(module) = &self.module {
            let mut mapping = Vec::new();
            module.extend_environment(&mut mapping);
            evidence(module.validate_launch(Some(&self.profile_name), Some(&mapping)))
        } else {
            evidence(self.directory_roots.as_ref()
                .ok_or("native session launch: physical directory custody absent")?.verify())
        }
    }

    pub(crate) fn cwd(&self) -> &Path { &self.worktree.path }
    pub(crate) fn seat_id(&self) -> &str { &self.seat.seat_id }

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
        if spelling(observed)!=spelling(&self.worktree.path.to_string_lossy()) {
            return Err(format!("native thread cwd outside bound path spellings: expected={:?}; observed={observed:?}",self.worktree.path));
        }
        let returned=crate::root::inspect_root(path).map_err(|error|
            format!("native thread cwd observation: expected={:?}; observed={observed:?}; error={error:?}",self.worktree.path))?;
        if returned.identity != self.worktree.identity {
            return Err(format!("native thread physical cwd mismatch: expected={:?} identity={:?}; observed={observed:?} identity={:?}",
                self.worktree.path,self.worktree.identity,returned.identity));
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
        Ok((field("model")?, field("effort")?))
    }

    pub(crate) fn request(&self) -> Result<PrepareRequest, String> {
        let system_root = evidence(std::env::var("SystemRoot"))?;
        if !Path::new(&system_root).is_absolute() || system_root.contains('\0') {
            return Err("native session launch: invalid SystemRoot".into());
        }
        let mut environment = vec![("SystemRoot".into(), system_root.clone()), ("WINDIR".into(), system_root)];
        let runtime = self.homes.session.path.to_string_lossy().into_owned();
        let instance_home = self.homes.instance.path.to_string_lossy().into_owned();
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
                    evidence(commands::claude_launch_args(&model,&effort,None))?
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
        launch.current_directory = Some(self.worktree.path.clone());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.environment = Some(environment);
        launch.app_container_profile = Some(self.profile_name.clone());
        launch.app_container_internet_client = self.tier == PermissionTier::NetworkedWrite;
        launch.app_container_cli_identity_services = true;
        launch.path_compat = self.module.clone();
        launch.directory_roots = self.directory_roots.clone();
        launch.worktree_guard = Some(evidence(self.worktree.retained_pointer())?);
        Ok(PrepareRequest { launch, binding: NativeBinding {
            binary_digest_sha256: self.pin.digest.clone(), profile_id: self.claim.instance_id.clone(),
            domain_id: self.claim.domain_id.clone(), generation: self.claim.generation.clone(),
        } })
    }
}
