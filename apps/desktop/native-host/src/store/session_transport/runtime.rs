//! H.1 native session transition guard. This module deliberately cannot mint a
//! caller, program pin, capacity, directory, or stop proof. The product entry
//! must supply those from the current native issuer and the same verified DB.
//! In particular, an uncertain external write is terminal until a trusted
//! reconciliation reads the original operation; a new request cannot retry it.

use crate::store::atomic::{AtomicError, Statement};
use crate::store::authority::{self, OwnerIssuer, ProductIdentitySnapshot};
use crate::store::instance;
use crate::store::inbox::host_rule::{self as host_rule, HostCleanupCandidate, HostRecipient};
use crate::store::same_open::VerifiedDatabaseConnection;
use crate::store::seat::{self, Layer as SeatLayer, NativeOrigin, State as SeatState};
use crate::store::seat::HostEscalationProof;
use super::admission::{self, AdmissionError, AdmissionRequest, AdmissionResult, TrustedLimits};

fn host_recipient_admission_error(error: crate::store::inbox::InboxError) -> AdmissionError {
    use crate::store::inbox::InboxError;
    match error {
        InboxError::Invalid(name) => AdmissionError::Invalid(name),
        InboxError::Denied => AdmissionError::Denied,
        InboxError::Conflict => AdmissionError::Conflict,
        InboxError::Stale => AdmissionError::Stale,
        InboxError::Unknown => AdmissionError::Unknown,
        InboxError::Sqlite(error) => AdmissionError::Store(error),
        InboxError::Open(error) => AdmissionError::Sqlite(error),
        InboxError::CommitUnknown(error) => AdmissionError::CommitUnknown(error),
        InboxError::RollbackUnknown(error) => AdmissionError::RollbackUnknown(error),
        InboxError::Authority(error) => AdmissionError::Identity(error),
        original => AdmissionError::Identity(crate::store::orchestration::OrchestrationError::V37StoreFailure(
            format!("host recipient admission: {original:?}"))),
    }
}

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

pub(crate) use crate::store::seat::PermissionTier;

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

/// The internal HostRule path is a separate native authority. It consumes C's
/// frozen recipient and current E cause inside H's own admission transaction;
/// no User-origin wire call or synthetic model caller is involved.
pub(crate) fn reserve_native_for_host(db:&mut VerifiedDatabaseConnection<'_>,
    host:&OwnerIssuer,proof:&HostEscalationProof,choice:&HostRecipient,
    request:&AdmissionRequest<'_>)->Result<AdmissionResult,AdmissionError> {
    if choice.mode!="FRESH" || request.domain_id!=proof.domain_id()
        || request.session_id!=choice.session_id || request.instance_id!=choice.instance_id
        || request.generation!=choice.generation || request.request_id!=choice.reserve_request_id {
        return Err(AdmissionError::Denied);
    }
    let identity=authority::read_product_identity(db,host).map_err(AdmissionError::Identity)?;
    admission::reserve_admission(db,request,|db| {
        check_owner_current(db,&identity)?;
        let seat=host_rule::revalidate_host_recipient_in_transaction(db,host,proof,choice)
            .map_err(host_recipient_admission_error)?;
        let seat=if seat.state==SeatState::Idle {
            seat::set_dispatch_state_in_transaction(db,&seat,true).map_err(AdmissionError::Seat)?
        } else {seat};
        if seat.state!=SeatState::Busy || seat.generation.to_string()!=request.generation {
            return Err(AdmissionError::Denied);
        }
        admission::bind_seat_in_transaction(db,&seat,request.session_id)?;
        current_instance_pin(db,request.instance_id)?;
        persisted_limits(db,request.domain_id,request.instance_id)
    })
}

pub(crate) fn commit_native_for_host(db:&mut VerifiedDatabaseConnection<'_>,
    host:&OwnerIssuer,proof:&HostEscalationProof,choice:&HostRecipient,
    request:&AdmissionRequest<'_>)->Result<AdmissionResult,AdmissionError> {
    if choice.mode!="FRESH" || request.domain_id!=proof.domain_id()
        || request.session_id!=choice.session_id || request.instance_id!=choice.instance_id
        || request.generation!=choice.generation || request.request_id!=choice.commit_request_id {
        return Err(AdmissionError::Denied);
    }
    let identity=authority::read_product_identity(db,host).map_err(AdmissionError::Identity)?;
    admission::commit_admission(db,request,|db| {
        check_owner_current(db,&identity)?;
        let seat=host_rule::revalidate_host_recipient_in_transaction(db,host,proof,choice)
            .map_err(host_recipient_admission_error)?;
        if seat.state!=SeatState::Busy || seat.generation.to_string()!=request.generation {
            return Err(AdmissionError::Denied);
        }
        current_instance_pin(db,request.instance_id).map(|_|())
    })
}

fn exact_lead_reservation(db:&VerifiedDatabaseConnection<'_>,
    seat:&seat::Seat,request:&AdmissionRequest<'_>,
    replay_request:Option<&str>) -> Result<(),AdmissionError> {
    if seat.state!=SeatState::Busy || seat.instance_id!=request.instance_id
        || seat.generation.to_string()!=request.generation
        || seat.domain_id!=request.domain_id {return Err(AdmissionError::Denied);}
    let q=Statement::prepare(db.as_ptr(),
        "SELECT o.request_id FROM main.gogoke_v37_h_operation o
           JOIN main.gogoke_v37_h_claim a ON a.domain_id=o.domain_id
             AND a.session_id=o.session_id AND a.instance_id=?3
             AND a.home_id=?4 AND a.generation=?5
             AND a.state IN ('RESERVED','COMMITTED')
           JOIN main.gogoke_v37_h_seat_binding sb ON sb.domain_id=a.domain_id
             AND sb.session_id=a.session_id AND sb.generation=a.generation
             AND sb.seat_id=?6 AND sb.seat_incarnation=?7
          WHERE o.domain_id=?1 AND o.session_id=?2
            AND o.operation='admission-reserve' AND o.status='APPLIED'")?;
    for (index,value) in [request.domain_id,request.session_id,request.instance_id,
        request.home_id,request.generation,seat.seat_id.as_str(),
        seat.incarnation.as_str()].iter().enumerate() {
        q.bind_text((index+1) as i32,value)?;
    }
    if !q.step_row()? {return Err(AdmissionError::Denied);}
    let original=q.column_text(0)?;
    if q.step_row()? || replay_request.is_some_and(|wanted|wanted!=original) {
        return Err(AdmissionError::Denied);
    }
    Ok(())
}

fn lead_child_for_admission(db:&mut VerifiedDatabaseConnection<'_>,
    admission:&seat::NativeLeadAdmission,seat_id:&str,
    request:&AdmissionRequest<'_>,first_reserve:bool)->Result<seat::Seat,AdmissionError> {
    let caller=admission.model_call().ok_or(AdmissionError::Denied)?;
    let parent=super::model_call::revalidate_model_call_in_transaction(db,caller)
        .map_err(|_|AdmissionError::Denied)?;
    let (domain,parent_id,generation)=admission.parent_identity();
    if parent.layer!=SeatLayer::User || parent.domain_id!=domain
        || parent.seat_id!=parent_id || parent.incarnation!=admission.parent_incarnation()
        || parent.generation!=generation || domain!=request.domain_id {
        return Err(AdmissionError::Denied);
    }
    let child=seat::get(db,request.domain_id,seat_id).map_err(AdmissionError::Seat)?
        .ok_or(AdmissionError::Denied)?;
    if child.seat_id!=seat_id || child.instance_id!=request.instance_id
        || child.layer!=SeatLayer::Lead
        || child.parent_seat_id.as_deref()!=Some(parent_id) {
        return Err(AdmissionError::Denied);
    }
    if child.state==SeatState::Idle && first_reserve {
        if caller.host_request_id()!=Some(request.request_id) {
            return Err(AdmissionError::Denied);
        }
        seat::authorize_child_dispatch(db,caller,&child).map_err(AdmissionError::Seat)?;
        if child.generation.checked_add(1).map(|next|next.to_string()).as_deref()
            !=Some(request.generation) {return Err(AdmissionError::Denied);}
        seat::set_dispatch_state_in_transaction(db,&child,true)
            .map_err(AdmissionError::Seat)
    } else {
        seat::current_child_dispatch_context(db,caller,&child)
            .map_err(AdmissionError::Seat)?;
        exact_lead_reservation(db,&child,request,
            Some(caller.host_request_id().ok_or(AdmissionError::Denied)?))?;
        Ok(child)
    }
}

/// Root's native Owner issuer authenticates the DB/root. A Lead admission
/// separately carries the exact H/A model-call proof and current E grant.
pub(crate) fn reserve_native_with_origin(db:&mut VerifiedDatabaseConnection<'_>,
    host:&OwnerIssuer,origin:&NativeOrigin<'_>,seat_id:&str,
    request:&AdmissionRequest<'_>)->Result<AdmissionResult,AdmissionError> {
    if matches!(origin,NativeOrigin::User(_)) {return reserve_native(db,origin,seat_id,request);}
    let NativeOrigin::Lead(admission)=origin else {return Err(AdmissionError::Denied)};
    let identity=authority::read_product_identity(db,host).map_err(AdmissionError::Identity)?;
    admission::reserve_admission(db,request,|db| {
        check_owner_current(db,&identity)?;
        let child=lead_child_for_admission(db,admission,seat_id,request,true)?;
        admission::bind_seat_in_transaction(db,&child,request.session_id)?;
        current_instance_pin(db,request.instance_id)?;
        persisted_limits(db,request.domain_id,request.instance_id)
    })
}

pub(crate) fn commit_native_with_origin(db:&mut VerifiedDatabaseConnection<'_>,
    host:&OwnerIssuer,origin:&NativeOrigin<'_>,seat_id:&str,
    request:&AdmissionRequest<'_>)->Result<AdmissionResult,AdmissionError> {
    if matches!(origin,NativeOrigin::User(_)) {return commit_native(db,origin,seat_id,request);}
    let NativeOrigin::Lead(admission)=origin else {return Err(AdmissionError::Denied)};
    let identity=authority::read_product_identity(db,host).map_err(AdmissionError::Identity)?;
    admission::commit_admission(db,request,|db| {
        check_owner_current(db,&identity)?;
        lead_child_for_admission(db,admission,seat_id,request,false)?;
        current_instance_pin(db,request.instance_id).map(|_|())
    })
}

fn persisted_limits(
    db: &VerifiedDatabaseConnection<'_>,
    domain_id: &str,
    instance_id: &str,
) -> Result<TrustedLimits, AdmissionError> {
    seat::refresh_host_parallel_fact_in_transaction(db)
        .map_err(AdmissionError::ProjectCapacity)?;
    let (project_parallel, _) = seat::read_effective_project_parallel_cap(db, domain_id)
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
    admission::release_unstarted_owner_commit(db, request, |db| check_owner_current(db, &identity))
}

/// Private cleanup of C's frozen failed Host FRESH recipe. The caller has
/// checked that exact C operation; H independently refuses any open intent.
pub(crate) fn release_failed_host_native(
    db:&mut VerifiedDatabaseConnection<'_>,owner:&OwnerIssuer,
    candidate:&HostCleanupCandidate,request:&AdmissionRequest<'_>,unstarted:bool,
)->Result<AdmissionResult,AdmissionError> {
    if request.domain_id!=candidate.domain_id ||request.session_id!=candidate.choice.session_id
        ||request.instance_id!=candidate.choice.instance_id
        ||request.generation!=candidate.choice.generation
        ||request.request_id!=format!("{}-release",
            candidate.message_id.replacen("hostmsg-","hostcleanup-",1)) {
        return Err(AdmissionError::Denied);
    }
    let identity=authority::read_product_identity(db,owner).map_err(AdmissionError::Identity)?;
    let authorize=|db:&mut VerifiedDatabaseConnection<'_>| {
        check_owner_current(db,&identity)?;
        host_rule::verify_frozen_host_cleanup_in_transaction(db,candidate,
            "release",request.raw_bytes)
            .map_err(host_recipient_admission_error)
    };
    if unstarted {
        admission::release_unstarted_host_commit(db,request,authorize)
    } else {
        admission::release_admission(db,request,authorize)
    }
}

/// Later child control has a new sealed model call, not the old reserve call.
/// Release only its exact stopped child; replay reads the same released claim
/// and original journal without requiring the child to remain BUSY.
pub(crate) fn release_native_with_origin(db:&mut VerifiedDatabaseConnection<'_>,
    host:&OwnerIssuer,origin:&NativeOrigin<'_>,seat_id:&str,
    request:&AdmissionRequest<'_>)->Result<AdmissionResult,AdmissionError> {
    if matches!(origin,NativeOrigin::User(_)) {return release_native(db,origin,request);}
    let NativeOrigin::Lead(admission)=origin else {return Err(AdmissionError::Denied)};
    let caller=admission.model_call().ok_or(AdmissionError::Denied)?;
    let host_id=caller.host_request_id().ok_or(AdmissionError::Denied)?;
    if request.request_id!=format!("{host_id}-release") {return Err(AdmissionError::Denied);}
    let identity=authority::read_product_identity(db,host).map_err(AdmissionError::Identity)?;
    admission::release_admission(db,request,|db| {
        check_owner_current(db,&identity)?;
        let child=seat::get(db,request.domain_id,seat_id).map_err(AdmissionError::Seat)?
            .ok_or(AdmissionError::Denied)?;
        seat::current_child_dispatch_context(db,caller,&child).map_err(AdmissionError::Seat)?;
        if child.instance_id!=request.instance_id {
            return Err(AdmissionError::Denied);
        }
        let fact=Statement::prepare(db.as_ptr(),
            "SELECT a.state,a.revision FROM main.gogoke_v37_h_claim a
               JOIN main.gogoke_v37_h_seat_binding s ON s.domain_id=a.domain_id
                 AND s.session_id=a.session_id AND s.generation=a.generation
               JOIN main.gogoke_coordination_process_custody c
                 ON c.operation_id=a.process_operation_id AND c.domain_id=a.domain_id
                 AND c.generation=a.generation AND c.state='STOPPED'
                 AND c.stop_proof_hash=a.stop_fact_id
              WHERE a.domain_id=?1 AND a.session_id=?2 AND s.seat_id=?3
                AND s.seat_incarnation=?4 AND a.generation=?5
                AND a.instance_id=?6 AND a.home_id=?7 AND a.stop_fact_id IS NOT NULL")?;
        for (index,value) in [request.domain_id,request.session_id,seat_id,child.incarnation.as_str(),
            request.generation,request.instance_id,request.home_id].iter().enumerate() {
            fact.bind_text((index+1) as i32,value)?;
        }
        if !fact.step_row()? {return Err(AdmissionError::Denied);}
        let state=fact.column_text(0)?;
        let revision=fact.column_text(1)?.parse::<i64>().map_err(|_|AdmissionError::Denied)?;
        if fact.step_row()? {return Err(AdmissionError::Denied);}
        drop(fact);
        if state=="STOPPED" && child.state==SeatState::Busy && revision==request.expected_revision
            && child.generation.to_string()==request.generation {
            return Ok(());
        }
        if state!="RELEASED" || child.state!=SeatState::Idle
            || request.expected_revision.checked_add(1)!=Some(revision)
            || request.generation.parse::<i64>().ok().and_then(|generation|generation.checked_add(1))
                !=Some(child.generation) {
            return Err(AdmissionError::Denied);
        }
        let prior=Statement::prepare(db.as_ptr(),
            "SELECT raw_hex FROM main.gogoke_v37_h_operation WHERE domain_id=?1
               AND session_id=?2 AND request_id=?3 AND operation='admission-release'
               AND status='APPLIED' AND previous_revision=?4 AND revision=?5")?;
        prior.bind_text(1,request.domain_id)?;prior.bind_text(2,request.session_id)?;
        prior.bind_text(3,request.request_id)?;prior.bind_i64(4,request.expected_revision)?;
        prior.bind_i64(5,revision)?;
        let raw:String=request.raw_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
        if !prior.step_row()? || prior.column_text(0)?!=raw || prior.step_row()? {
            return Err(AdmissionError::Denied);
        }
        Ok(())
    })
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
         WHERE instance_id=?1 AND login_state='LOGGED_IN'")?;
    row.bind_text(1, instance_id)?;
    if !row.step_row()? { return Err(AdmissionError::Denied); }
    let pin = InstancePin { driver_id: row.column_text(0)?,
        digest: row.column_text(1)?, version: row.column_text(2)? };
    if pin.driver_id.is_empty() || pin.digest.len() != 71
        || !pin.digest.starts_with("sha256:")
        || !pin.digest[7..].bytes().all(|b| b.is_ascii_hexdigit()) || pin.version.is_empty()
        || row.step_row()? { return Err(AdmissionError::Denied); }
    drop(row);
    // F's install-state read uses this same catalog observation. Recheck the
    // registered CLI bytes and version at every H admission; a persisted
    // install_state is not maintained by that read.
    instance::locate_pinned_program(&pin.driver_id, &pin.digest, &pin.version)
        .map_err(AdmissionError::Catalog)?;
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

impl StopFact {
    pub(crate) fn generation(&self)->&str {&self.generation}
    pub(crate) fn process_operation_id(&self)->&str {&self.process_operation_id}
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
    if let NativeOrigin::Lead(admission)=origin {
        let caller=admission.model_call().ok_or(AtomicError::OperationConflict)?;
        let child=seat::get(db,domain_id,seat_id)
            .map_err(|_|AtomicError::OperationConflict)?
            .ok_or(AtomicError::OperationConflict)?;
        seat::current_child_dispatch_context(db,caller,&child)
            .map_err(|_|AtomicError::OperationConflict)?;
        let claim=observe_claim_bound(db,domain_id,seat_id,session_id)?
            .ok_or(AtomicError::OperationConflict)?;
        let request=AdmissionRequest {domain_id,session_id,
            request_id:caller.host_request_id().ok_or(AtomicError::OperationConflict)?,
            raw_bytes:&[],instance_id:&claim.instance_id,home_id:&claim.home_id,
            generation:&claim.generation,expected_revision:0};
        exact_lead_reservation(db,&child,&request,Some(request.request_id))
            .map_err(|_|AtomicError::OperationConflict)?;
    }
    observe_claim_bound(db,domain_id,seat_id,session_id)
}

/// Pure H bound-fact read for an already admitted child. It grants no new
/// admission or launch permission and does not retain the parent turn.
pub(crate) fn observe_claim_bound(
    db:&VerifiedDatabaseConnection<'_>,domain_id:&str,seat_id:&str,session_id:&str,
) -> Result<Option<ClaimObservation>,AtomicError> {
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
    fn admission_refreshes_os_cap_with_owner_and_instance_caps_in_one_transaction() {
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

        db.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(persisted_limits(&db, "projectA", "instanceA"),
            Err(AdmissionError::ProjectCapacity(seat::SeatError::Denied))));
        db.execute("ROLLBACK").unwrap();
        assert!(matches!(seat::read_host_parallel_fact(&db), Err(seat::SeatError::Denied)));
        seat::set_project_parallel_cap(&mut db, &owner, "projectA", 4).unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        assert!(matches!(persisted_limits(&db, "projectA", "instanceA"),
            Err(AdmissionError::InstanceCapacity(
                crate::store::orchestration::OrchestrationError::AccessDenied))));
        db.execute("ROLLBACK").unwrap();
        instance::set_instance_concurrency_cap(&mut db, &owner, "instanceA", 4).unwrap();
        db.execute("INSERT INTO main.gogoke_v37_seat_host_resources(singleton,source,observed_parallelism,machine_limit,revision) VALUES(1,'STD_AVAILABLE_PARALLELISM',9223372036854775807,9223372036854775807,1)").unwrap();
        db.execute("BEGIN IMMEDIATE").unwrap();
        let limits = persisted_limits(&db, "projectA", "instanceA").unwrap();
        let fact = seat::read_host_parallel_fact(&db).unwrap();
        assert_eq!(fact.observed_parallelism,
            i64::try_from(std::thread::available_parallelism().unwrap().get()).unwrap());
        assert_eq!(fact.machine_limit, fact.observed_parallelism);
        assert_eq!((limits.project_parallel, limits.instance_concurrency),
            (4_i64.min(fact.machine_limit), 4));
        assert_eq!(seat::read_project_parallel_cap(&db, "projectA").unwrap(), 4);
        db.execute("COMMIT").unwrap();
        db.close_checked().unwrap();

        let mut reopened = open_existing(&root, &path).unwrap();
        reopened.execute("BEGIN IMMEDIATE").unwrap();
        let limits = persisted_limits(&reopened, "projectA", "instanceA").unwrap();
        let fact = seat::read_host_parallel_fact(&reopened).unwrap();
        assert_eq!((limits.project_parallel, limits.instance_concurrency),
            (4_i64.min(fact.machine_limit), 4));
        assert_eq!(seat::read_project_parallel_cap(&reopened, "projectA").unwrap(), 4);
        reopened.execute("COMMIT").unwrap();
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
        instance::initialize_schema(&mut db).unwrap();
        let catalog = instance::discover_program("codex").expect("actual fixed CLI catalog");
        instance::register_instance(&mut db, &root, &instance::Registration {
            request_id: "registerA", request_bytes: b"current H pin fixture",
            instance_id: "instanceA", driver_id: "codex", program: &catalog,
        }).unwrap();
        let row = Statement::prepare(db.as_ptr(),
            "SELECT program_digest,version,install_state FROM gogoke_v37_instances WHERE instance_id='instanceA'").unwrap();
        assert!(row.step_row().unwrap());
        let digest = row.column_text(0).unwrap();
        let version = row.column_text(1).unwrap();
        assert_eq!(row.column_text(2).unwrap(), "UNKNOWN");
        drop(row);
        assert!(matches!(current_instance_pin(&db, "instanceA"), Err(AdmissionError::Denied)));
        db.execute("UPDATE gogoke_v37_instances SET login_state='LOGGED_IN' WHERE instance_id='instanceA'").unwrap();
        let current = current_instance_pin(&db, "instanceA").unwrap();
        assert_eq!((current.digest, current.version), (digest, version));
        db.execute(&format!("UPDATE gogoke_v37_instances SET program_digest='sha256:{}' WHERE instance_id='instanceA'", "0".repeat(64))).unwrap();
        assert!(matches!(current_instance_pin(&db, "instanceA"),
            Err(AdmissionError::Catalog(instance::CatalogError::IdentityChanged))));
        db.execute("UPDATE gogoke_v37_instances SET driver_id='missing-provider' WHERE instance_id='instanceA'").unwrap();
        assert!(matches!(current_instance_pin(&db, "instanceA"),
            Err(AdmissionError::Catalog(instance::CatalogError::UnknownDriver))));
        db.close_checked().unwrap();
        drop(root);
        std::fs::remove_dir_all(folder).unwrap();
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
