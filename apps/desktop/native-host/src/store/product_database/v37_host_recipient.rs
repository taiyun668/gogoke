//! Private HostRule recipient preparation. C freezes the selection and exact
//! K-SESSION requests; E/F/H remain the authority for every physical effect.
use super::*;
use crate::store::inbox::host_rule::{self as c, HostRecipient};
use crate::store::ledger::{self, SessionPurpose};
use crate::store::seat::{self, HostEscalationProof, State};
use crate::store::session_transport::{self as h, runtime};
use crate::store::worktree;
use crate::store::atomic::Parser;

fn recipient_error(error: impl std::fmt::Debug) -> OrchestrationError {
    OrchestrationError::V37StoreFailure(format!("host recipient: {error:?}"))
}
fn string(value: &str) -> Json { Json::String(JsonString::from_str(value)) }
fn stage_request(choice: &HostRecipient, proof: &HostEscalationProof,
    stage: &str, revision: u64) -> Result<V37Request> {
    let (operation, request_id, payload) = match stage {
        "reserve" => ("admission-reserve", &choice.reserve_request_id,
            BTreeMap::from([(JsonString::from_str("seatId"),string(proof.destination_seat_id())),
                (JsonString::from_str("generation"),string(&choice.generation))])),
        "commit" => ("admission-commit", &choice.commit_request_id,
            BTreeMap::from([(JsonString::from_str("seatId"),string(proof.destination_seat_id())),
                (JsonString::from_str("generation"),string(&choice.generation))])),
        "start" if choice.mode == "FRESH" => ("open", &choice.start_request_id,
            BTreeMap::from([(JsonString::from_str("seatId"),string(proof.destination_seat_id())),
                (JsonString::from_str("generation"),string(&choice.generation)),
                (JsonString::from_str("repositoryId"),string(&choice.repository_id)),
                (JsonString::from_str("worktreeId"),string(&choice.worktree_id))])),
        "start" if choice.mode == "RESUME" => ("resume", &choice.start_request_id,
            BTreeMap::from([(JsonString::from_str("generation"),string(&choice.generation))])),
        _ => return Err(OrchestrationError::Invalid("host recipient stage")),
    };
    let raw = Json::Object(BTreeMap::from([
        (JsonString::from_str("schema"),string("gogoke.37.operations.v1")),
        (JsonString::from_str("family"),string("K-SESSION")),
        (JsonString::from_str("operation"),string(operation)),
        (JsonString::from_str("requestId"),string(request_id)),
        (JsonString::from_str("targetId"),string(&choice.session_id)),
        (JsonString::from_str("domainId"),string(proof.domain_id())),
        (JsonString::from_str("expectedRevision"),string(&revision.to_string())),
        (JsonString::from_str("payload"),Json::Object(payload)),
    ])).canonical();
    h::decode_request(raw.as_bytes()).map_err(recipient_error)
}

impl<'root> ProductDatabase<'root> {
    /// This check is called only inside an existing BEGIN IMMEDIATE. It binds
    /// the current E route to C's immutable choice and F's same physical tree.
    pub(super) fn check_host_recipient_choice_in_transaction(&mut self,
        proof: &HostEscalationProof, choice: &HostRecipient) -> Result<()> {
        seat::revalidate_host_escalation_in_transaction(&self.connection,&self.owner,proof)
            .map_err(recipient_error)?;
        if c::read_host_recipient(&self.connection,proof).map_err(recipient_error)?.as_ref()!=Some(choice) {
            return Err(OrchestrationError::AccessDenied);
        }
        let seat=seat::get(&self.connection,proof.domain_id(),proof.destination_seat_id())?
            .ok_or(OrchestrationError::AccessDenied)?;
        if seat.incarnation!=choice.seat_incarnation || seat.instance_id!=choice.instance_id
            || seat.state==State::Reclaimed
            || format!("{:?}",seat::permission_tier(&seat).map_err(recipient_error)?)!=choice.permission_tier {
            return Err(OrchestrationError::AccessDenied);
        }
        let valid_generation=if choice.mode=="FRESH" {
            (seat.state==State::Idle && seat.generation.checked_add(1)
                .is_some_and(|next|next.to_string()==choice.generation))
                || (seat.state==State::Busy && seat.generation.to_string()==choice.generation)
        } else {seat.state==State::Busy && seat.generation.to_string()==choice.generation};
        if !valid_generation {return Err(OrchestrationError::AccessDenied);}
        let pin=runtime::current_instance_pin(&self.connection,&choice.instance_id)
            .map_err(recipient_error)?;
        if pin.driver_id!="codex" {return Err(OrchestrationError::AccessDenied);}
        let binding=worktree::resolve_for_launch(&self.connection,self.root,&choice.worktree_id,
            &choice.repository_id,proof.domain_id(),proof.destination_seat_id(),
            &choice.seat_incarnation,seat.generation).map_err(recipient_error)?;
        worktree::resolve_group_for_launch(&self.connection,self.root,&binding)
            .map_err(recipient_error)?;
        Ok(())
    }

    pub(super) fn check_host_recipient_choice(&mut self,
        proof: &HostEscalationProof, choice: &HostRecipient) -> Result<()> {
        self.connection.execute("BEGIN IMMEDIATE")
            .map_err(OrchestrationError::CommitUnknownWithCause)?;
        let checked=self.check_host_recipient_choice_in_transaction(proof,choice);
        match checked {
            Ok(())=>self.connection.execute("COMMIT")
                .map_err(OrchestrationError::CommitUnknownWithCause),
            Err(primary)=>{
                if let Err(rollback)=self.connection.execute("ROLLBACK") {
                    return Err(recipient_error((primary,rollback)));
                }
                Err(primary)
            },
        }
    }

    fn host_recipient_stage(&mut self,proof:&HostEscalationProof,choice:&HostRecipient,
        stage:&str,revision:u64)->Result<V37Request> {
        let proposed=stage_request(choice,proof,stage,revision)?;
        let bytes=c::freeze_host_stage(&mut self.connection,&self.owner,proof,choice,
            stage,&proposed.raw_bytes).map_err(recipient_error)?;
        h::decode_request(&bytes).map_err(recipient_error)
    }

    fn host_recipient_candidate(&mut self,proof:&HostEscalationProof)
        ->Result<Option<HostRecipient>> {
        let seat=seat::get(&self.connection,proof.domain_id(),proof.destination_seat_id())?
            .ok_or(OrchestrationError::AccessDenied)?;
        if seat.instance_id.is_empty() || seat.state==State::Reclaimed {
            return Err(OrchestrationError::V37StoreFailure(
                "host recipient: E destination has no usable bound instance".into()));
        }
        let tier=format!("{:?}",seat::permission_tier(&seat).map_err(recipient_error)?);
        let pin=runtime::current_instance_pin(&self.connection,&seat.instance_id)
            .map_err(recipient_error)?;
        if pin.driver_id!="codex" {return Err(OrchestrationError::AccessDenied);}
        let worktrees=Statement::prepare(self.connection.as_ptr(),
            "SELECT worktree_id,repository_id FROM main.gogoke_v37_worktrees
              WHERE domain_id=?1 AND seat_id=?2 AND seat_incarnation=?3
                AND state='REGISTERED' ORDER BY worktree_id")?;
        worktrees.bind_text(1,proof.domain_id())?;
        worktrees.bind_text(2,proof.destination_seat_id())?;
        worktrees.bind_text(3,&seat.incarnation)?;
        let mut trees=Vec::new();while worktrees.step_row()? {
            trees.push((worktrees.column_text(0)?,worktrees.column_text(1)?));
        }
        drop(worktrees);
        let claims=Statement::prepare(self.connection.as_ptr(),
            "SELECT a.session_id,a.generation,a.state,a.revision
               FROM main.gogoke_v37_h_claim a JOIN main.gogoke_v37_h_seat_binding b
                 ON b.domain_id=a.domain_id AND b.session_id=a.session_id
              WHERE a.domain_id=?1 AND b.seat_id=?2 AND b.seat_incarnation=?3
                AND a.state!='RELEASED' ORDER BY a.session_id")?;
        claims.bind_text(1,proof.domain_id())?;
        claims.bind_text(2,proof.destination_seat_id())?;
        claims.bind_text(3,&seat.incarnation)?;
        let mut current=Vec::new();while claims.step_row()? {
            current.push((claims.column_text(0)?,claims.column_text(1)?,
                claims.column_text(2)?,claims.column_text(3)?));
        }
        drop(claims);
        let ids=c::host_message_ids(proof);
        let basis=ids.enqueue_request_id.replacen("hostenqueue-","hostrecipient-",1);
        let (mode,session_id,generation,worktree_id,repository_id)=match current.as_slice() {
            [] if seat.state==State::Idle => {
                let [(tree,repo)]=trees.as_slice() else {
                    return Err(OrchestrationError::V37StoreFailure(format!(
                        "host recipient: F registered worktree count for first launch is {}",trees.len())));
                };
                let next=seat.generation.checked_add(1)
                    .ok_or(OrchestrationError::OperationConflict)?;
                ("FRESH",basis.replacen("hostrecipient-","hostsession-",1),
                    next.to_string(),tree.clone(),repo.clone())
            },
            [(session,old_generation,state,_)] if state=="STOPPED"
                && seat.state==State::Busy && seat.generation.to_string()==*old_generation => {
                let Some(registered)=ledger::read_registered_session(&self.connection,session)? else {
                    return Err(OrchestrationError::V37StoreFailure(
                        "host recipient: original A WORK registration absent".into()));
                };
                if registered.purpose!=SessionPurpose::Work || registered.domain_id!=proof.domain_id()
                    || registered.seat_id!=proof.destination_seat_id() {return Ok(None);}
                let old=runtime::observe_claim(&self.connection,
                    &seat::NativeOrigin::user(&self.owner),proof.domain_id(),
                    proof.destination_seat_id(),session).map_err(recipient_error)?;
                if old.as_ref().map_or(true,|claim|claim.phase!=runtime::SessionPhase::Stopped
                    || claim.instance_id!=seat.instance_id) {return Ok(None);}
                if runtime::observe_stop_fact(&self.connection,proof.domain_id(),session)
                    .map_err(recipient_error)?.is_none() {return Ok(None);}
                if h::generation_change::active_for_session(&self.connection,proof.domain_id(),session)
                    .map_err(recipient_error)?.is_some() {return Ok(None);}
                let (repo,tree,_)=self.original_native_continuation(proof.domain_id(),session)?;
                if !trees.iter().any(|(tree_id,repo_id)|*tree_id==tree && *repo_id==repo) ||
                    !self.host_stopped_recipient_turn_terminal(proof.domain_id(),session,old_generation)? {
                    return Err(OrchestrationError::V37StoreFailure(
                        "host recipient: original STOPPED WORK thread or terminal does not match F binding".into()));
                }
                ("RESUME",session.clone(),old_generation.clone(),tree,repo)
            },
            _=>return Ok(None),
        };
        let resolved=worktree::resolve_for_launch(&self.connection,self.root,&worktree_id,
            &repository_id,proof.domain_id(),proof.destination_seat_id(),&seat.incarnation,
            seat.generation).map_err(recipient_error)?;
        worktree::resolve_group_for_launch(&self.connection,self.root,&resolved)
            .map_err(recipient_error)?;
        Ok(Some(HostRecipient {mode:mode.into(),session_id,
            seat_incarnation:seat.incarnation,instance_id:seat.instance_id,
            permission_tier:tier,repository_id,worktree_id,
            generation,reserve_request_id:format!("{basis}-reserve"),
            commit_request_id:format!("{basis}-commit"),start_request_id:format!("{basis}-start")}))
    }

    /// A stopped process alone does not mean its vendor turn ended. The last
    /// original H send must have a matching A terminal and no pending input.
    fn host_stopped_initial_idle(&self,domain:&str,session:&str,_generation:&str)
        ->Result<bool> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT hex(r.raw_bytes) FROM main.gogoke_v37_rpc_steps s
               JOIN main.v37_ledger_raw_source r ON r.operation_id=s.process_operation_id
                 AND r.source_epoch=s.source_epoch AND r.source_cursor=s.source_cursor
                 AND r.process_ticket=s.ticket AND r.custodian_nonce=s.custodian_nonce
                 AND r.domain_id=s.domain_id AND r.session_id=s.session_id
                 AND r.generation=s.generation
              WHERE s.domain_id=?1 AND s.session_id=?2 AND s.step_id='thread-start'
                AND s.phase='OBSERVED' AND r.state='NO_EVENT'
                AND r.no_event_reason='CODEX_RPC_RESPONSE'")?;
        q.bind_text(1,domain)?;q.bind_text(2,session)?;
        if !q.step_row()? {return Ok(false);}
        let bytes=super::v37_host_rule::original_bytes(&q.column_text(0)?)?;
        if q.step_row()? {return Ok(false);}drop(q);
        let Some(body)=bytes.strip_suffix(b"\n") else {return Ok(false)};
        let Ok(text)=std::str::from_utf8(body) else {return Ok(false)};
        let Ok(Json::Object(root))=Parser::parse(text) else {return Ok(false)};
        let Some(Json::Object(result))=root.get(&JsonString::from_str("result")) else {return Ok(false)};
        let Some(Json::Object(thread))=result.get(&JsonString::from_str("thread")) else {return Ok(false)};
        let Some(Json::Object(status))=thread.get(&JsonString::from_str("status")) else {return Ok(false)};
        let Some(Json::String(kind))=status.get(&JsonString::from_str("type")) else {return Ok(false)};
        Ok(kind.to_well_formed_string().as_deref()==Some("idle"))
    }

    fn host_stopped_recipient_turn_terminal(&self,domain:&str,session:&str,generation:&str)
        ->Result<bool> {
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT 1 FROM main.gogoke_v37_qcard_native WHERE domain_id=?1 AND seat_id=(
                SELECT seat_id FROM main.gogoke_v37_h_seat_binding WHERE domain_id=?1 AND session_id=?2)
               AND state IN ('OPEN','ANSWER_UNKNOWN') LIMIT 1")?;
        q.bind_text(1,domain)?;q.bind_text(2,session)?;
        if q.step_row()? {return Ok(false);}drop(q);
        let q=Statement::prepare(self.connection.as_ptr(),
            "SELECT COALESCE(receipt_hex,''),phase,generation FROM main.gogoke_v37_h_stdin_journal
              WHERE domain_id=?1 AND session_id=?2
                AND operation='send' ORDER BY rowid DESC")?;
        q.bind_text(1,domain)?;q.bind_text(2,session)?;
        if !q.step_row()? {return self.host_stopped_initial_idle(domain,session,generation);}
        let receipt_hex=q.column_text(0)?;
        if q.column_text(1)?!="RECEIPTED" {return Ok(false);}
        let send_generation=q.column_text(2)?;
        drop(q);
        let receipt_bytes=match super::v37_host_rule::original_bytes(&receipt_hex) {
            Ok(bytes)=>bytes,Err(_)=>return Ok(false)};
        let receipt=match h::decode_receipt(&receipt_bytes) {Ok(value)=>value,Err(_)=>return Ok(false)};
        if receipt.status!=V37Status::Applied {return Ok(false);}
        let turn=match receipt.into_result().get(&JsonString::from_str("turnId")) {
            Some(Json::String(value))=>match value.to_well_formed_string() {Some(value)=>value,None=>return Ok(false)},
            _=>return Ok(false),
        };
        let expected_thread=match self.original_native_continuation(domain,session) {
            Ok((_,_,thread))=>thread,Err(_)=>return Ok(false),
        };
        let sources=Statement::prepare(self.connection.as_ptr(),
            "SELECT hex(r.raw_bytes),r.state,COALESCE(r.no_event_reason,'')
               FROM main.v37_ledger_raw_source r
               JOIN main.gogoke_v37_h_process_episode e ON e.process_operation_id=r.operation_id
                AND e.domain_id=r.domain_id AND e.generation=r.generation
              WHERE r.domain_id=?1 AND r.session_id=?2 AND r.generation=?3
                AND e.phase='STOPPED' ORDER BY CAST(r.source_cursor AS INTEGER)")?;
        sources.bind_text(1,domain)?;sources.bind_text(2,session)?;sources.bind_text(3,&send_generation)?;
        let mut last=None;
        while sources.step_row()? {
            if sources.column_text(1)?=="PENDING" {return Ok(false);}
            let bytes=super::v37_host_rule::original_bytes(&sources.column_text(0)?)?;
            let Some(body)=bytes.strip_suffix(b"\n") else {return Ok(false)};
            let Ok(text)=std::str::from_utf8(body) else {return Ok(false)};
            let Ok(Json::Object(root))=Parser::parse(text) else {return Ok(false)};
            if root.contains_key(&JsonString::from_str("id")) {
                if root.contains_key(&JsonString::from_str("method"))
                    && sources.column_text(2)?!="NATIVE_SERVER_REQUEST_RESOLVED" {
                    return Ok(false);
                }
                continue;
            }
            match h::codex_rpc::decode(&bytes,None) {
                Ok(h::codex_rpc::Reply::TurnNotification {thread_id,turn_id,status,..})=>{
                    if thread_id!=expected_thread {return Ok(false);}
                    if turn_id==turn {last=Some(status);}
                    else {last=None;}
                },
                Ok(h::codex_rpc::Reply::Event {method,..}) if method=="thread/status/changed"=>{
                    let Some(Json::Object(params))=root.get(&JsonString::from_str("params")) else {return Ok(false)};
                    let Some(Json::Object(status))=params.get(&JsonString::from_str("status")) else {return Ok(false)};
                    let Some(Json::String(kind))=status.get(&JsonString::from_str("type")) else {return Ok(false)};
                    if kind.to_well_formed_string().as_deref()!=Some("idle") {last=None;}
                },
                Err(_)=>return Ok(false),
                _=>{},
            }
        }
        Ok(matches!(last,Some(h::codex_rpc::TurnStatus::Completed)))
    }

    pub(super) fn prepare_host_rule_recipient(&mut self,proof:&HostEscalationProof)
        ->Result<Option<(String,String)>> {
        let choice=match c::read_host_recipient(&self.connection,proof).map_err(recipient_error)? {
            Some(choice)=>choice,
            None=>{
                let Some(choice)=self.host_recipient_candidate(proof)? else {return Ok(None)};
                c::reserve_host_recipient(&mut self.connection,&self.owner,proof,&choice)
                    .map_err(recipient_error)?
            },
        };
        self.check_host_recipient_choice(proof,&choice)?;
        let key=(proof.domain_id().to_owned(),choice.session_id.clone());
        if self.native_sessions.contains_key(&key) {return Ok(Some(key));}
        if choice.mode=="RESUME" {
            let Some(claim)=runtime::observe_claim(&self.connection,&seat::NativeOrigin::user(&self.owner),
                proof.domain_id(),proof.destination_seat_id(),&choice.session_id)
                    .map_err(recipient_error)? else {return Ok(None)};
            if claim.phase!=runtime::SessionPhase::Stopped || claim.generation!=choice.generation {
                return Ok(None);
            }
            let request=self.host_recipient_stage(proof,&choice,"start",
                u64::try_from(claim.revision).map_err(recipient_error)?)?;
            let receipt=self.dispatch_host_recipient_resume(&request,proof,&choice)?;
            if !matches!(h::decode_receipt(&receipt).map_err(recipient_error)?.status,
                V37Status::Applied|V37Status::Replayed) {return Ok(None);}
        } else {
            let reserve=self.host_recipient_stage(proof,&choice,"reserve",0)?;
            let receipt=self.dispatch_host_recipient_admission(&reserve,proof,&choice)?;
            if !matches!(h::decode_receipt(&receipt).map_err(recipient_error)?.status,
                V37Status::Applied|V37Status::Replayed) {return Ok(None);}
            let claim=runtime::observe_claim(&self.connection,&seat::NativeOrigin::user(&self.owner),
                proof.domain_id(),proof.destination_seat_id(),&choice.session_id)
                    .map_err(recipient_error)?.ok_or(OrchestrationError::OperationConflict)?;
            let commit=self.host_recipient_stage(proof,&choice,"commit",
                u64::try_from(claim.revision).map_err(recipient_error)?)?;
            let receipt=self.dispatch_host_recipient_admission(&commit,proof,&choice)?;
            if !matches!(h::decode_receipt(&receipt).map_err(recipient_error)?.status,
                V37Status::Applied|V37Status::Replayed) {return Ok(None);}
            let claim=runtime::observe_claim(&self.connection,&seat::NativeOrigin::user(&self.owner),
                proof.domain_id(),proof.destination_seat_id(),&choice.session_id)
                    .map_err(recipient_error)?.ok_or(OrchestrationError::OperationConflict)?;
            let open=self.host_recipient_stage(proof,&choice,"start",
                u64::try_from(claim.revision).map_err(recipient_error)?)?;
            let receipt=self.dispatch_host_recipient_open(&open,proof,&choice)?;
            if !matches!(h::decode_receipt(&receipt).map_err(recipient_error)?.status,
                V37Status::Applied|V37Status::Replayed) {return Ok(None);}
        }
        Ok(self.native_sessions.contains_key(&key).then_some(key))
    }
}
