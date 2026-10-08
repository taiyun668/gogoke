//! Owner-origin H admission uses the product's one E/F connection. Paths and
//! capacities are resolved natively; neither is accepted in operation payloads.
use super::*;
use crate::process::AppContainerProfile;
use crate::store::ledger::{self,SessionPurpose};
use crate::store::seat::{self, NativeOrigin, State, SecretaryConfiguration};
use crate::store::session_transport::{self as h, runtime, AdmissionError,
    AdmissionRequest, AdmissionResult, OwnerBinding};

fn text(value: &str) -> Json { Json::String(JsonString::from_str(value)) }

fn worktree_failure(request: &V37Request, error: crate::store::worktree::WorktreeError)
    -> Vec<u8> {
    use crate::store::worktree::WorktreeError;
    let status = match error.without_context() {
        WorktreeError::Denied | WorktreeError::Invalid(_) => V37Status::Denied,
        WorktreeError::Conflict => V37Status::Conflict,
        _ => V37Status::Unknown,
    };
    encode_receipt(request, status, request.expected_revision, request.expected_revision,
        BTreeMap::from([(JsonString::from_str("reason"),
            text(&format!("native worktree: {error:?}")))]))
}

/// M1 create and F.2 lifecycle effects use separate native journals. A
/// request ID cannot cross that boundary with a different operation or bytes.
fn worktree_request_identity_matches(db: &VerifiedDatabaseConnection<'_>,
    request: &V37Request) -> Result<bool> {
    let prior = Statement::prepare(db.as_ptr(),
        "SELECT request_hash,'create' FROM main.gogoke_v37_worktree_operations WHERE request_id=?1 UNION ALL SELECT request_hash,lower(operation) FROM main.gogoke_v37_worktree_lifecycle_ops WHERE request_id=?1")?;
    prior.bind_text(1, &request.request_id)?;
    let hash = crate::store::digest::sha256_hex(&request.raw_bytes);
    while prior.step_row()? {
        if prior.column_text(0)? != hash || prior.column_text(1)? != request.operation {
            return Ok(false);
        }
    }
    Ok(true)
}

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
    fn require_one_active_secretary_claim(&self,session_id:&str,
        seat_id:&str,incarnation:&str)->Result<()> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT a.session_id,a.state FROM main.gogoke_v37_h_claim a
               JOIN main.gogoke_v37_native_selection b ON b.domain_id=a.domain_id
                 AND b.session_id=a.session_id
              WHERE a.domain_id='global' AND b.seat_id=?1 AND b.seat_incarnation=?2
            ")?;
        q.bind_text(1,seat_id)?;q.bind_text(2,incarnation)?;
        while q.step_row()? {
            let existing=q.column_text(0)?;
            let state=q.column_text(1)?;
            if existing==session_id && state=="RELEASED" {
                return Err(OrchestrationError::OperationConflict);
            }
            if existing!=session_id && state!="RELEASED" {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        Ok(())
    }
    fn secretary_current_seat(&self)->Result<seat::Seat> {
        let SecretaryConfiguration::Designated {seat_id,incarnation,
            instance_id:Some(_),model:Some(_),effort:Some(_),permission:Some(_),..}=
            seat::read_secretary_configuration_in_transaction(&self.connection,&self.owner)? else {
            return Err(OrchestrationError::AccessDenied);
        };
        seat::require_secretary_session(&self.connection,&self.owner,&seat_id,&incarnation)
            .map_err(Into::into)
    }

    /// An H journal replay precedes mutable E qualification. The original
    /// H seat binding supplies the initial admission generation even when a
    /// later physical resume has advanced the current claim generation.
    fn original_secretary_admission(&self,request:&V37Request)->Result<Option<Vec<u8>>> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex,operation,session_id,status,previous_revision,revision
               FROM main.gogoke_v37_h_operation WHERE domain_id='global' AND request_id=?1")?;
        q.bind_text(1,&request.request_id)?;
        if !q.step_row()? {return Ok(None);}
        let raw:String=request.raw_bytes.iter().map(|byte|format!("{byte:02x}")).collect();
        let same=q.column_text(0)?==raw
            && q.column_text(1)?==request.operation && q.column_text(2)?==request.target_id;
        let state=q.column_text(3)?;
        let before=q.column_text(4)?.parse::<u64>().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("secretary admission prior revision: {error}")))?;
        let after=q.column_text(5)?.parse::<u64>().map_err(|error|
            OrchestrationError::V37StoreFailure(format!("secretary admission result revision: {error}")))?;
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
        drop(q);
        if !same {return Ok(Some(encode_receipt(request,V37Status::Conflict,
            request.expected_revision,request.expected_revision,Default::default())));}
        let binding=Statement::prepare(self.connection.as_ptr(),
            "SELECT CAST(b.seat_authorization_generation AS TEXT),a.generation FROM main.gogoke_v37_h_claim a
               JOIN main.gogoke_v37_native_selection b ON b.domain_id=a.domain_id
                 AND b.session_id=a.session_id
               JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=b.domain_id
                 AND d.seat_id=b.seat_id AND d.incarnation=b.seat_incarnation
              WHERE a.domain_id='global' AND a.session_id=?1")?;
        binding.bind_text(1,&request.target_id)?;
        let generation=if binding.step_row()? {
            let value=if request.operation=="admission-release" {binding.column_text(1)?}
                else {binding.column_text(0)?};
            if binding.step_row()? {return Err(OrchestrationError::OperationConflict);}
            Some(value)
        } else {None};
        let status=if state=="APPLIED" && generation.is_some() {V37Status::Replayed}
            else {V37Status::Unknown};
        let mut result=BTreeMap::new();
        if let Some(generation)=generation {result.insert(JsonString::from_str("generation"),text(&generation));}
        Ok(Some(encode_receipt(request,status,before,after,result)))
    }

    fn original_secretary_claim(&self,session_id:&str)->Result<(String,String,String,String)> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT b.seat_id,a.generation,a.instance_id,a.home_id
               FROM main.gogoke_v37_h_claim a
               JOIN main.gogoke_v37_native_selection b ON b.domain_id=a.domain_id
                 AND b.session_id=a.session_id
               JOIN main.gogoke_v37_seat_secretary d ON d.domain_id=b.domain_id
                 AND d.seat_id=b.seat_id AND d.incarnation=b.seat_incarnation
              WHERE a.domain_id='global' AND a.session_id=?1")?;
        q.bind_text(1,session_id)?;
        if !q.step_row()? {return Err(OrchestrationError::AccessDenied);}
        let result=(q.column_text(0)?,q.column_text(1)?,q.column_text(2)?,q.column_text(3)?);
        if q.step_row()? {return Err(OrchestrationError::OperationConflict);}
        if let Some(registration)=ledger::read_registered_session(&self.connection,session_id)? {
            if registration.domain_id!="global" || registration.seat_id!=result.0
                || registration.purpose!=SessionPurpose::Secretary {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        Ok(result)
    }

    /// The USER supplies a session identity and request bytes only. H derives
    /// the sole configured E seat and the committed H generation itself.
    pub(super) fn secretary_committed_selection(&self,request:&V37Request)->Result<(String,String)> {
        if request.domain_id!="global" || !request.payload.is_empty() {
            return Err(OrchestrationError::AccessDenied);
        }
        let seat=self.secretary_current_seat()?;
        self.require_one_active_secretary_claim(&request.target_id,&seat.seat_id,&seat.incarnation)?;
        let claim=runtime::observe_claim_bound(&self.connection,"global",
            &seat.seat_id,&request.target_id)
            .map_err(|error|OrchestrationError::V37StoreFailure(format!("secretary claim: {error:?}")))?
            .ok_or(OrchestrationError::AccessDenied)?;
        if claim.phase!=runtime::SessionPhase::Committed || claim.instance_id!=seat.instance_id
            || claim.generation!=seat.generation.to_string() {
            return Err(OrchestrationError::AccessDenied);
        }
        Ok((seat.seat_id,claim.generation))
    }
    /// K-WORKTREE creation is User-only at the parent ingress. Source and Git
    /// paths live solely in the separate Owner configuration plane; this
    /// closed operation accepts logical repository/seat IDs only.
    pub(super) fn dispatch_user_worktree(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        use crate::store::worktree::{self as f, WorktreeError};
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if !worktree_request_identity_matches(&self.connection, request)? {
            return Ok(encode_receipt(request, V37Status::Conflict,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if request.operation == "register" {
            return Ok(match f::register_created_worktree(&mut self.connection, self.root,
                &self.owner, &request.raw_bytes) {
                Ok(receipt) => encode_receipt(request,
                    if receipt.replayed { V37Status::Replayed } else { V37Status::Applied },
                    1, receipt.revision as u64,
                    BTreeMap::from([(JsonString::from_str("worktreeId"),
                        text(&receipt.worktree_id))])),
                Err(error) => worktree_failure(request, error),
            });
        }
        if matches!(request.operation.as_str(), "classify-single-or-mixed" | "graph-query") {
            if !request.payload.is_empty() {
                return Ok(encode_receipt(request, V37Status::Denied,
                    request.expected_revision, request.expected_revision, Default::default()));
            }
            return Ok(match f::graph_query(&self.connection, &request.target_id) {
                Ok(Some(graph)) if graph.members.iter().any(|member|
                    member.worktree_id == request.target_id && member.domain_id == request.domain_id) => {
                    let revision = graph.revision as u64;
                    if request.expected_revision != revision {
                        encode_receipt(request, V37Status::Stale, revision, revision,
                            Default::default())
                    } else {
                        let mut result = BTreeMap::from([
                            (JsonString::from_str("spaceId"), text(&graph.space_id)),
                            (JsonString::from_str("classification"), text(&graph.classification)),
                        ]);
                        if request.operation == "graph-query" {
                            result.insert(JsonString::from_str("state"), text(&graph.state));
                            result.insert(JsonString::from_str("mergeReason"),
                                graph.merge_reason.as_deref().map(text).unwrap_or(Json::Null));
                            result.insert(JsonString::from_str("mergeTargetCommit"),
                                graph.merge_target_commit.as_deref().map(text).unwrap_or(Json::Null));
                            result.insert(JsonString::from_str("members"), Json::Array(
                                graph.members.iter().map(|member| Json::Object(BTreeMap::from([
                                    (JsonString::from_str("worktreeId"), text(&member.worktree_id)),
                                    (JsonString::from_str("repositoryId"), text(&member.repository_id)),
                                    (JsonString::from_str("domainId"), text(&member.domain_id)),
                                    (JsonString::from_str("seatId"), text(&member.seat_id)),
                                    (JsonString::from_str("instanceId"), text(&member.instance_id)),
                                    (JsonString::from_str("baselineCommit"), text(&member.baseline_commit)),
                                ]))).collect()));
                        }
                        encode_receipt(request, V37Status::Applied, revision, revision, result)
                    }
                }
                Ok(_) => encode_receipt(request, V37Status::Denied,
                    request.expected_revision, request.expected_revision, Default::default()),
                Err(error) => worktree_failure(request, error),
            });
        }
        if request.operation == "cleanup" {
            let result = (|| {
                let repository = f::repository_for_worktree(&self.connection,
                    &request.domain_id, &request.target_id)?;
                let pin = f::resolve_registered_git(&mut self.connection, self.root,
                    &self.owner, &repository, &mut self.process_custodian)?;
                f::cleanup_worktree(&mut self.connection, self.root, &self.owner,
                    &pin, &mut self.process_custodian, &request.raw_bytes)
            })();
            return Ok(match result {
                Ok(receipt) => encode_receipt(request,
                    if receipt.replayed { V37Status::Replayed } else { V37Status::Applied },
                    request.expected_revision, receipt.revision as u64,
                    BTreeMap::from([
                        (JsonString::from_str("worktreeId"), text(&receipt.worktree_id)),
                        (JsonString::from_str("stopFactId"), text(&receipt.stop_fact_id)),
                    ])),
                Err(error) => worktree_failure(request, error),
            });
        }
        if request.operation != "create" {
            return Ok(encode_receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if request.expected_revision != 0 || !matches!(request.payload.len(), 2 | 3) {
            return Ok(encode_receipt(request, V37Status::Denied, 0, 0, Default::default()));
        }
        let repository = user_payload_string(request, "repositoryId")?;
        let seat_id = user_payload_string(request, "seatId")?;
        let layout = if request.payload.len() == 2 { None } else {
            match request.payload.get(&JsonString::from_str("layout")) {
                Some(Json::String(value)) => match value.to_well_formed_string().as_deref() {
                    Some("single") => Some("single"),
                    Some("mixed") => Some("mixed"),
                    _ => return Ok(encode_receipt(request, V37Status::Denied,
                        0, 0, Default::default())),
                },
                _ => return Ok(encode_receipt(request, V37Status::Denied,
                    0, 0, Default::default())),
            }
        };
        let readback = f::readback_create_receipt(&self.connection,
            &request.request_id, &request.raw_bytes, &request.target_id,
            &repository, &request.domain_id, &seat_id);
        let mut replayed = false;
        let result = match readback {
            Ok(Some(history)) => { replayed = true; Ok(history) }
            Ok(None) => (|| {
                let pin = f::resolve_registered_git(&mut self.connection, self.root,
                    &self.owner, &repository, &mut self.process_custodian)?;
                let input = f::CreateWorktree {
                    request_id: &request.request_id, request_bytes: &request.raw_bytes,
                    target_id: &request.target_id, repository_id: &repository,
                    domain_id: &request.domain_id, seat_id: &seat_id,
                };
                let binding = match layout {
                    None => f::create_worktree(&mut self.connection, self.root, &self.owner,
                        &pin, &mut self.process_custodian, input),
                    Some("single") => f::create_m2_single_worktree(&mut self.connection,
                        self.root, &self.owner, &pin, &mut self.process_custodian, input),
                    Some("mixed") => f::create_mixed_worktree(&mut self.connection,
                        self.root, &self.owner, &pin, &mut self.process_custodian, input),
                    _ => Err(WorktreeError::Denied),
                }?;
                let graph = f::graph_query(&self.connection, &binding.worktree_id)?
                    .ok_or(WorktreeError::Unknown)?;
                if !graph.members.iter().any(|member|
                    member.worktree_id == binding.worktree_id
                        && member.domain_id == request.domain_id
                        && member.repository_id == repository
                        && member.seat_id == seat_id)
                    || graph.state != (if layout.is_some() { "CREATED" } else { "REGISTERED" }) {
                    return Err(WorktreeError::Unknown);
                }
                Ok(f::CreateHistory { worktree_id: binding.worktree_id,
                    baseline_commit: binding.baseline_commit,
                    classification: graph.classification, space_id: graph.space_id })
            })(),
            Err(error) => Err(error),
        };
        match result {
            Ok(history) => {
                if history.classification != (if layout == Some("mixed") { "MIXED" } else { "SINGLE" }) {
                    return Ok(worktree_failure(request, WorktreeError::Unknown));
                }
                let mut result = BTreeMap::from([
                    (JsonString::from_str("worktreeId"), text(&history.worktree_id)),
                    (JsonString::from_str("repositoryId"), text(&repository)),
                    (JsonString::from_str("seatId"), text(&seat_id)),
                    (JsonString::from_str("classification"), text(&history.classification)),
                    (JsonString::from_str("baselineCommit"), text(&history.baseline_commit)),
                ]);
                if layout.is_some() {
                    result.insert(JsonString::from_str("state"), text("CREATED"));
                    result.insert(JsonString::from_str("spaceId"), text(&history.space_id));
                }
                Ok(encode_receipt(request,
                    if replayed { V37Status::Replayed } else { V37Status::Applied },
                    0, 1, result))
            }
            Err(error) => {
                let status = match error.without_context() {
                    WorktreeError::Denied | WorktreeError::Invalid(_) => V37Status::Denied,
                    WorktreeError::Conflict => V37Status::Conflict,
                    _ => V37Status::Unknown,
                };
                Ok(encode_receipt(request, status, 0, 0, BTreeMap::from([
                    (JsonString::from_str("reason"), text(&format!("native worktree: {error:?}")))])))
            }
        }
    }

    /// H will call this with its verified current seat and original turn. The
    /// public User pipe cannot manufacture NativeSeatCall or a merge grant.
    pub(super) fn dispatch_native_worktree(&mut self, request: &V37Request,
        caller: &seat::NativeSeatCall) -> Result<Vec<u8>> {
        use crate::store::worktree as f;
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if !worktree_request_identity_matches(&self.connection, request)? {
            return Ok(encode_receipt(request, V37Status::Conflict,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if request.family=="K-WORKTREE" && request.operation=="create" {
            let child=seat::get(&self.connection,&request.domain_id,&request.target_id)?
                .ok_or(OrchestrationError::AccessDenied)?;
            seat::authorize_child_dispatch(&self.connection,caller,&child)?;
            let repository=user_payload_string(request,"repositoryId")?;
            let outcome=(|| {
                let pin=f::resolve_registered_git(&mut self.connection,self.root,
                    &self.owner,&repository,&mut self.process_custodian)?;
                f::create_and_register_native_child_worktree(&mut self.connection,self.root,
                    &self.owner,&pin,&mut self.process_custodian,caller,&child,request)
            })();
            return Ok(match outcome {
                Ok(binding)=>encode_receipt(request,V37Status::Applied,0,binding.revision as u64,
                    BTreeMap::from([
                        (JsonString::from_str("worktreeId"),text(&binding.worktree_id)),
                        (JsonString::from_str("state"),text("REGISTERED")),
                    ])),
                Err(error)=>worktree_failure(request,error),
            });
        }
        if request.family != "K-WORKTREE" || request.operation != "merge" {
            return Ok(encode_receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let result = (|| {
            let mut authorize = |db: &VerifiedDatabaseConnection<'_>,
                domain: &str, writer_seat: &str, target: &str| {
                if target != request.target_id { return Err(f::WorktreeError::Denied); }
                seat::authorize_merge_for_f2(db, caller, domain, writer_seat)
                    .map_err(f::WorktreeError::Seat)
            };
            if let Some(receipt) = f::readback_merge_receipt_request(
                &mut self.connection, request, &mut authorize)
                .map_err(|error| error.at("native-merge.history"))? {
                return Ok(receipt);
            }
            let repository = f::repository_for_worktree(&self.connection,
                &request.domain_id, &request.target_id)
                .map_err(|error| error.at("native-merge.repository"))?;
            let pin = f::resolve_registered_git(&mut self.connection, self.root,
                &self.owner, &repository, &mut self.process_custodian)
                .map_err(|error| error.at("native-merge.registered-git"))?;
            f::merge_worktree_request(&mut self.connection, self.root, &pin,
                &mut self.process_custodian, request, &mut authorize)
        })();
        Ok(match result {
            Ok(receipt) => {
                let mut result = BTreeMap::from([
                    (JsonString::from_str("worktreeId"), text(&receipt.worktree_id)),
                    (JsonString::from_str("targetCommit"), text(&receipt.target_commit)),
                ]);
                // Legacy receipts have neither field. Preserve their original
                // projection instead of inventing a child seal or null facts.
                if let Some(intent) = &receipt.child_seal_intent {
                    result.insert(JsonString::from_str("childSealIntent"), text(intent));
                }
                if let Some(commit) = &receipt.child_commit {
                    result.insert(JsonString::from_str("childCommit"), text(commit));
                }
                encode_receipt(request,
                    if receipt.replayed { V37Status::Replayed } else { V37Status::Applied },
                    request.expected_revision, receipt.revision as u64, result)
            },
            Err(error) => worktree_failure(request, error),
        })
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
        self.prepare_session_home(request, seat_id, generation, false, None, None)
    }

    /// A stopped session keeps its admission. A resume candidate prepares a
    /// separate F home for the next process generation while E remains BUSY.
    pub(super) fn prepare_resume_session_home(&mut self, request: &V37Request,
        seat_id: &str, generation: &str) -> Result<(String, String)> {
        self.prepare_session_home(request, seat_id, generation, true, None, None)
    }

    pub(super) fn prepare_native_child_session_home(&mut self,request:&V37Request,
        seat_id:&str,generation:&str,caller:&seat::NativeSeatCall)->Result<(String,String)> {
        self.prepare_session_home(request,seat_id,generation,false,Some(caller),None)
    }

    fn check_native_child_home_caller(&self,request:&V37Request,child:&seat::Seat,
        caller:&seat::NativeSeatCall)->Result<()> {
        if request.domain_id!=caller.domain_id() {return Err(OrchestrationError::AccessDenied)};
        if child.state==State::Idle {seat::authorize_child_dispatch(&self.connection,caller,child)?;}
        else {
            let admission=seat::NativeLeadAdmission::from_model_call(caller)?;
            runtime::observe_claim(&self.connection,&NativeOrigin::lead(&admission),
                &request.domain_id,&child.seat_id,&request.target_id)?
                .ok_or(OrchestrationError::AccessDenied)?;
        }
        Ok(())
    }

    pub(super) fn prepare_host_recipient_session_home(&mut self,request:&V37Request,
        seat_id:&str,generation:&str,resume:bool,
        proof:&seat::HostEscalationProof,
        choice:&crate::store::inbox::host_rule::HostRecipient)->Result<(String,String)> {
        self.prepare_session_home(request,seat_id,generation,resume,None,Some((proof,choice)))
    }

    fn prepare_session_home(&mut self, request: &V37Request, seat_id: &str,
        generation: &str, resume: bool,caller:Option<&seat::NativeSeatCall>,
        host:Option<(&seat::HostEscalationProof,&crate::store::inbox::host_rule::HostRecipient)>)
        -> Result<(String, String)> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        if let Some((proof,choice))=host {self.check_host_recipient_choice(proof,choice)?;}
        let seat = seat::get(&self.connection, &request.domain_id, seat_id)?
            .ok_or(OrchestrationError::AccessDenied)?;
        if request.domain_id=="global" {
            if caller.is_some() || host.is_some() ||
                seat::require_secretary_session(&self.connection,&self.owner,
                    &seat.seat_id,&seat.incarnation)?!=seat {
                return Err(OrchestrationError::AccessDenied);
            }
            self.require_one_active_secretary_claim(&request.target_id,&seat.seat_id,&seat.incarnation)?;
            if resume && !ledger::read_registered_session(&self.connection,&request.target_id)?
                .is_some_and(|row|row.domain_id=="global" && row.seat_id==seat.seat_id
                    && row.purpose==SessionPurpose::Secretary) {
                return Err(OrchestrationError::AccessDenied);
            }
        }
        if let Some(caller)=caller {self.check_native_child_home_caller(request,&seat,caller)?;}
        if seat.instance_id.is_empty() || !matches!(seat.state, State::Idle | State::Busy) {
            return Err(OrchestrationError::AccessDenied);
        }
        let native_resume=if resume {
            h::session_binding::read_pending(&self.connection,&request.domain_id,&request.target_id)
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("native resume selection: {error:?}")))?
        } else {None};
        let (expected,selected_instance)=if let Some(binding)=native_resume {
            if binding.seat_id!=seat.seat_id || binding.seat_incarnation!=seat.incarnation
                || binding.seat_authorization_generation!=seat.generation
                || binding.selected_instance_id!=seat.instance_id || seat.state!=State::Busy {
                return Err(OrchestrationError::OperationConflict);
            }
            let original=h::session_binding::read(&self.connection,&request.domain_id,&request.target_id)
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("native resume relationship: {error:?}")))?;
            if original.as_ref()!=Some(&binding) {return Err(OrchestrationError::OperationConflict);}
            let old=runtime::observe_claim_bound(&self.connection,&request.domain_id,seat_id,&request.target_id)?
                .ok_or(OrchestrationError::OperationConflict)?;
            if old.phase!=runtime::SessionPhase::Stopped || old.instance_id!=binding.selected_instance_id {
                return Err(OrchestrationError::OperationConflict);
            }
            (old.generation.parse::<i64>().ok().and_then(|n|n.checked_add(1))
                .ok_or(OrchestrationError::OperationConflict)?,binding.selected_instance_id)
        } else if seat.state == State::Idle || resume {
            (seat.generation.checked_add(1).ok_or(OrchestrationError::OperationConflict)?,seat.instance_id.clone())
        } else {(seat.generation,seat.instance_id.clone())};
        if expected.to_string() != generation { return Err(OrchestrationError::OperationConflict); }
        // Refuse absent configuration before any filesystem work. H reads both
        // again under BEGIN IMMEDIATE when it decides actual capacity.
        seat::read_project_parallel_cap(&self.connection, &request.domain_id)?;
        instance::read_instance_concurrency_cap(&self.connection, &selected_instance)?;
        runtime::current_instance_pin(&self.connection, &selected_instance)
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
            if request.domain_id=="global" &&
                seat::require_secretary_session(&self.connection,&self.owner,
                    &now.seat_id,&now.incarnation)?!=now {
                return Err(OrchestrationError::AccessDenied);
            }
            if request.domain_id=="global" {
                self.require_one_active_secretary_claim(&request.target_id,&now.seat_id,&now.incarnation)?;
            }
            if let Some(caller)=caller {self.check_native_child_home_caller(request,&now,caller)?;}
            if let Some((proof,choice))=host {
                self.check_host_recipient_choice_in_transaction(proof,choice)?;
            }
            let found = Statement::prepare(self.connection.as_ptr(),
                "SELECT binding_id,instance_id,generation,state FROM main.gogoke_v37_h_owner_binding WHERE domain_id=?1 AND kind='SESSION' AND owner_id=?2 AND generation=?3")?;
            found.bind_text(1, &request.domain_id)?;
            found.bind_text(2, &request.target_id)?;
            found.bind_text(3, generation)?;
            if found.step_row()? {
                if found.column_text(0)? != binding_id || found.column_text(1)? != selected_instance
                    || found.column_text(2)? != generation || found.column_text(3)? != "ACTIVE"
                    || found.step_row()? { return Err(OrchestrationError::OperationConflict); }
            } else {
                h::bind_owner_in_transaction(&mut self.connection, &OwnerBinding {
                    binding_id: &binding_id, instance_id: &selected_instance,
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
        if let Some((proof,choice))=host {self.check_host_recipient_choice(proof,choice)?;}
        let profile = AppContainerProfile::ensure(&format!("Gogoke37.Session.{suffix}"), false)
            .map_err(|error| OrchestrationError::V37StoreFailure(format!("session profile: {error}")))?;
        let preparation_hash = crate::store::digest::sha256_hex(
            format!("{}\n{}", request.domain_id, request.request_id).as_bytes());
        let preparation_id = format!("homeprep-{}", &preparation_hash[..40]);
        if let Some(caller)=caller {self.check_native_child_home_caller(request,&seat,caller)?;}
        if let Some((proof,choice))=host {self.check_host_recipient_choice(proof,choice)?;}
        let home = instance::create_temporary_home(&mut self.connection, self.root, &profile,
            &instance::CreateTemporaryHome {
                request_id: &preparation_id, request_bytes: &request.raw_bytes, home_id: &home_id,
                instance_id: &selected_instance, domain_id: &request.domain_id,
                kind: instance::TemporaryKind::Session, owner_id: &request.target_id, generation,
            }).map_err(|error| OrchestrationError::V37StoreFailure(format!("session home preparation: {error:?}")))?;
        if !matches!(home.disposition, "APPLIED" | "REPLAYED") {
            return Err(OrchestrationError::V37StoreFailure(format!("session home unresolved: {}", home.disposition)));
        }
        Ok((selected_instance, home_id))
    }

    pub(super) fn dispatch_host_recipient_admission(&mut self,request:&V37Request,
        proof:&seat::HostEscalationProof,
        choice:&crate::store::inbox::host_rule::HostRecipient)->Result<Vec<u8>> {
        let reserve=request.operation=="admission-reserve";
        if request.family!="K-SESSION" || request.domain_id!=proof.domain_id()
            || request.target_id!=choice.session_id || choice.mode!="FRESH"
            || request.payload.len()!=2
            || request.request_id.as_str()!=(if reserve {choice.reserve_request_id.as_str()}
                else {choice.commit_request_id.as_str()})
            || (!reserve && request.operation!="admission-commit")
            || user_payload_string(request,"seatId")?!=proof.destination_seat_id()
            || user_payload_string(request,"generation")?!=choice.generation {
            return Err(OrchestrationError::AccessDenied);
        }
        self.check_host_recipient_choice(proof,choice)?;
        let expected=i64::try_from(request.expected_revision)
            .map_err(|_|OrchestrationError::OperationConflict)?;
        let (instance_id,home_id)=if reserve {
            if expected!=0 {return Err(OrchestrationError::OperationConflict);}
            self.prepare_host_recipient_session_home(request,proof.destination_seat_id(),
                &choice.generation,false,proof,choice)?
        } else {
            let claim=runtime::observe_claim(&self.connection,&NativeOrigin::user(&self.owner),
                proof.domain_id(),proof.destination_seat_id(),&choice.session_id)
                .map_err(|error|OrchestrationError::V37StoreFailure(format!("host commit claim: {error:?}")))?
                .ok_or(OrchestrationError::OperationConflict)?;
            (claim.instance_id,claim.home_id)
        };
        if instance_id!=choice.instance_id {return Err(OrchestrationError::AccessDenied);}
        let input=AdmissionRequest {domain_id:&request.domain_id,session_id:&request.target_id,
            request_id:&request.request_id,raw_bytes:&request.raw_bytes,instance_id:&instance_id,
            home_id:&home_id,generation:&choice.generation,expected_revision:expected};
        let result=if reserve {
            runtime::reserve_native_for_host(&mut self.connection,&self.owner,proof,choice,&input)
        } else {
            runtime::commit_native_for_host(&mut self.connection,&self.owner,proof,choice,&input)
        };
        let (status,revision,reason)=match result {
            Ok(AdmissionResult::Applied(value))=>(V37Status::Applied,value,None),
            Ok(AdmissionResult::Replayed(value))=>(V37Status::Replayed,value,None),
            Ok(AdmissionResult::Conflict)=>(V37Status::Conflict,expected,None),
            Ok(AdmissionResult::Stale)=>(V37Status::Stale,expected,None),
            Ok(AdmissionResult::Unknown)=>(V37Status::Unknown,expected,None),
            Err(error)=>(admission_status(&error),expected,Some(format!("host admission: {error:?}"))),
        };
        let mut body=BTreeMap::new();
        if let Some(reason)=reason {body.insert(JsonString::from_str("reason"),text(&reason));}
        body.insert(JsonString::from_str("generation"),text(&choice.generation));
        Ok(encode_receipt(request,status,request.expected_revision,
            u64::try_from(revision).map_err(|_|OrchestrationError::OperationConflict)?,body))
    }

    pub(super) fn user_session_request_identity_matches(&mut self, request: &V37Request) -> Result<bool> {
        authority::read_product_identity(&mut self.connection, &self.owner)?;
        // One K-SESSION request identity covers both operation and stdin
        // journals. Check it before any home/process/provider side effect.
        let prior = Statement::prepare(self.connection.as_ptr(),
            "SELECT raw_hex FROM main.gogoke_v37_h_operation WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT request_hex FROM main.gogoke_v37_h_stdin_journal WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT raw_hex FROM main.gogoke_v37_h_generation_change WHERE domain_id=?1 AND request_id=?2 UNION ALL SELECT lower(hex(request_bytes)) FROM main.v37_ledger_receipt WHERE family='K-SESSION' AND domain_id=?1 AND request_id=?2")?;
        prior.bind_text(1, &request.domain_id)?;
        prior.bind_text(2, &request.request_id)?;
        let original: String = request.raw_bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        while prior.step_row()? {
            if prior.column_text(0)? != original {
                return Ok(false);
            }
        }
        drop(prior);
        Ok(true)
    }

    pub(super) fn dispatch_user_session(&mut self, request: &V37Request) -> Result<Vec<u8>> {
        self.dispatch_user_session_with_input(request, None)
    }

    pub(super) fn dispatch_verified_user_session(&mut self, request: &V37Request,
        input: &VerifiedDirectUserInput<'_>) -> Result<Vec<u8>> {
        if request.operation != "send" { return Err(OrchestrationError::AccessDenied); }
        self.dispatch_user_session_with_input(request, Some(input))
    }

    fn dispatch_user_session_with_input(&mut self, request: &V37Request,
        input: Option<&VerifiedDirectUserInput<'_>>) -> Result<Vec<u8>> {
        if request.request_id.starts_with("hostrecipient-")
            || (request.target_id.starts_with("hostsession-")
                && !matches!(request.operation.as_str(),
                    "stop" | "output-stream" | "admission-release")) {
            // Host preparation and start are internal. A later User stop,
            // read, or release still needs the original H claim and its
            // ordinary Owner, generation, revision, and stop-fact checks.
            return Ok(encode_receipt(request,V37Status::Denied,request.expected_revision,
                request.expected_revision,Default::default()));
        }
        if !self.user_session_request_identity_matches(request)? {
            return Ok(encode_receipt(request, V37Status::Conflict,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if request.operation == "open" { return self.dispatch_native_open(request); }
        if request.operation == "resume" { return self.dispatch_native_resume(request); }
        if request.operation == "reconnect" { return self.dispatch_native_reconnect(request); }
        if matches!(request.operation.as_str(),"compact"|"renew-session") {
            return self.dispatch_native_generation_change(request);
        }
        if request.operation == "stop" { return self.dispatch_native_stop(request); }
        if request.operation == "send" { return self.dispatch_native_send_with_input(request, input); }
        if request.operation == "append-without-turn" { return self.dispatch_native_send(request); }
        if request.operation == "output-stream" { return self.dispatch_native_output(request); }
        if request.operation == "capability-probe" { return self.dispatch_native_capability(request); }
        if request.operation == "model-list-read" { return self.dispatch_native_model_list(request); }
        if !matches!(request.operation.as_str(), "admission-reserve" | "admission-commit" | "admission-release") {
            return Ok(encode_receipt(request, V37Status::Unsupported,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        let secretary=request.domain_id=="global";
        if request.payload.len() != if secretary {0} else {2} {
            return Ok(encode_receipt(request, V37Status::Denied,
                request.expected_revision, request.expected_revision, Default::default()));
        }
        if secretary {
            if let Some(original)=self.original_secretary_admission(request)? {
                return Ok(original);
            }
        }
        let (seat_id,generation)=if secretary {
            if request.operation=="admission-reserve" {
                let seat=self.secretary_current_seat()?;
                self.require_one_active_secretary_claim(&request.target_id,&seat.seat_id,&seat.incarnation)?;
                let next=if seat.state==State::Idle {
                    seat.generation.checked_add(1).ok_or(OrchestrationError::OperationConflict)?
                } else {seat.generation};
                (seat.seat_id,next.to_string())
            } else {
                let (seat_id,generation,_,_)=self.original_secretary_claim(&request.target_id)?;
                if request.operation=="admission-commit" {
                    let seat=self.secretary_current_seat()?;
                    if seat.seat_id!=seat_id {return Err(OrchestrationError::AccessDenied);}
                    self.require_one_active_secretary_claim(&request.target_id,&seat.seat_id,&seat.incarnation)?;
                }
                (seat_id,generation)
            }
        } else {
            (user_payload_string(request,"seatId")?,user_payload_string(request,"generation")?)
        };
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
                "SELECT a.instance_id,a.home_id FROM main.gogoke_v37_h_claim AS a JOIN main.gogoke_v37_effective_seat AS s ON s.domain_id=a.domain_id AND s.session_id=a.session_id AND s.generation=a.generation WHERE a.domain_id=?1 AND a.session_id=?2 AND s.seat_id=?3 AND a.generation=?4")?;
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
