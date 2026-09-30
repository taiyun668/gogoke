//! Owner-origin H admission uses the product's one E/F connection. Paths and
//! capacities are resolved natively; neither is accepted in operation payloads.
use super::*;
use crate::process::AppContainerProfile;
use crate::store::seat::{self, NativeOrigin, State};
use crate::store::session_transport::{self as h, runtime, AdmissionError,
    AdmissionRequest, AdmissionResult, OwnerBinding};

fn text(value: &str) -> Json { Json::String(JsonString::from_str(value)) }

fn admission_status(error: &AdmissionError) -> V37Status {
    match error {
        AdmissionError::Invalid(_) | AdmissionError::Denied
        | AdmissionError::ProjectCapacity(seat::SeatError::Denied)
        | AdmissionError::InstanceCapacity(OrchestrationError::AccessDenied) => V37Status::Denied,
        AdmissionError::Conflict => V37Status::Conflict,
        AdmissionError::Stale => V37Status::Stale,
        AdmissionError::UnsupportedCapacity => V37Status::Unsupported,
        _ => V37Status::Unknown,
    }
}

impl<'root> ProductDatabase<'root> {
    /// K-WORKTREE creation is User-only at the parent ingress. Source and Git
    /// paths live solely in the separate Owner configuration plane; this
    /// closed operation accepts logical repository/seat IDs only.
    pub(super) fn dispatch_user_worktree(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        use crate::store::worktree::{self as f, WorktreeError};
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if request.operation != "create" {
            return Ok(encode_receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if request.expected_revision != 0 || request.payload.len() != 2 {
            return Ok(encode_receipt(request, V37Status::Denied, 0, 0, Default::default()));
        }
        let repository = user_payload_string(request, "repositoryId")?;
        let seat_id = user_payload_string(request, "seatId")?;
        let readback = f::readback_create(&self.connection, self.root,
            &request.request_id, &request.raw_bytes);
        let mut replayed = false;
        let result = match readback {
            Ok(Some(binding)) => { replayed = true; Ok(binding) }
            Ok(None) => (|| {
                let pin = f::resolve_registered_git(&mut self.connection, self.root,
                    &self.owner, &repository, &mut self.process_custodian)?;
                f::create_worktree(&mut self.connection, self.root, &self.owner, &pin,
                    &mut self.process_custodian, f::CreateWorktree {
                        request_id: &request.request_id, request_bytes: &request.raw_bytes,
                        target_id: &request.target_id, repository_id: &repository,
                        domain_id: &request.domain_id, seat_id: &seat_id,
                    })
            })(),
            Err(error) => Err(error),
        };
        match result {
            Ok(binding) => Ok(encode_receipt(request,
                if replayed { V37Status::Replayed } else { V37Status::Applied }, 0, 1,
                BTreeMap::from([
                    (JsonString::from_str("worktreeId"), text(&binding.worktree_id)),
                    (JsonString::from_str("repositoryId"), text(&repository)),
                    (JsonString::from_str("seatId"), text(&seat_id)),
                    (JsonString::from_str("classification"), text("SINGLE")),
                    (JsonString::from_str("baselineCommit"), text(&binding.baseline_commit)),
                ]))),
            Err(error) => {
                let status = match &error {
                    WorktreeError::Denied | WorktreeError::Invalid(_) => V37Status::Denied,
                    WorktreeError::Conflict => V37Status::Conflict,
                    _ => V37Status::Unknown,
                };
                Ok(encode_receipt(request, status, 0, 0, BTreeMap::from([
                    (JsonString::from_str("reason"), text(&format!("native worktree: {error:?}")))])))
            }
        }
    }

    pub(super) fn read_user_instance_capacity(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let revision = self.user_instance_revision(&request.target_id)?;
        if request.domain_id != "global" || !request.payload.is_empty() {
            return Ok(encode_receipt(request, V37Status::Denied, revision, revision, Default::default()));
        }
        if request.expected_revision != revision {
            return Ok(encode_receipt(request, V37Status::Stale, revision, revision, Default::default()));
        }
        let (status, result) = match instance::read_instance_concurrency_cap(&self.connection, &request.target_id) {
            Ok(cap) => (V37Status::Applied, BTreeMap::from([
                (JsonString::from_str("capacity"), text(&cap.to_string()))])),
            Err(OrchestrationError::AccessDenied) => (V37Status::Denied, BTreeMap::new()),
            Err(error) => (V37Status::Unknown, BTreeMap::from([
                (JsonString::from_str("reason"), text(&format!("instance capacity: {error:?}")))])),
        };
        Ok(encode_receipt(request, status, revision, revision, result))
    }

    /// Prepare only the named native seat's home. F's external directory
    /// operation has its own original-request journal; it is never described
    /// as atomic with the subsequent admission or OS process creation.
    fn prepare_user_session_home(&mut self, request: &V37Request, seat_id: &str,
        generation: &str) -> Result<(String, String)> {
        self.prepare_session_home(request, seat_id, generation, false)
    }

    /// A stopped session keeps its admission. A resume candidate prepares a
    /// separate F home for the next process generation while E remains BUSY.
    pub(super) fn prepare_resume_session_home(&mut self, request: &V37Request,
        seat_id: &str, generation: &str) -> Result<(String, String)> {
        self.prepare_session_home(request, seat_id, generation, true)
    }

    fn prepare_session_home(&mut self, request: &V37Request, seat_id: &str,
        generation: &str, resume: bool) -> Result<(String, String)> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        let seat = seat::get(&self.connection, &request.domain_id, seat_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if seat.instance_id.is_empty() || !matches!(seat.state, State::Idle | State::Busy) {
            return Err(OrchestrationError::AccessDenied);
        }
        let expected = if seat.state == State::Idle || resume {
            seat.generation.checked_add(1).ok_or(OrchestrationError::OperationConflict)?
        } else { seat.generation };
        if expected.to_string() != generation { return Err(OrchestrationError::OperationConflict); }
        // Refuse absent configuration before any filesystem work. H reads both
        // again under BEGIN IMMEDIATE when it decides actual capacity.
        seat::read_project_parallel_cap(&self.connection, &request.domain_id)?;
        instance::read_instance_concurrency_cap(&self.connection, &seat.instance_id)?;
        runtime::current_instance_pin(&self.connection, &seat.instance_id)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("session pin: {error:?}")))?;
        let identity_bytes = format!("{}\n{}\n{}\n{}\n{}", self.root.canonical_root().identity.opaque(),
            request.domain_id, request.target_id, seat.incarnation, generation);
        let suffix = crate::store::digest::sha256_hex(identity_bytes.as_bytes());
        let suffix = &suffix[..40];
        let binding_id = format!("binding-{suffix}");
        let home_id = format!("home-{suffix}");
        self.connection.execute("BEGIN IMMEDIATE")
            .map_err(|error| OrchestrationError::Atomic(error.into()))?;
        let bound = (|| -> Result<()> {
            authority::check_owner_in_current_transaction(&self.connection, &self.owner)?;
            let now = seat::get(&self.connection, &request.domain_id, seat_id)?
                .ok_or(OrchestrationError::AccessDenied)?;
            if now != seat { return Err(OrchestrationError::OperationConflict); }
            let found = Statement::prepare(self.connection.as_ptr(),
                "SELECT binding_id,instance_id,generation,state FROM main.gogoke_v37_h_owner_binding WHERE domain_id=?1 AND kind='SESSION' AND owner_id=?2 AND generation=?3")?;
            found.bind_text(1, &request.domain_id)?;
            found.bind_text(2, &request.target_id)?;
            found.bind_text(3, generation)?;
            if found.step_row()? {
                if found.column_text(0)? != binding_id || found.column_text(1)? != seat.instance_id
                    || found.column_text(2)? != generation || found.column_text(3)? != "ACTIVE"
                    || found.step_row()? { return Err(OrchestrationError::OperationConflict); }
            } else {
                h::bind_owner_in_transaction(&mut self.connection, &OwnerBinding {
                    binding_id: &binding_id, instance_id: &seat.instance_id,
                    domain_id: &request.domain_id, kind: "SESSION", owner_id: &request.target_id,
                    generation,
                }).map_err(|error| OrchestrationError::V37StoreFailure(format!("session home binding: {error:?}")))?;
            }
            Ok(())
        })();
        match bound {
            Ok(()) => self.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause)?,
            Err(error) => {
                self.connection.execute("ROLLBACK").map_err(OrchestrationError::CommitUnknownWithCause)?;
                return Err(error);
            }
        }
        let profile = AppContainerProfile::ensure(&format!("Gogoke37.Session.{suffix}"), false)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("session profile: {error}")))?;
        let preparation_hash = crate::store::digest::sha256_hex(
            format!("{}\n{}", request.domain_id, request.request_id).as_bytes());
        let preparation_id = format!("homeprep-{}", &preparation_hash[..40]);
        let home = instance::create_temporary_home(&mut self.connection, self.root, &profile,
            &instance::CreateTemporaryHome {
                request_id: &preparation_id, request_bytes: &request.raw_bytes, home_id: &home_id,
                instance_id: &seat.instance_id, domain_id: &request.domain_id,
                kind: instance::TemporaryKind::Session, owner_id: &request.target_id, generation,
            }).map_err(|error| OrchestrationError::V37StoreFailure(format!("session home preparation: {error:?}")))?;
        if !matches!(home.disposition, "APPLIED" | "REPLAYED") {
            return Err(OrchestrationError::V37StoreFailure(format!("session home unresolved: {}", home.disposition)));
        }
        Ok((seat.instance_id, home_id))
    }

    pub(super) fn dispatch_user_session(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        // One K-SESSION request identity covers both operation and stdin
        // journals. Check it before any home/process/provider side effect.
        let prior = Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT request_hex FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT lower(hex(request_bytes)) FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1, &request.domain_id)?;
        prior.bind_text(2, &request.request_id)?;
        let original: String = request.raw_bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        while prior.step_row()? {
            if prior.column_text(0)? != original {
                return Ok(encode_receipt(request, V37Status::Conflict,
                    request.expected_revision, request.expected_revision, Default::default()));
            }
        }
        drop(prior);
        if request.operation == "open" { return self.dispatch_native_open(request); }
        if request.operation == "resume" { return self.dispatch_native_resume(request); }
        if request.operation == "stop" { return self.dispatch_native_stop(request); }
        if request.operation == "send" { return self.dispatch_native_send(request); }
        if request.operation == "append-without-turn" { return self.dispatch_native_send(request); }
        if request.operation == "output-stream" { return self.dispatch_native_output(request); }
        if request.operation == "capability-probe" { return self.dispatch_native_capability(request); }
        if !matches!(request.operation.as_str(), "admission-reserve" | "admission-commit" | "admission-release") {
            return Ok(encode_receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if request.payload.len() != 2 {
            return Ok(encode_receipt(request, V37Status::Denied,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let seat_id = user_payload_string(request, "seatId")?;
        let generation = user_payload_string(request, "generation")?;
        let expected = i64::try_from(request.expected_revision).map_err(|error|
            OrchestrationError::V37StoreFailure(format!("admission revision: {error}")))?;
        let (instance_id, home_id) = if request.operation == "admission-reserve" {
            if expected != 0 {
                return Ok(encode_receipt(request, V37Status::Denied, 0, 0, Default::default()));
            }
            match self.prepare_user_session_home(request, &seat_id, &generation) {
                Ok(pair) => pair,
                Err(error) => {
                    let status = match &error {
                        OrchestrationError::AccessDenied | OrchestrationError::Invalid(_) => V37Status::Denied,
                        OrchestrationError::OperationConflict => V37Status::Conflict,
                        _ => V37Status::Unknown,
                    };
                    return Ok(encode_receipt(request, status, 0, 0, BTreeMap::from([
                        (JsonString::from_str("reason"), text(&format!("native session preparation: {error:?}")))])));
                }
            }
        } else {
            let row = Statement::prepare(self.connection.as_ptr(),
                "SELECT a.instance_id,a.home_id FROM main.gogoke_v37_h_claim AS a JOIN main.gogoke_v37_h_seat_binding AS s ON s.domain_id=a.domain_id AND s.session_id=a.session_id AND s.generation=a.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND s.seat_id=?3 AND a.generation=?4")?;
            for (index, value) in [request.domain_id.as_str(), request.target_id.as_str(),
                seat_id.as_str(), generation.as_str()].iter().enumerate() { row.bind_text((index + 1) as i32, value)?; }
            if !row.step_row()? {
                return Ok(encode_receipt(request, V37Status::Conflict, request.expected_revision,
                    request.expected_revision, Default::default()));
            }
            let pair = (row.column_text(0)?, row.column_text(1)?);
            if row.step_row()? { return Err(OrchestrationError::OperationConflict); }
            pair
        };
        let input = AdmissionRequest { domain_id: &request.domain_id, session_id: &request.target_id,
            request_id: &request.request_id, raw_bytes: &request.raw_bytes, instance_id: &instance_id,
            home_id: &home_id, generation: &generation, expected_revision: expected };
        let origin = NativeOrigin::user(&self.owner);
        let outcome = match request.operation.as_str() {
            "admission-reserve" => runtime::reserve_native(&mut self.connection, &origin, &seat_id, &input),
            "admission-commit" => runtime::commit_native(&mut self.connection, &origin, &seat_id, &input),
            _ => runtime::release_native(&mut self.connection, &origin, &input),
        };
        let (status, revision, reason) = match outcome {
            Ok(AdmissionResult::Applied(revision)) => (V37Status::Applied, revision, None),
            Ok(AdmissionResult::Replayed(revision)) => (V37Status::Replayed, revision, None),
            Ok(AdmissionResult::Conflict) => (V37Status::Conflict, expected, None),
            Ok(AdmissionResult::Stale) => (V37Status::Stale, expected, None),
            Ok(AdmissionResult::Unknown) => (V37Status::Unknown, expected, None),
            Err(error) => (admission_status(&error), expected, Some(format!("native admission: {error:?}"))),
        };
        let mut result = BTreeMap::new();
        if let Some(reason) = reason { result.insert(JsonString::from_str("reason"), text(&reason)); }
        result.insert(JsonString::from_str("generation"), text(&generation));
        Ok(encode_receipt(request, status, request.expected_revision,
            u64::try_from(revision).map_err(|error|
                OrchestrationError::V37StoreFailure(format!("admission result revision: {error}")))?, result))
    }
}

#[cfg(all(test, windows))]
#[path = "v37_session_tests.rs"]
mod tests;
