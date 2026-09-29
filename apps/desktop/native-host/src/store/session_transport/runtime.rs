//! H.1 native session transition guard. This module deliberately cannot mint a
//! caller, program pin, capacity, directory, or stop proof. The product entry
//! must supply those from the current native issuer and the same verified DB.
//! In particular, an uncertain external write is terminal until a trusted
//! reconciliation reads the original operation; a new request cannot retry it.

use crate::store::atomic::{AtomicError, Statement};
use crate::store::authority::{self, ProductIdentitySnapshot};
use crate::store::instance;
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, NativeOrigin, State as SeatState};
use super::admission::{self, AdmissionError, AdmissionRequest, AdmissionResult, TrustedLimits};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionPhase {
    Reserved,
    Committed,
    Prepared,
    Active,
    WriteUnknown,
    StopUnknown,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeError {
    InvalidTransition,
    StaleGeneration,
    UnknownExternalEffect,
    MissingDurableFact,
    PermissionNotEnforced,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PermissionTier {
    ReadOnly,
    NoNetwork,
    IsolatedWrite,
    NetworkedWrite,
}

/// A reservation is not a process permission. H's open path must call this
/// before prepare. F currently returns an opaque directory receipt but no
/// verified launch profile/path pair, and E has no durable tier binding. Even
/// NoNetwork cannot promise its full filesystem scope from those facts alone.
/// Refuse every runnable tier until the native ACL/profile/worktree witness is
/// available; no weaker tier is silently substituted.
pub(crate) fn require_launch_permission(_tier: PermissionTier)
    -> Result<(), RuntimeError> {
    Err(RuntimeError::PermissionNotEnforced)
}

/// Owner-origin admission rechecks the native issuer before and within H's
/// BEGIN IMMEDIATE transaction. Lead admission stays denied until E exposes
/// an H-facing verifier for the opaque native lead channel. The two required
/// caps are read from E and F on this same connection inside that transaction.
/// Neither the wire nor cached UI state can supply a capacity value.
pub(crate) fn reserve_native(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    seat_id: &str,
    request: &AdmissionRequest<'_>,
) -> Result<AdmissionResult, AdmissionError> {
    let NativeOrigin::User(owner) = origin else { return Err(AdmissionError::Denied); };
    let identity = authority::read_product_identity(db, owner)
        .map_err(AdmissionError::Identity)?;
    admission::reserve_admission(db, request, |db| {
        check_owner_current(db, &identity)?;
        let current = seat::get(db, request.domain_id, seat_id).map_err(AdmissionError::Seat)?
            .ok_or(AdmissionError::Denied)?;
        if current.instance_id != request.instance_id { return Err(AdmissionError::Denied); }
        let current = if current.state == SeatState::Idle
            && current.generation.checked_add(1).map(|value| value.to_string()).as_deref()
                == Some(request.generation) {
            seat::set_dispatch_state_in_transaction(db, &current, true)
                .map_err(AdmissionError::Seat)?
        } else { current };
        if current.state != SeatState::Busy || current.generation.to_string() != request.generation {
            return Err(AdmissionError::Denied);
        }
        admission::bind_seat_in_transaction(db, &current, request.session_id)?;
        current_instance_pin(db, request.instance_id)?;
        persisted_limits(db, request.domain_id, request.instance_id)
    })
}

fn persisted_limits(
    db: &VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    instance_id: &str,
) -> Result<TrustedLimits, AdmissionError> {
    let project_parallel = seat::read_project_parallel_cap(db, domain_id)
        .map_err(AdmissionError::ProjectCapacity)?;
    let instance_concurrency = instance::read_instance_concurrency_cap(db, instance_id)
        .map_err(AdmissionError::InstanceCapacity)?;
    Ok(TrustedLimits { project_parallel, instance_concurrency })
}

/// Commit consumes the exact reservation only while the native Owner issuer,
/// target seat, instance and pin remain current. No caller-supplied capacity
/// or grant is accepted by this entry.
pub(crate) fn commit_native(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    seat_id: &str,
    request: &AdmissionRequest<'_>,
) -> Result<AdmissionResult, AdmissionError> {
    let NativeOrigin::User(owner) = origin else { return Err(AdmissionError::Denied); };
    let identity = authority::read_product_identity(db, owner)
        .map_err(AdmissionError::Identity)?;
    admission::commit_admission(db, request, |db| {
        check_owner_and_seat(db, &identity, seat_id, request)?;
        current_instance_pin(db, request.instance_id).map(|_| ())
    })
}

pub(crate) fn release_native(
    db: &mut VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    request: &AdmissionRequest<'_>,
) -> Result<AdmissionResult, AdmissionError> {
    let NativeOrigin::User(owner) = origin else { return Err(AdmissionError::Denied); };
    let identity = authority::read_product_identity(db, owner)
        .map_err(AdmissionError::Identity)?;
    admission::release_admission(db, request, |db| check_owner_current(db, &identity))
}

/// This is the registered F pin, not a path or digest supplied by the request.
/// The native catalog must provide the executable path and ProcessCustodian
/// must compare the file and launched image against this digest again.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstancePin {
    pub(crate) driver_id: String,
    pub(crate) digest: String,
    pub(crate) version: String,
}

pub(crate) fn current_instance_pin(
    db: &VerifiedDatabaseConnection<'_>,
    instance_id: &str,
) -> Result<InstancePin, AdmissionError> {
    let row = Statement::prepare(db.as_ptr(),
        "SELECT driver_id,program_digest,version FROM main.gogoke_v37_instances \
         WHERE instance_id=?1 AND install_state='INSTALLED' AND login_state='LOGGED_IN'")?;
    row.bind_text(1, instance_id)?;
    if !row.step_row()? { return Err(AdmissionError::Denied); }
    let pin = InstancePin { driver_id: row.column_text(0)?,
        digest: row.column_text(1)?, version: row.column_text(2)? };
    if pin.driver_id.is_empty() || pin.digest.len() != 71
        || !pin.digest.starts_with("sha256:")
        || !pin.digest[7..].bytes().all(|b| b.is_ascii_hexdigit()) || pin.version.is_empty()
        || row.step_row()? { return Err(AdmissionError::Denied); }
    Ok(pin)
}

fn check_owner_and_seat(
    db: &mut VerifiedDatabaseConnection<'_>,
    identity: &ProductIdentitySnapshot,
    seat_id: &str,
    request: &AdmissionRequest<'_>,
) -> Result<(), AdmissionError> {
    check_owner_current(db, identity)?;
    let seat = seat::get(db, request.domain_id, seat_id).map_err(AdmissionError::Seat)?
        .ok_or(AdmissionError::Denied)?;
    if seat.state != SeatState::Busy || seat.instance_id != request.instance_id
        || seat.generation.to_string() != request.generation {
        return Err(AdmissionError::Denied);
    }
    let binding = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2 AND seat_id=?3 AND seat_incarnation=?4 AND generation=?5")?;
    for (index, value) in [request.domain_id, request.session_id, seat_id,
        seat.incarnation.as_str(), request.generation].iter().enumerate() {
        binding.bind_text((index + 1) as i32, value)?;
    }
    if !binding.step_row()? || binding.step_row()? { return Err(AdmissionError::Denied); }
    Ok(())
}

fn check_owner_current(
    db: &VerifiedDatabaseConnection<'_>,
    identity: &ProductIdentitySnapshot,
) -> Result<(), AdmissionError> {
    // The issuer was checked against the held root before the transaction.
    // Rechecking every mutable profile head inside this transaction prevents
    // an intervening policy/revocation change from authorizing a write.
    let profile = Statement::prepare(db.as_ptr(),
        "SELECT 1 FROM main.gogoke_authority_profile WHERE singleton=1 \
         AND profile_id=?1 AND root_identity=?2 AND owner_principal_id=?3 \
         AND owner_seat_id=?4 AND policy_revision=?5 AND revocation_head=?6")?;
    for (index, value) in [
        &identity.profile_id, &identity.root_identity, &identity.principal_id,
        &identity.seat_id, &identity.policy_revision, &identity.revocation_head,
    ].iter().enumerate() { profile.bind_text((index + 1) as i32, value)?; }
    if !profile.step_row()? || profile.step_row()? { return Err(AdmissionError::Denied); }
    Ok(())
}

/// Only an exact, current claim read from H's admission table. It is an
/// observation, not an authority token. E/F and native origin checks remain
/// mandatory at the entry that uses it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaimObservation {
    pub(crate) domain_id: String,
    pub(crate) session_id: String,
    pub(crate) instance_id: String,
    pub(crate) home_id: String,
    pub(crate) binding_id: String,
    pub(crate) generation: String,
    pub(crate) revision: i64,
    pub(crate) phase: SessionPhase,
    pub(crate) process_operation_id: Option<String>,
}

/// A stop fact bound by H to the same session and process operation. Merely
/// observing a process exit or receiving a CLI message cannot construct it.
pub(crate) struct StopFact {
    domain_id: String,
    session_id: String,
    generation: String,
    process_operation_id: String,
    proof_hash: String,
}

pub(crate) fn observe_stop_fact(
    db: &VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    session_id: &str,
) -> Result<Option<StopFact>, AtomicError> {
    let row = Statement::prepare(db.as_ptr(),
        "SELECT a.generation,a.process_operation_id,a.stop_fact_id \
         FROM main.gogoke_v37_h_claim AS a \
         JOIN main.gogoke_coordination_process_custody AS c \
           ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id \
          AND c.generation=a.generation AND c.state='STOPPED' \
          AND c.stop_proof_hash=a.stop_fact_id \
         WHERE a.domain_id=?1 AND a.session_id=?2 AND a.state='STOPPED' \
           AND a.stop_fact_id IS NOT NULL")?;
    row.bind_text(1, domain_id)?;
    row.bind_text(2, session_id)?;
    if !row.step_row()? { return Ok(None); }
    let fact = StopFact {
        domain_id: domain_id.to_owned(),
        session_id: session_id.to_owned(),
        generation: row.column_text(0)?,
        process_operation_id: row.column_text(1)?,
        proof_hash: row.column_text(2)?,
    };
    if fact.process_operation_id.is_empty() || fact.proof_hash.is_empty() || row.step_row()? {
        return Err(AtomicError::OperationConflict);
    }
    Ok(Some(fact))
}

/// Read through the product's existing verified connection. The join checks
/// the active owner binding, home and registered instance on that connection;
/// absence or ambiguity never becomes a launch permission.
pub(crate) fn observe_claim(
    db: &VerifiedDatabaseConnection<'_>,
    origin: &NativeOrigin<'_>,
    domain_id: &str,
    seat_id: &str,
    session_id: &str,
) -> Result<Option<ClaimObservation>, AtomicError> {
    // The User variant carries the native OwnerIssuer. The opaque Lead
    // admission presently has no H-facing method to revalidate its live
    // channel, seat incarnation and generation. Deny it until E supplies
    // that bridge rather than trusting a model-provided seat string.
    if !matches!(origin, NativeOrigin::User(_)) { return Ok(None); }
    let row = Statement::prepare(db.as_ptr(),
        "SELECT a.instance_id,a.home_id,a.binding_id,a.generation,a.revision,a.state,\
                COALESCE(a.process_operation_id,'') \
         FROM main.gogoke_v37_h_claim AS a \
         JOIN main.gogoke_v37_h_owner_binding AS b \
           ON b.binding_id=a.binding_id AND b.instance_id=a.instance_id \
          AND b.domain_id=a.domain_id AND b.owner_id=a.session_id \
          AND b.generation=a.generation AND b.kind='SESSION' AND b.state='ACTIVE' \
         JOIN main.gogoke_v37_instance_homes AS h \
           ON h.home_id=a.home_id AND h.instance_id=a.instance_id \
          AND h.domain_id=a.domain_id AND h.owner_id=a.session_id \
          AND h.generation=a.generation AND h.kind='SESSION' AND h.state='ACTIVE' \
         JOIN main.gogoke_v37_instances AS i ON i.instance_id=a.instance_id \
         JOIN main.gogoke_v37_seats AS s \
           ON s.domain_id=a.domain_id AND s.seat_id=?3 \
          AND s.instance_id=a.instance_id AND CAST(s.generation AS TEXT)=a.generation \
          AND s.state='BUSY' \
         JOIN main.gogoke_v37_h_seat_binding AS sb \
           ON sb.domain_id=a.domain_id AND sb.session_id=a.session_id \
          AND sb.seat_id=s.seat_id AND sb.seat_incarnation=s.incarnation \
          AND sb.generation=a.generation \
         WHERE a.domain_id=?1 AND a.session_id=?2")?;
    row.bind_text(1, domain_id)?;
    row.bind_text(2, session_id)?;
    row.bind_text(3, seat_id)?;
    if !row.step_row()? { return Ok(None); }
    let phase = match row.column_text(5)?.as_str() {
        "RESERVED" => SessionPhase::Reserved,
        "COMMITTED" => SessionPhase::Committed,
        "UNKNOWN" => SessionPhase::WriteUnknown,
        "STOPPED" => SessionPhase::Stopped,
        // RELEASED is not an active session, even if its home still exists.
        _ => return Ok(None),
    };
    let operation = row.column_text(6)?;
    let observation = ClaimObservation {
        domain_id: domain_id.to_owned(),
        session_id: session_id.to_owned(),
        instance_id: row.column_text(0)?,
        home_id: row.column_text(1)?,
        binding_id: row.column_text(2)?,
        generation: row.column_text(3)?,
        revision: row.column_text(4)?.parse().map_err(|_| AtomicError::InvalidRecord("claim revision"))?,
        phase,
        process_operation_id: if operation.is_empty() { None } else { Some(operation) },
    };
    if row.step_row()? { return Err(AtomicError::OperationConflict); }
    Ok(Some(observation))
}

/// This is an in-process sequencing guard. Each transition must follow its
/// durable database write and, for process transitions, the actual custodian
/// operation. It is never used to infer that a SQLite commit or an OS action
/// succeeded. On restart every PREPARED/ACTIVE observation is UNKNOWN until
/// the native custody record and Job are reconciled.
pub(crate) struct SessionTransitions {
    domain_id: String,
    session_id: String,
    generation: String,
    phase: SessionPhase,
    outstanding_write: Option<String>,
}

impl SessionTransitions {
    pub(crate) fn from_durable(claim: &ClaimObservation) -> Self {
        let phase = match claim.phase {
            SessionPhase::Reserved | SessionPhase::Stopped => claim.phase,
            // A committed claim may have an external start whose outcome was
            // lost; reconstruction does not replay it or infer a live Job.
            _ => SessionPhase::WriteUnknown,
        };
        Self { domain_id: claim.domain_id.clone(), session_id: claim.session_id.clone(),
            generation: claim.generation.clone(), phase, outstanding_write: None }
    }

    pub(crate) fn reserved(claim: &ClaimObservation)
        -> Result<Self, RuntimeError> {
        if claim.phase != SessionPhase::Reserved || claim.domain_id.is_empty()
            || claim.session_id.is_empty() || claim.generation.is_empty()
            || !claim.generation.bytes().all(|b| b.is_ascii_digit()) {
            return Err(RuntimeError::StaleGeneration);
        }
        Ok(Self { domain_id: claim.domain_id.clone(), session_id: claim.session_id.clone(),
            generation: claim.generation.clone(), phase: SessionPhase::Reserved,
            outstanding_write: None })
    }

    pub(crate) fn phase(&self) -> SessionPhase { self.phase }

    fn generation(&self, generation: &str) -> Result<(), RuntimeError> {
        if self.generation == generation { Ok(()) } else { Err(RuntimeError::StaleGeneration) }
    }

    pub(crate) fn committed(&mut self, generation: &str) -> Result<(), RuntimeError> {
        self.generation(generation)?;
        if self.phase != SessionPhase::Reserved { return Err(RuntimeError::InvalidTransition); }
        self.phase = SessionPhase::Committed;
        Ok(())
    }

    pub(crate) fn prepared(&mut self, generation: &str) -> Result<(), RuntimeError> {
        self.generation(generation)?;
        if self.phase != SessionPhase::Committed { return Err(RuntimeError::InvalidTransition); }
        self.phase = SessionPhase::Prepared;
        Ok(())
    }

    pub(crate) fn activated(&mut self, generation: &str) -> Result<(), RuntimeError> {
        self.generation(generation)?;
        if self.phase != SessionPhase::Prepared { return Err(RuntimeError::InvalidTransition); }
        self.phase = SessionPhase::Active;
        Ok(())
    }

    /// Called before writing the first byte. The caller must durably record
    /// this operation ID first. A partial/failed write is UNKNOWN, not a
    /// negative acknowledgment and not permission to send again.
    pub(crate) fn begin_write(&mut self, generation: &str, operation_id: &str)
        -> Result<(), RuntimeError> {
        self.generation(generation)?;
        if self.phase != SessionPhase::Active || self.outstanding_write.is_some()
            || operation_id.is_empty() || operation_id.len() > 128 {
            return Err(RuntimeError::InvalidTransition);
        }
        self.outstanding_write = Some(operation_id.to_owned());
        Ok(())
    }

    /// The adapter receipt binding is not yet exposed to H. A write can only
    /// settle as UNKNOWN here; this prevents a caller from turning a boolean
    /// or a model-produced message into a delivery proof.
    pub(crate) fn write_uncertain(&mut self, operation_id: &str)
        -> Result<(), RuntimeError> {
        if self.outstanding_write.as_deref() != Some(operation_id) {
            return Err(RuntimeError::InvalidTransition);
        }
        self.outstanding_write = None;
        self.phase = SessionPhase::WriteUnknown;
        Err(RuntimeError::UnknownExternalEffect)
    }

    pub(crate) fn stop_unknown(&mut self) {
        self.outstanding_write = None;
        self.phase = SessionPhase::StopUnknown;
    }

    /// Only after H has committed the native custodian STOPPED proof and
    /// record_session_stop_in_transaction has bound it to this exact claim.
    pub(crate) fn stopped(&mut self, fact: &StopFact) -> Result<(), RuntimeError> {
        self.generation(&fact.generation)?;
        if self.domain_id != fact.domain_id || self.session_id != fact.session_id
            || fact.process_operation_id.is_empty() || fact.proof_hash.is_empty() {
            return Err(RuntimeError::MissingDurableFact);
        }
        if !matches!(self.phase, SessionPhase::Active | SessionPhase::WriteUnknown | SessionPhase::StopUnknown) {
            return Err(RuntimeError::InvalidTransition);
        }
        self.outstanding_write = None;
        self.phase = SessionPhase::Stopped;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn admission_reads_both_required_owner_caps_on_the_same_connection() {
        use crate::root::RootLock;
        use crate::store::same_open::{create_new, open_existing, route_b_test_guard};
        use std::time::{SystemTime, UNIX_EPOCH};

        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!(
            "gogoke-h-persisted-cap-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        db.execute("PRAGMA foreign_keys=ON").unwrap();
        let owner = authority::initialize_profile(&mut db, &root).unwrap();
        instance::initialize_schema(&mut db).unwrap();
        seat::initialize_schema(&mut db).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,version,install_state,login_state,revision) VALUES('instanceA','codex','homeA','identityA','sha256:test','1','INSTALLED','LOGGED_IN',1)").unwrap();

        assert!(matches!(persisted_limits(&db, "projectA", "instanceA"),
            Err(AdmissionError::ProjectCapacity(seat::SeatError::Denied))));
        seat::set_project_parallel_cap(&mut db, &owner, "projectA", 4).unwrap();
        assert!(matches!(persisted_limits(&db, "projectA", "instanceA"),
            Err(AdmissionError::InstanceCapacity(
                crate::store::orchestration::OrchestrationError::AccessDenied))));
        instance::set_instance_concurrency_cap(&mut db, &owner, "instanceA", 4).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let limits = persisted_limits(&db, "projectA", "instanceA").unwrap();
        assert_eq!((limits.project_parallel, limits.instance_concurrency), (4, 4));
        db.execute("COMMIT").unwrap();
        db.close_checked().unwrap();

        let reopened = open_existing(&root, &path).unwrap();
        let limits = persisted_limits(&reopened, "projectA", "instanceA").unwrap();
        assert_eq!((limits.project_parallel, limits.instance_concurrency), (4, 4));
        reopened.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(path).unwrap();
        if let Err(error) = std::fs::remove_dir(&folder) {
            eprintln!("owned fixture retained: {} ({error})", folder.display());
        }
    }

    #[test]
    fn permission_tiers_do_not_silently_downgrade() {
        for tier in [PermissionTier::ReadOnly, PermissionTier::NoNetwork,
            PermissionTier::IsolatedWrite, PermissionTier::NetworkedWrite] {
            assert_eq!(require_launch_permission(tier), Err(RuntimeError::PermissionNotEnforced));
        }
    }

    #[cfg(windows)]
    #[test]
    fn pin_is_read_from_same_verified_store_and_requires_observed_login() {
        use crate::root::RootLock;
        use crate::store::same_open::{create_new, route_b_test_guard};
        use std::time::{SystemTime, UNIX_EPOCH};

        let _guard = route_b_test_guard();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-h-pin-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let root = RootLock::acquire(&folder).unwrap();
        let path = folder.join("state.sqlite");
        let mut db = create_new(&root, &path).unwrap();
        db.execute("CREATE TABLE gogoke_v37_instances(instance_id TEXT PRIMARY KEY,driver_id TEXT,program_digest TEXT,version TEXT,install_state TEXT,login_state TEXT) STRICT").unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        db.execute(&format!("INSERT INTO gogoke_v37_instances VALUES('instanceA','codex','{digest}','1.0','INSTALLED','UNKNOWN')")).unwrap();
        assert!(matches!(current_instance_pin(&db, "instanceA"), Err(AdmissionError::Denied)));
        db.execute("UPDATE gogoke_v37_instances SET login_state='LOGGED_IN' WHERE instance_id='instanceA'").unwrap();
        assert_eq!(current_instance_pin(&db, "instanceA").unwrap().digest, digest);
        db.execute("UPDATE gogoke_v37_instances SET program_digest='caller-value' WHERE instance_id='instanceA'").unwrap();
        assert!(matches!(current_instance_pin(&db, "instanceA"), Err(AdmissionError::Denied)));
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(folder).unwrap();
    }

    #[test]
    fn uncertain_write_never_replays_or_switches_generation() {
        let claim = ClaimObservation {
            domain_id: "project".into(), session_id: "session".into(),
            instance_id: "instance".into(), home_id: "home".into(),
            binding_id: "binding".into(), generation: "3".into(),
            revision: 1, phase: SessionPhase::Reserved, process_operation_id: None,
        };
        let mut run = SessionTransitions::reserved(&claim).unwrap();
        run.committed("3").unwrap();
        run.prepared("3").unwrap();
        run.activated("3").unwrap();
        run.begin_write("3", "sendA").unwrap();
        assert_eq!(run.write_uncertain("sendA"), Err(RuntimeError::UnknownExternalEffect));
        assert_eq!(run.begin_write("3", "sendA"), Err(RuntimeError::InvalidTransition));
        assert_eq!(run.begin_write("3", "sendB"), Err(RuntimeError::InvalidTransition));
        let fact = StopFact { domain_id: "project".into(), session_id: "session".into(),
            generation: "3".into(), process_operation_id: "process".into(),
            proof_hash: "proof".into() };
        let wrong_generation = StopFact { generation: "4".into(), ..fact };
        assert_eq!(run.stopped(&wrong_generation), Err(RuntimeError::StaleGeneration));
        let fact = StopFact { generation: "3".into(), ..wrong_generation };
        run.stopped(&fact).unwrap();
        assert_eq!(run.phase(), SessionPhase::Stopped);
    }

    #[test]
    fn reconstruction_does_not_treat_committed_claim_as_live_process() {
        let observation = ClaimObservation {
            domain_id: "project".into(), session_id: "session".into(),
            instance_id: "instance".into(), home_id: "home".into(),
            binding_id: "binding".into(), generation: "1".into(),
            revision: 2, phase: SessionPhase::Committed, process_operation_id: None,
        };
        let mut run = SessionTransitions::from_durable(&observation);
        assert_eq!(run.phase(), SessionPhase::WriteUnknown);
        assert_eq!(run.prepared("1"), Err(RuntimeError::InvalidTransition));
    }
}
