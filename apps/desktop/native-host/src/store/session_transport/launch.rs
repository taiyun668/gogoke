//! Native launch evidence for the product's existing E/F/H composition.
//! Logical IDs select stored facts; they never supply a path or permission.
use super::runtime::{self, ClaimObservation, InstancePin, SessionPhase};
use crate::process::{AppContainerProfile, CompatModule, NativeBinding, PrepareRequest, ProcessLaunch};
use crate::root::{RootIdentity, RootLock};
use crate::store::authority::{self, OwnerIssuer, ProductIdentitySnapshot};
use crate::store::instance::{self, InstanceLaunchHomes};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, NativeOrigin, PermissionTier, Seat, State};
use crate::store::worktree::{self, ResolvedBinding};
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
    module: Arc<CompatModule>,
    tier: PermissionTier,
}

impl LaunchEvidence {
    pub(crate) fn observe(
        db: &mut VerifiedDatabaseConnection<'_>, root: &RootLock, owner: &OwnerIssuer,
        domain_id: &str, seat_id: &str, session_id: &str,
        repository_id: &str, worktree_id: &str,
    ) -> Result<Self, String> {
        let identity = evidence(authority::read_product_identity(db, owner))?;
        let seat = evidence(seat::get(db, domain_id, seat_id))?
            .ok_or("native session launch: missing seat")?;
        if seat.state != State::Busy { return Err("native session launch: seat is not busy".into()); }
        let tier = evidence(seat::permission_tier(&seat))?;
        let claim = evidence(runtime::observe_claim(db, &NativeOrigin::user(owner),
            domain_id, seat_id, session_id))?.ok_or("native session launch: missing claim")?;
        if claim.phase != SessionPhase::Committed || claim.process_operation_id.is_some() {
            return Err("native session launch: claim is not an unused committed reservation".into());
        }
        let pin = evidence(runtime::current_instance_pin(db, &claim.instance_id))?;
        if pin.driver_id != "codex" || pin.version != "0.149.0" {
            return Err("native session launch: unsupported pinned driver/version".into());
        }
        let suffix = crate::store::digest::sha256_hex(format!("{}\n{}\n{}\n{}\n{}",
            root.canonical_root().identity.opaque(), domain_id, session_id,
            seat.incarnation, claim.generation).as_bytes());
        let profile_name = format!("Gogoke37.Session.{}", &suffix[..40]);
        let profile = evidence(AppContainerProfile::ensure(&profile_name,
            tier == PermissionTier::NetworkedWrite))?;
        let homes = evidence(instance::resolve_codex_session_launch_homes(db, root, &profile,
            &claim.instance_id, &claim.home_id, domain_id, session_id, &claim.generation))?;
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
        let module = evidence(CompatModule::prepare_with_roots(root, &[
            (homes.instance.path.clone(), homes.instance.identity.clone()),
            (homes.session.path.clone(), homes.session.identity.clone()),
            (worktree.path.clone(), worktree.identity.clone()),
        ], &profile, &profile_name))?;
        let observed = Self { identity, seat, claim, pin, homes, repository_id: repository_id.into(),
            worktree, profile, profile_name, program, program_identity, module, tier };
        observed.verify(db, root, owner, None)?;
        Ok(observed)
    }

    /// Recheck immediately before prepare and again before activate. A saved
    /// witness is not a grant; current Owner/E/F/H facts must still agree.
    pub(crate) fn verify(&self, db: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock, owner: &OwnerIssuer, expected_operation: Option<&str>) -> Result<(), String> {
        if evidence(authority::read_product_identity(db, owner))? != self.identity
            || evidence(seat::get(db, &self.seat.domain_id, &self.seat.seat_id))?.as_ref() != Some(&self.seat)
            || evidence(runtime::current_instance_pin(db, &self.claim.instance_id))? != self.pin {
            return Err("native session launch: current identity/seat/pin changed".into());
        }
        let claim = evidence(runtime::observe_claim(db, &NativeOrigin::user(owner),
            &self.claim.domain_id, &self.seat.seat_id, &self.claim.session_id))?
            .ok_or("native session launch: claim no longer current")?;
        // prepare persistence may attach a native custody operation; all
        // reservation facts and revisions remain fixed until activation.
        if claim.instance_id != self.claim.instance_id || claim.home_id != self.claim.home_id
            || claim.binding_id != self.claim.binding_id || claim.generation != self.claim.generation
            || claim.revision != self.claim.revision || claim.phase != SessionPhase::Committed
            || claim.process_operation_id.as_deref() != expected_operation {
            return Err("native session launch: current reservation changed".into());
        }
        let homes = evidence(instance::resolve_codex_session_launch_homes(db, root, &self.profile,
            &claim.instance_id, &claim.home_id, &claim.domain_id, &claim.session_id, &claim.generation))?;
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
        ] { evidence(self.profile.verify_bound_tree_grant(path, identity, writable))?; }
        evidence(self.profile.verify_bound_program_grant(&self.program, &self.program_identity))?;
        let mut mapping = Vec::new();
        self.module.extend_environment(&mut mapping);
        evidence(self.module.validate_launch(Some(&self.profile_name), Some(&mapping)))
    }

    pub(crate) fn cwd(&self) -> &Path { &self.worktree.path }

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
        for name in ["HOME", "USERPROFILE", "LOCALAPPDATA", "APPDATA", "TEMP", "TMP"] {
            environment.push((name.into(), runtime.clone()));
        }
        environment.push(("CODEX_HOME".into(), self.homes.instance.path.to_string_lossy().into_owned()));
        self.module.extend_environment(&mut environment);
        let mut launch = ProcessLaunch::new(self.program.clone());
        launch.arguments = vec!["-c".into(), "features.memories=false".into(),
            "-c".into(), "memories.generate_memories=false".into(),
            "-c".into(), "memories.use_memories=false".into(),
            "-c".into(), format!("sqlite_home={}", crate::store::atomic::Json::String(
                crate::store::atomic::JsonString::from_str(&runtime)).canonical()),
            "-c".into(), format!("log_dir={}", crate::store::atomic::Json::String(
                crate::store::atomic::JsonString::from_str(&runtime)).canonical()),
            "app-server".into()];
        launch.current_directory = Some(self.worktree.path.clone());
        launch.protocol_stdio = true;
        launch.persistent_protocol_stdio = true;
        launch.environment = Some(environment);
        launch.app_container_profile = Some(self.profile_name.clone());
        launch.app_container_internet_client = self.tier == PermissionTier::NetworkedWrite;
        launch.path_compat = Some(self.module.clone());
        launch.worktree_guard = Some(evidence(self.worktree.retained_pointer())?);
        Ok(PrepareRequest { launch, binding: NativeBinding {
            binary_digest_sha256: self.pin.digest.clone(), profile_id: self.claim.instance_id.clone(),
            domain_id: self.claim.domain_id.clone(), generation: self.claim.generation.clone(),
        } })
    }
}
